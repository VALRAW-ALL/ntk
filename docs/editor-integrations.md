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

## Continue (VS Code extension)

**Hook model:** tool plugin API via `~/.continue/config.json`.
Continue calls tools after the model invokes them; the extension
can wrap tool outputs in custom post-processors, but the API is
JavaScript.

**Workable path — Continue plugin:**
- Package a tiny JS plugin that calls the NTK daemon before
  returning tool output to the model
- The plugin posts to `http://127.0.0.1:8765/compress` with the
  same payload the Claude Code hook uses
- Distributes as `ntk-continue` npm package or inline snippet

**Example plugin call (pseudo, to be written):**

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

Tracked as follow-up issue.

---

## Aider

**Hook model:** `--lint-cmd` / `--test-cmd` flags in `.aider.conf.yml`,
but these are *pre*-command validators, not *post*-output filters.
Aider does not expose a post-tool-response hook.

**Workable path — wrapper CLI:**
- Ship `ntk-aider` as a shell wrapper that spawns `aider` with its
  stdout piped through a small NTK filter process
- Compression happens at the terminal-output layer, not the model
  context — less precise but works today

**Alternative — patch Aider upstream:**
- Aider accepts PRs that add a `--post-tool-cmd` hook. A well-scoped
  PR there would make native NTK integration possible.

Tracked as follow-up issue.

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
