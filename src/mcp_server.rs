// Model Context Protocol (MCP) server — stdio JSON-RPC transport.
//
// Exposes NTK's L1+L2 compression as a tool any MCP-compatible agent
// (Cursor, Zed, Windsurf, Claude Desktop) can call. Runs in-process,
// no daemon dependency: the tool computes L1+L2 synchronously and
// returns the compressed output.
//
// Why not call the daemon? MCP clients often spawn servers per-project
// and expect them to be self-contained. Requiring a separate `ntk start`
// daemon to be running would make the integration brittle. L3 inference
// would need the daemon — deferred to a future `compress_output_l3`
// tool that returns a handle the client polls, or a streaming variant.
//
// Protocol version pinned to 2024-11-05 (the oldest still-supported
// release); most clients negotiate newer versions but accept this one.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

use crate::compressor::{layer1_filter, layer2_tokenizer};

/// Pinned MCP protocol version. Clients may request newer versions in
/// the `initialize` handshake; we respond with this exact string and
/// most clients downgrade transparently.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// A single JSON-RPC 2.0 message. Either a request (has `method` and `id`),
/// a notification (has `method` but no `id`), or a response (has `id`
/// plus `result` or `error`). Fields are intentionally loose so we can
/// echo whatever shape the client sends.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

/// Entry point for `ntk mcp-server`. Reads JSON-RPC messages from stdin
/// one-per-line, writes responses to stdout, logs to stderr. Loops
/// until stdin EOF (the MCP client closed the transport).
pub fn run() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    // Startup signal so `ntk mcp-server` launched interactively shows
    // something on stderr before the first request arrives. Clients that
    // parse stdout only won't see it.
    eprintln!("ntk mcp-server ready (protocol {PROTOCOL_VERSION})");

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("mcp: stdin read error: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("mcp: malformed request ({e}): {line}");
                continue;
            }
        };

        // Notifications have no id — we process them but don't respond.
        let is_notification = req.id.is_none();
        let id_echo = req.id.clone().unwrap_or(serde_json::Value::Null);

        let response = handle_method(&req);

        if is_notification {
            continue;
        }

        let rpc = match response {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0",
                id: id_echo,
                result: Some(result),
                error: None,
            },
            Err((code, message)) => JsonRpcResponse {
                jsonrpc: "2.0",
                id: id_echo,
                result: None,
                error: Some(JsonRpcError { code, message }),
            },
        };

        match serde_json::to_string(&rpc) {
            Ok(s) => {
                if writeln!(out, "{s}").is_err() {
                    eprintln!("mcp: stdout write failed, exiting");
                    break;
                }
                let _ = out.flush();
            }
            Err(e) => eprintln!("mcp: response serialize error: {e}"),
        }
    }

    eprintln!("ntk mcp-server stopped");
    Ok(())
}

/// Route a request method to the corresponding handler. Returns the
/// JSON-RPC `result` payload or a `(code, message)` error tuple. Public
/// so tests can exercise the protocol without stdio.
pub fn handle_method(req: &JsonRpcRequest) -> Result<serde_json::Value, (i32, String)> {
    match req.method.as_str() {
        "initialize" => Ok(initialize_result()),
        "initialized" => Ok(serde_json::Value::Null), // client notification, no response needed
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => tools_call_result(&req.params),
        "ping" => Ok(serde_json::json!({})),
        // Unknown methods: -32601 is JSON-RPC's "method not found" code.
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

fn initialize_result() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "ntk",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn tools_list_result() -> serde_json::Value {
    serde_json::json!({
        "tools": [{
            "name": "compress_output",
            "description": "Compress a command's stdout/stderr via NTK's L1 (regex + template dedup + stack-trace filter) and L2 (BPE tokenizer + path shortening) layers. Returns the compressed text, original and compressed token counts, and the list of applied rules. Does NOT call the neural L3 stage (that requires a running daemon); the synchronous L1+L2 path is enough to drop 40-80% of the tokens from typical test/build/stack-trace output.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "output": {
                        "type": "string",
                        "description": "The raw command output to compress."
                    },
                    "command": {
                        "type": "string",
                        "description": "Optional: the command that produced this output (for context). Not used by the compression itself."
                    }
                },
                "required": ["output"]
            }
        }]
    })
}

fn tools_call_result(params: &serde_json::Value) -> Result<serde_json::Value, (i32, String)> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((-32602, "tools/call: missing params.name".to_owned()))?;

    match name {
        "compress_output" => compress_output_call(params),
        other => Err((-32602, format!("tools/call: unknown tool '{other}'"))),
    }
}

fn compress_output_call(params: &serde_json::Value) -> Result<serde_json::Value, (i32, String)> {
    let args = params
        .get("arguments")
        .ok_or((-32602, "tools/call: missing params.arguments".to_owned()))?;

    let output = args
        .get("output")
        .and_then(|v| v.as_str())
        .ok_or((-32602, "tools/call: missing arguments.output".to_owned()))?;

    // L1 first; L2 consumes L1's output (same contract as the HTTP handler).
    let l1 = layer1_filter::filter(output);
    let l2 = layer2_tokenizer::process(&l1.output)
        .map_err(|e| (-32603, format!("tokenizer error: {e}")))?;

    let ratio_pct = if l2.original_tokens > 0 {
        let saved = l2.original_tokens.saturating_sub(l2.compressed_tokens);
        saved
            .saturating_mul(100)
            .checked_div(l2.original_tokens)
            .unwrap_or(0)
    } else {
        0
    };

    // MCP tool result format: `content` array of typed blocks. `text` is
    // the primary channel; metadata goes next to it for any client that
    // chooses to surface the numbers.
    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": l2.output,
        }],
        "_meta": {
            "tokens_before": l2.original_tokens,
            "tokens_after": l2.compressed_tokens,
            "ratio_pct": ratio_pct,
            "l1_applied_rules": l1.applied_rules,
            "l2_applied_rules": l2.applied_rules,
            "lines_removed": l1.lines_removed,
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req(method: &str, params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            id: Some(serde_json::json!(1)),
            method: method.to_owned(),
            params,
        }
    }

    #[test]
    fn test_initialize_returns_pinned_protocol() {
        let req = make_req("initialize", serde_json::json!({}));
        let result = handle_method(&req).expect("ok");
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], "ntk");
    }

    #[test]
    fn test_tools_list_includes_compress_output() {
        let req = make_req("tools/list", serde_json::json!({}));
        let result = handle_method(&req).expect("ok");
        let tools = result["tools"].as_array().expect("array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "compress_output");
    }

    #[test]
    fn test_unknown_method_returns_method_not_found() {
        let req = make_req("nonesuch", serde_json::json!({}));
        let err = handle_method(&req).unwrap_err();
        assert_eq!(err.0, -32601);
    }

    #[test]
    fn test_compress_output_happy_path() {
        let req = make_req(
            "tools/call",
            serde_json::json!({
                "name": "compress_output",
                "arguments": {
                    "output": "test foo::bar ... ok\ntest foo::baz ... ok\ntest result: FAILED. 1 failed",
                    "command": "cargo test"
                }
            }),
        );
        let result = handle_method(&req).expect("ok");
        let content = &result["content"][0];
        assert_eq!(content["type"], "text");
        assert!(content["text"].as_str().expect("text").len() > 0);
        // The FAILED line must survive (invariant #1).
        assert!(
            content["text"].as_str().expect("text").contains("FAILED"),
            "error signal dropped: {}",
            content["text"]
        );
        // _meta fields present.
        assert!(result["_meta"]["tokens_before"].as_u64().is_some());
        assert!(result["_meta"]["tokens_after"].as_u64().is_some());
    }

    #[test]
    fn test_compress_output_missing_args_returns_invalid_params() {
        let req = make_req(
            "tools/call",
            serde_json::json!({ "name": "compress_output" }),
        );
        let err = handle_method(&req).unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("arguments"));
    }

    #[test]
    fn test_compress_output_unknown_tool_returns_invalid_params() {
        let req = make_req(
            "tools/call",
            serde_json::json!({ "name": "nonesuch", "arguments": {} }),
        );
        let err = handle_method(&req).unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("unknown tool"));
    }

    #[test]
    fn test_initialized_notification_noop() {
        // The client's post-handshake "initialized" notification should
        // not error — we just return null (caller skips sending a
        // response since the original request had no id).
        let req = make_req("initialized", serde_json::json!({}));
        let result = handle_method(&req).expect("ok");
        assert_eq!(result, serde_json::Value::Null);
    }
}
