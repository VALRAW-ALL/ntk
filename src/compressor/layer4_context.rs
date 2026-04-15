// Layer 4 — Context Injection
//
// Reads the Claude Code session transcript (.jsonl) and extracts the user's
// current intent so Layer 3 can produce a summary *relevant to what the user
// is actually trying to do*, not a generic "summarize this output".
//
// Transcript format (one JSON object per line):
//   { "type": "user",      "message": { "content": [{ "type": "text", "text": "..." }] } }
//   { "type": "assistant", "message": { "content": [...], "usage": {...} } }
//   { "type": "system",    ... }
//   { "type": "tool_use",  ... }
//
// We only care about the most recent `user` event — that's where the user
// expressed intent. Filtering rules:
//   - Ignore messages with isMeta=true (slash-command metadata).
//   - Ignore messages that start with "<command-name>" (local commands).
//   - Take only the text content; strip tool-use blocks.
//   - Truncate to 500 chars to keep the context prefix tight.
//
// The extracted context is model-agnostic prose (plain string). Formatting
// into a specific prompt template happens in the backend layer (layer3_*).

use serde::Deserialize;
use std::path::Path;

/// Extracted intent signal + optional metadata. `None` when the transcript
/// can't be read, doesn't exist, or contains no usable user message.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub user_intent: String,
    /// How many turns ago the intent was (0 = most recent).
    pub turns_ago: usize,
}

/// Minimal deserialization target — only the fields we actually read.
/// Unknown fields are ignored via `#[serde(default)]` on each alias so
/// Claude Code transcript schema changes don't break us.
#[derive(Debug, Deserialize)]
struct TranscriptEvent {
    #[serde(rename = "type", default)]
    event_type: String,
    #[serde(default)]
    #[serde(rename = "isMeta")]
    is_meta: bool,
    #[serde(default)]
    message: Option<UserMessage>,
    #[serde(default)]
    content: Option<String>, // Some user events store "content" at top level.
}

#[derive(Debug, Deserialize)]
struct UserMessage {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: serde_json::Value,
}

const MAX_INTENT_CHARS: usize = 500;

/// Read the transcript at `path`, return the most recent user intent.
/// Returns `None` when the file is missing, empty, or no usable message exists.
pub fn extract_context(path: &Path) -> Option<SessionContext> {
    let bytes = std::fs::read_to_string(path).ok()?;
    extract_from_jsonl(&bytes)
}

/// Parser split out for testability (no filesystem).
pub fn extract_from_jsonl(jsonl: &str) -> Option<SessionContext> {
    // Walk the transcript in reverse — the most recent user message wins.
    // We use a small ring of assistant turns between the user message and
    // the compressed tool output to compute `turns_ago`.
    let lines: Vec<&str> = jsonl.lines().collect();
    let mut assistant_turns_after = 0usize;

    for line in lines.iter().rev() {
        if line.is_empty() {
            continue;
        }
        let event: TranscriptEvent = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if event.is_meta {
            continue;
        }

        match event.event_type.as_str() {
            "assistant" => {
                assistant_turns_after = assistant_turns_after.saturating_add(1);
            }
            "user" => {
                if let Some(text) = extract_user_text(&event) {
                    if let Some(cleaned) = clean_intent(&text) {
                        return Some(SessionContext {
                            user_intent: cleaned,
                            turns_ago: assistant_turns_after,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_user_text(event: &TranscriptEvent) -> Option<String> {
    // Prefer message.content when present.
    if let Some(msg) = &event.message {
        if msg.role != "user" && !msg.role.is_empty() {
            return None;
        }
        return flatten_content(&msg.content);
    }
    // Fallback: some transcripts store plain-text content at the top level.
    event.content.clone()
}

/// Claude Code's message.content is either a String or an array of content
/// blocks ({"type":"text","text":...}, {"type":"tool_use",...}, etc).
/// We concatenate text blocks and ignore tool_use / tool_result blocks.
fn flatten_content(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_owned());
    }
    let arr = value.as_array()?;
    let mut parts: Vec<String> = Vec::new();
    for block in arr {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if block_type == "text" {
            if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                parts.push(t.to_owned());
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// Strip slash-command metadata and trim. Returns None for empty / metadata-
/// only messages.
fn clean_intent(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Drop slash-command wrappers: "<command-name>...</command-name>" blocks
    // and "<local-command-stdout>...</local-command-stdout>" blocks don't
    // represent user intent — they're command invocations.
    let cleaned = strip_xml_blocks(trimmed, "command-name");
    let cleaned = strip_xml_blocks(&cleaned, "command-message");
    let cleaned = strip_xml_blocks(&cleaned, "command-args");
    let cleaned = strip_xml_blocks(&cleaned, "local-command-stdout");
    let cleaned = strip_xml_blocks(&cleaned, "local-command-stderr");

    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return None;
    }

    // Truncate to keep the context prefix tight — L3 sees this, not the
    // full transcript.
    let truncated = if cleaned.chars().count() > MAX_INTENT_CHARS {
        let mut s: String = cleaned.chars().take(MAX_INTENT_CHARS).collect();
        s.push_str("...");
        s
    } else {
        cleaned.to_owned()
    };

    Some(truncated)
}

fn strip_xml_blocks(input: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while let Some(start) = input[cursor..].find(&open) {
        let abs_start = cursor.saturating_add(start);
        out.push_str(&input[cursor..abs_start]);
        if let Some(end_rel) = input[abs_start..].find(&close) {
            cursor = abs_start
                .saturating_add(end_rel)
                .saturating_add(close.len());
        } else {
            // No closing tag — dump the rest.
            out.push_str(&input[abs_start..]);
            return out;
        }
    }
    out.push_str(&input[cursor..]);
    out
}

// ---------------------------------------------------------------------------
// Format the context into a prompt prefix.
//
// Four formats are supported; experiments in bench/ will pick the best.
// All are <200 chars to minimize the token tax.
// ---------------------------------------------------------------------------

/// Default: Prefix narrowly beat Goal on cl100k_base avg ratio (68% vs 66%)
/// with 4 fewer tokens of overhead when the prompt-format A/B ran over the
/// 8 fixture library in Apr 2026.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptFormat {
    /// "CONTEXT: User asked: ... — focus on relevance."
    #[default]
    Prefix,
    /// "<intent>...</intent>"
    XmlWrap,
    /// "User goal: ... — extract only info that advances this goal."
    Goal,
    /// JSON wrapper — more rigid but easier for some models to parse.
    Json,
}

pub fn format_context(ctx: &SessionContext, fmt: PromptFormat) -> String {
    let intent = ctx.user_intent.as_str();
    match fmt {
        PromptFormat::Prefix => format!(
            "CONTEXT: The user's most recent request was: \"{}\". Summarize the tool output focusing on information relevant to that request.\n\n",
            intent
        ),
        PromptFormat::XmlWrap => format!(
            "<user_intent>{}</user_intent>\n\n",
            intent
        ),
        PromptFormat::Goal => format!(
            "User goal: {} — extract only information that advances this goal.\n\n",
            intent
        ),
        PromptFormat::Json => {
            let escaped = intent.replace('"', "\\\"").replace('\n', "\\n");
            format!("{{\"user_intent\": \"{escaped}\"}}\n\n")
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"type":"system","content":"startup"}
{"parentUuid":null,"type":"user","message":{"role":"user","content":[{"type":"text","text":"please help me debug this build error"}]},"isMeta":false,"uuid":"abc","sessionId":"s1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I will run cargo build"}],"usage":{"input_tokens":100,"output_tokens":50}},"uuid":"def"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"build failed"}]},"uuid":"ghi"}
"#;

    #[test]
    fn extracts_most_recent_user_text() {
        let ctx = extract_from_jsonl(SAMPLE).expect("should find user intent");
        assert_eq!(ctx.user_intent, "please help me debug this build error");
        // 1 assistant turn between that user and the end of transcript
        assert_eq!(ctx.turns_ago, 1);
    }

    #[test]
    fn ignores_is_meta_events() {
        let jsonl = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"slash command"}]}}
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"real user message"}]}}
"#;
        let ctx = extract_from_jsonl(jsonl).unwrap();
        assert_eq!(ctx.user_intent, "real user message");
    }

    #[test]
    fn strips_slash_command_wrappers() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<command-name>/help</command-name>\n<command-message>help</command-message>\nhow do I invoke the hook"}]}}
"#;
        let ctx = extract_from_jsonl(jsonl).unwrap();
        assert!(!ctx.user_intent.contains("<command-name>"));
        assert!(ctx.user_intent.contains("how do I invoke the hook"));
    }

    #[test]
    fn returns_none_for_empty_transcript() {
        assert!(extract_from_jsonl("").is_none());
        assert!(extract_from_jsonl("\n\n").is_none());
    }

    #[test]
    fn returns_none_for_malformed_json() {
        assert!(extract_from_jsonl("{not json\n").is_none());
    }

    #[test]
    fn truncates_long_messages() {
        let big = "x".repeat(2000);
        let jsonl = format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"{big}"}}]}}}}
"#
        );
        let ctx = extract_from_jsonl(&jsonl).unwrap();
        assert!(ctx.user_intent.ends_with("..."));
        assert!(ctx.user_intent.len() <= MAX_INTENT_CHARS + 10);
    }

    #[test]
    fn flattens_mixed_content_blocks() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"first"},{"type":"tool_use","name":"Bash"},{"type":"text","text":"second"}]}}
"#;
        let ctx = extract_from_jsonl(jsonl).unwrap();
        assert!(ctx.user_intent.contains("first"));
        assert!(ctx.user_intent.contains("second"));
    }

    #[test]
    fn accepts_string_content() {
        // Some older transcripts store content as a plain string.
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"legacy format"}}
"#;
        let ctx = extract_from_jsonl(jsonl).unwrap();
        assert_eq!(ctx.user_intent, "legacy format");
    }

    #[test]
    fn format_prefix_is_model_agnostic() {
        let ctx = SessionContext {
            user_intent: "run cargo test".to_owned(),
            turns_ago: 0,
        };
        let formatted = format_context(&ctx, PromptFormat::Prefix);
        assert!(formatted.starts_with("CONTEXT:"));
        assert!(formatted.contains("run cargo test"));
    }

    #[test]
    fn format_json_escapes_quotes() {
        let ctx = SessionContext {
            user_intent: r#"find "foo" error"#.to_owned(),
            turns_ago: 0,
        };
        let formatted = format_context(&ctx, PromptFormat::Json);
        assert!(formatted.contains(r#"\"foo\""#));
        // Must be valid JSON-like
        assert!(formatted.starts_with('{'));
    }
}
