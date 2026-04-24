//! Regression guards for the PostToolUse hook scripts.
//!
//! Historical bug (v0.2.30): both `ntk-hook.ps1` and `ntk-hook.sh` read the
//! Bash tool output from `tool_response.output`, but Claude Code actually
//! emits it under `tool_response.stdout`. The field mismatch made the hook
//! exit silently on every real invocation — no metrics, no compression, no
//! telemetry. These tests fail if a future edit drops the `stdout` path.
//!
//! The hook scripts are embedded into the binary via `include_str!` in
//! `src/installer.rs`, so validating the source files is equivalent to
//! validating what ships.

const PS1: &str = include_str!("../../scripts/ntk-hook.ps1");
const SH: &str = include_str!("../../scripts/ntk-hook.sh");

#[test]
fn ps1_reads_stdout_field() {
    assert!(
        PS1.contains("tool_response.stdout"),
        "ntk-hook.ps1 must read tool_response.stdout (Claude Code's real field for Bash output). \
         Regression guard for the v0.2.30 silent-hook bug."
    );
}

#[test]
fn sh_reads_stdout_field() {
    assert!(
        SH.contains("stdout"),
        "ntk-hook.sh must reference the stdout field when extracting tool_response output. \
         Regression guard for the v0.2.30 silent-hook bug."
    );
}

#[test]
fn ps1_keeps_output_fallback_for_backward_compat() {
    assert!(
        PS1.contains("tool_response.output"),
        "ntk-hook.ps1 should still recognize tool_response.output as a fallback \
         to remain compatible with older Claude Code hook payload shapes."
    );
}

#[test]
fn sh_keeps_output_fallback_for_backward_compat() {
    assert!(
        SH.contains("'output'"),
        "ntk-hook.sh should still recognize 'output' as a fallback key in the \
         tool_response extraction logic."
    );
}

#[test]
fn ps1_still_gates_on_bash_tool_name() {
    assert!(
        PS1.contains("\"Bash\""),
        "ntk-hook.ps1 must only process tool_name == \"Bash\" — other tools \
         (Edit, Read, Grep, etc.) produce output shapes we don't compress."
    );
}

#[test]
fn sh_still_gates_on_bash_tool_name() {
    assert!(
        SH.contains("\"Bash\""),
        "ntk-hook.sh must only process tool_name == \"Bash\"."
    );
}

#[test]
fn ps1_has_min_chars_threshold() {
    assert!(
        PS1.contains("$MinChars"),
        "ntk-hook.ps1 must keep the MinChars threshold — compressing tiny outputs \
         wastes a daemon round-trip that costs more than the tokens saved."
    );
}

#[test]
fn sh_has_min_chars_threshold() {
    assert!(
        SH.contains("MIN_CHARS"),
        "ntk-hook.sh must keep the MIN_CHARS threshold."
    );
}
