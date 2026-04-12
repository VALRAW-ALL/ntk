// ---------------------------------------------------------------------------
// Etapa 19 — Formatted tables printed to stdout (non-interactive)
//
// All functions print to stdout and return immediately — no event loop,
// no alternate screen, no keyboard capture.
// ---------------------------------------------------------------------------

use crate::metrics::{CompressionRecord, SessionSummary};

// ---------------------------------------------------------------------------
// print_metrics_table
// ---------------------------------------------------------------------------

/// Print a per-command metrics table to stdout.
pub fn print_metrics_table(records: &[CompressionRecord]) {
    if records.is_empty() {
        println!("No compression records yet.");
        return;
    }

    // Column widths (minimum widths — expand to content).
    let cmd_w = records
        .iter()
        .map(|r| r.command.len().min(30))
        .max()
        .unwrap_or(7)
        .max(7);

    println!(
        "{:<cmd_w$}  {:<8}  {:>8}  {:>8}  {:>6}  {:>5}  RTK",
        "COMMAND",
        "TYPE",
        "BEFORE",
        "AFTER",
        "RATIO",
        "LAYER",
        cmd_w = cmd_w,
    );
    let sep_len = cmd_w
        .saturating_add(8)
        .saturating_add(8)
        .saturating_add(8)
        .saturating_add(6)
        .saturating_add(5)
        .saturating_add(5)
        .saturating_add(14);
    println!("{}", "-".repeat(sep_len));

    for r in records {
        let cmd = if r.command.len() > 30 {
            &r.command[..30]
        } else {
            &r.command
        };
        let ratio_pct = r.ratio() * 100.0;
        let rtk = if r.rtk_pre_filtered { "yes" } else { "no" };
        println!(
            "{:<cmd_w$}  {:<8}  {:>8}  {:>8}  {:>5.0}%  L{:<4}  {}",
            cmd,
            format!("{:?}", r.output_type).to_lowercase(),
            r.original_tokens,
            r.compressed_tokens,
            ratio_pct,
            r.layer_used,
            rtk,
            cmd_w = cmd_w,
        );
    }
}

// ---------------------------------------------------------------------------
// print_session_summary
// ---------------------------------------------------------------------------

/// Print an aggregate session summary to stdout.
pub fn print_session_summary(summary: &SessionSummary) {
    println!("Session Summary");
    println!("  Total compressions : {}", summary.total_compressions);
    println!("  Tokens saved       : {}", summary.total_tokens_saved);
    println!(
        "  Average ratio      : {:.1}%",
        summary.average_ratio * 100.0
    );
    println!(
        "  Layer distribution : L1={} L2={} L3={}",
        summary.layer_counts[0], summary.layer_counts[1], summary.layer_counts[2]
    );
    println!("  RTK pre-filtered   : {}", summary.rtk_pre_filtered_count);
}

// ---------------------------------------------------------------------------
// print_gain_rtk_compat
// ---------------------------------------------------------------------------

/// Print a one-line gain summary in RTK-compatible format.
///
/// Format matches `rtk gain` output so scripts can consume either tool.
pub fn print_gain_rtk_compat(summary: &SessionSummary) {
    let pct = summary.average_ratio * 100.0;
    println!(
        "NTK: {} tokens saved across {} compressions ({:.0}% avg)",
        summary.total_tokens_saved, summary.total_compressions, pct,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::OutputType;
    use chrono::Utc;

    fn make_record(original: usize, compressed: usize, layer: u8, rtk: bool) -> CompressionRecord {
        CompressionRecord {
            command: "cargo test".to_owned(),
            output_type: OutputType::Test,
            original_tokens: original,
            compressed_tokens: compressed,
            layer_used: layer,
            latency_ms: 10,
            rtk_pre_filtered: rtk,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_print_metrics_table_empty() {
        // Should not panic.
        print_metrics_table(&[]);
    }

    #[test]
    fn test_print_metrics_table_with_records() {
        let records = vec![
            make_record(1000, 200, 2, false),
            make_record(500, 100, 3, true),
        ];
        // Should not panic and should produce output.
        print_metrics_table(&records);
    }

    #[test]
    fn test_print_session_summary() {
        let summary = SessionSummary {
            total_compressions: 5,
            total_original_tokens: 8000,
            total_compressed_tokens: 6000,
            total_tokens_saved: 2000,
            average_ratio: 0.75,
            layer_counts: [1, 3, 1],
            rtk_pre_filtered_count: 2,
        };
        print_session_summary(&summary);
    }

    #[test]
    fn test_print_gain_rtk_compat() {
        let summary = SessionSummary {
            total_compressions: 10,
            total_original_tokens: 27000,
            total_compressed_tokens: 22000,
            total_tokens_saved: 5000,
            average_ratio: 0.82,
            layer_counts: [2, 5, 3],
            rtk_pre_filtered_count: 0,
        };
        // Should not panic.
        print_gain_rtk_compat(&summary);
    }
}
