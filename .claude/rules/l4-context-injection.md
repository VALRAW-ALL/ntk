# Rule: L4 Context Injection

Applies to: any change touching `src/compressor/layer4_context.rs`,
`src/server.rs`'s `handle_compress` (specifically the `context_prefix`
branch), or the PostToolUse hooks in `scripts/ntk-hook.ps1` and
`scripts/ntk-hook.sh`.

## Architectural guarantees

1. **Model-agnostic.** The L4 prefix is plain text (four formats:
   Prefix / XmlWrap / Goal / Json). No format may assume a specific
   model's system-prompt syntax. Every format has to work with Phi-3,
   Claude, Llama, GPT, etc.
2. **Zero coupling to Layer 3 backend.** L4 runs in the server handler,
   prepends a string to `l2.output`, and passes the concatenated string
   to whatever `BackendKind::compress(...)` implementation is active
   (Ollama / Candle / llama.cpp). The backend is oblivious to L4.
3. **Graceful degradation.** Any failure in L4 (transcript missing,
   malformed JSON, empty user messages, I/O error) falls back silently
   to L3-without-context. Never return HTTP 500 because of L4.
4. **Privacy.** The extracted intent is truncated to `MAX_INTENT_CHARS`
   (500) before being injected. Never forward the entire transcript.

## Where the context comes from

Priority order (higher overrides lower):
1. Explicit `request.context` field in `/compress` JSON — used by
   bench/prompt_formats.ps1 and for testing.
2. Transcript parse of `request.transcript_path` — used by the hook.
3. No prefix — when both are absent or `context_aware = false`.

## Prompt format experiments

To A/B a new format:
1. Add a variant to `PromptFormat` enum in `layer4_context.rs`.
2. Map it in the `NTK_L4_FORMAT` match arm in `src/server.rs`.
3. Add the variant to the `$formats` array in `bench/prompt_formats.ps1`.
4. Run the bench on L3-triggering fixtures and compare avg ratio.
5. If the new format beats the current default by > 2 pp AND the
   output passes the information-preservation check (error lines, file
   paths, etc.), update the `#[default]` attribute.

## Cross-platform guarantees

- Transcript path resolution uses `std::path::Path` exclusively. Both
  `scripts/ntk-hook.ps1` and `scripts/ntk-hook.sh` forward the
  `transcript_path` field verbatim from Claude Code's hook payload, so
  Windows `\` vs POSIX `/` is preserved end-to-end.
- JSONL parsing uses `serde_json` in streaming line-by-line mode —
  never load the whole transcript into memory. Transcripts grow
  unbounded during long sessions.

## Config migration

When bumping the default of `compression.context_aware`:
- Any existing `~/.ntk/config.json` keeps the explicit user value.
- New installs inherit the new default via `serde(default)`.
- Document the change in the commit message AND the README settings table.
