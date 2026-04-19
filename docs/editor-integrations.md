# Editor integration guide

NTK's PostToolUse hook spec follows Claude Code's:

```json
{
  "session_id": "...", "transcript_path": "...", "cwd": "...",
  "tool_name": "Bash",
  "tool_input":    { "command": "...", "description": "..." },
  "tool_response": { "output": "...", "exit_code": 0 }
}
```

The hook reads that from stdin, POSTs to the daemon's `/compress`, and
emits Claude Code's `hookSpecificOutput.additionalContext` JSON back to
stdout. Any editor that implements that exact protocol gets NTK "for
free" via `ntk init -g`.

**Native support** (today):

| Editor | Config file | Integration | Status |
|---|---|---|---|
| Claude Code | `~/.claude/settings.json` | PostToolUse hook | ✅ `ntk init -g` |
| OpenCode | `~/.opencode/settings.json` | PostToolUse hook | ✅ `ntk init -g --opencode` |
| Cursor | `~/.cursor/mcp.json` | MCP server (`ntk mcp-server`) | ✅ `ntk init -g --cursor` |
| Zed | `<config>/zed/settings.json` | MCP via `context_servers` | ✅ `ntk init -g --zed` |
| Continue | `~/.continue/config.json` | MCP via `mcpServers[]` | ✅ `ntk init -g --continue` |

Everything below needs an adapter because the editor uses a different
integration model. Each section documents the concrete shape; a
contributor picks one, reads the editor's docs, and opens a PR adding
a new `EditorTarget` variant.

---

## Cursor ✅ shipped

```bash
ntk init -g --cursor
```

Registers NTK as an MCP server in `~/.cursor/mcp.json`. The agent gets
a `compress_output` tool it can call whenever a long command result
would otherwise blow up the context window.

**How it works:**
- `ntk mcp-server` is a stdio JSON-RPC server (protocol `2024-11-05`)
- Synchronous — runs L1+L2 in-process, no `ntk start` daemon needed
- Protocol: `initialize` → `tools/list` → `tools/call compress_output`
- Tool returns `{ content: [{type:"text", text:<compressed>}], _meta: { tokens_before, tokens_after, ratio_pct, applied_rules } }`

**Installed config (auto-written):**

```json
{
  "mcpServers": {
    "ntk": {
      "command": "/path/to/.ntk/bin/ntk",
      "args": ["mcp-server"],
      "_ntk": "ntk-hook"
    }
  }
}
```

**L3 note:** the MCP tool runs L1+L2 only. L3 (neural inference) would
require a running daemon — deferred to a future `compress_output_l3`
tool that returns a poll handle, or a streaming variant.

---

## Continue ✅ shipped

```bash
ntk init -g --continue
```

Registers NTK as an MCP server in `~/.continue/config.json`. Continue
added MCP support in 2025; the agent can call `compress_output` the
same way Cursor and Zed do.

**Installed config (auto-written):**

```json
{
  "mcpServers": [
    {
      "name": "ntk",
      "command": "/abs/path/.ntk/bin/ntk",
      "args": ["mcp-server"],
      "_ntk": "ntk-hook"
    }
  ]
}
```

Note Continue uses an **array** of server objects (each with an inline
`name`) rather than Cursor's object-keyed-by-name — hence a separate
inject function under the hood.

The original plugin-based path (`~/.continue/plugins/ntk.js` calling
the daemon over HTTP) is still documented below as an alternative for
users on older Continue versions without MCP support.

<details>
<summary>Legacy plugin path (Continue &lt; 2025 MCP release)</summary>

```javascript
// ~/.continue/plugins/ntk.js
export async function postProcess(toolName, output) {
  if (toolName !== 'Bash') return output;
  const token = fs.readFileSync(path.join(os.homedir(), '.ntk/.token'), 'utf8').trim();
  const r = await fetch('http://127.0.0.1:8765/compress', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-NTK-Token': token },
    body: JSON.stringify({ output, command: '', cwd: process.cwd() }),
  });
  const body = await r.json();
  return body.compressed;
}
```

</details>

---

## Aider ⚠️ blocked on upstream

Aider has **no viable live-compression hook** today. The runtime
flags it exposes (`--lint-cmd`, `--test-cmd`) are *pre*-command
validators, not *post*-output filters — they run before a command
executes and can block it, but they can't rewrite the output the
model sees.

### Why a wrapper script is not a real solution

Piping `aider`'s stdout through a filter would break its interactive
TUI (user input prompts, streaming generation, multi-line edits).
Aider is a chat-like loop in the terminal, not a unidirectional
stream; intercepting stdout corrupts the interface.

### Post-session review (best-effort, partial value)

Aider writes a chat transcript to `.aider.chat.history.md`. After a
session ends you can feed that file through NTK manually for review:

```bash
ntk test-compress .aider.chat.history.md --verbose
```

This gives you the same compression statistics you'd see for a
live hook, but **only retrospectively** — the actual LLM calls
during the session used the full uncompressed context and paid full
token cost.

### The real path forward

A well-scoped PR to [paul-gauthier/aider](https://github.com/paul-gauthier/aider)
adding a `--post-tool-cmd <cmd>` flag (mirroring Claude Code's
PostToolUse hook contract) would make native NTK integration a
one-line config change. Opening that PR requires alignment with the
Aider maintainer on the hook shape — hence an upstream coordination
task rather than a local NTK code change.

Tracked as a **blocked** follow-up until upstream accepts the hook.

---

## Zed ✅ shipped

```bash
ntk init -g --zed
```

Registers NTK as a context server in Zed's `settings.json` (resolved
via `dirs::config_dir()`: `~/.config/zed/settings.json` on Linux/macOS,
`%APPDATA%\Zed\settings.json` on Windows).

**Installed config (auto-written):**

```json
{
  "context_servers": {
    "ntk": {
      "command": {
        "path": "/path/to/.ntk/bin/ntk",
        "args": ["mcp-server"],
        "env": {}
      },
      "_ntk": "ntk-hook"
    }
  }
}
```

Reuses the exact same `ntk mcp-server` binary as Cursor (shipped in
#27). Zed's agent calls `compress_output` the same way — only the
JSON shape around it differs (`context_servers` + nested `command`
object vs Cursor's flat `mcpServers`).

---

## Adding your editor

Opening a PR for a new editor should touch:

1. `src/installer.rs` — new `EditorTarget` variant + `editor_settings_path` arm
2. `src/main.rs` — new CLI flag routing to the variant
3. `tests/integration/cli_tests.rs` — `test_ntk_install_creates_hook` variant
4. This file — add a section above the "Adding your editor" heading

Before opening the PR, **verify the editor actually has a hook
point** that can carry the Claude Code PostToolUse JSON. If it
doesn't, the right path is an adapter (MCP / wrapper / plugin) —
open a discussion issue first so we agree on the integration shape.
