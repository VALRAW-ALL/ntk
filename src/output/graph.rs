// ---------------------------------------------------------------------------
// Etapa 19 — ASCII bar charts and sparklines printed to stdout (non-interactive)
//
// Pure string-based rendering — no alternate screen, no event loop.
// Every function prints and returns immediately.
// ---------------------------------------------------------------------------

use crate::metrics::{CompressionRecord, SessionSummary};
use std::collections::HashMap;

// Sparkline characters (ascending density).
const SPARKS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

// Inner width of the box (between the │ border characters).
const W: usize = 70;

// ---------------------------------------------------------------------------
// ANSI helpers
// ---------------------------------------------------------------------------

struct Palette {
    bar: &'static str,
    val: &'static str,
    dim: &'static str,
    l1: &'static str,
    l2: &'static str,
    l3: &'static str,
    rst: &'static str,
}

impl Palette {
    fn new() -> Self {
        if std::env::var("NO_COLOR").is_ok() {
            Self {
                bar: "",
                val: "",
                dim: "",
                l1: "",
                l2: "",
                l3: "",
                rst: "",
            }
        } else {
            Self {
                bar: "\x1b[34m", // blue
                val: "\x1b[33m", // yellow
                dim: "\x1b[2m",  // dim
                l1: "\x1b[32m",  // green
                l2: "\x1b[33m",  // yellow
                l3: "\x1b[34m",  // blue
                rst: "\x1b[0m",
            }
        }
    }
}

/// Visual length counting Unicode characters (not bytes).
fn vis_chars(s: &str) -> usize {
    let mut n = 0usize;
    let mut esc = false;
    for c in s.chars() {
        match c {
            '\x1b' => esc = true,
            _ if esc => {
                if c.is_ascii_alphabetic() {
                    esc = false;
                }
            }
            _ => n = n.saturating_add(1),
        }
    }
    n
}

/// Pad an already-built string (may contain ANSI codes) to `width` visual chars.
fn pad_to(s: &str, width: usize) -> String {
    let v = vis_chars(s);
    if v >= width {
        s.to_owned()
    } else {
        format!("{s}{:pad$}", "", pad = width.saturating_sub(v))
    }
}

// ---------------------------------------------------------------------------
// print_bar_chart  — horizontal bars, top-8 commands by token savings
// ---------------------------------------------------------------------------

/// Print a horizontal bar chart of token savings per command (top-8).
pub fn print_bar_chart(records: &[CompressionRecord]) {
    if records.is_empty() {
        println!("[ntk graph] No compression data yet.");
        return;
    }

    // ── Aggregate by command base name ────────────────────────────────
    let mut savings_map: HashMap<String, u64> = HashMap::new();
    let mut layer_counts = [0u64; 3];
    let mut total_orig: u64 = 0;
    let mut total_comp: u64 = 0;

    for r in records {
        let base = r
            .command
            .split_whitespace()
            .next()
            .unwrap_or("?")
            .to_string();
        let sv = r.original_tokens.saturating_sub(r.compressed_tokens) as u64;
        let entry = savings_map.entry(base).or_insert(0);
        *entry = entry.saturating_add(sv);
        let li = r.layer_used.saturating_sub(1) as usize;
        if li < 3 {
            layer_counts[li] = layer_counts[li].saturating_add(1);
        }
        total_orig = total_orig.saturating_add(r.original_tokens as u64);
        total_comp = total_comp.saturating_add(r.compressed_tokens as u64);
    }

    let total_saved = total_orig.saturating_sub(total_comp);
    let n = records.len() as u64;
    let avg_pct = if total_orig > 0 {
        total_saved as f64 / total_orig as f64 * 100.0
    } else {
        0.0
    };

    // ── Sort + top 8 ─────────────────────────────────────────────────
    let mut entries: Vec<(String, u64)> = savings_map.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.truncate(8);
    if entries.is_empty() {
        println!("[ntk graph] Zero token savings recorded.");
        return;
    }

    // ── Layout ───────────────────────────────────────────────────────
    // Inner line: "  " + cmd(cmd_w) + "  " + bar(bar_w) + "  " + val(5) + " tok" + "  " + pct(3) + "%  "
    // visual:       2  +   cmd_w    +  2  +   bar_w     +  2  +    5    +   4   +   2  +   3    + 1 + 2
    //             = cmd_w + bar_w + 23 = W
    // bar_w = W - cmd_w - 23
    let cmd_w = entries
        .iter()
        .map(|(k, _)| k.len())
        .max()
        .unwrap_or(4)
        .min(14);
    let bar_w = W.saturating_sub(cmd_w.saturating_add(23)).max(8);
    let max_sv = entries[0].1.max(1);

    let p = Palette::new();

    // ── Header ────────────────────────────────────────────────────────
    let title = " NTK · Token Savings ";
    let dashes = W.saturating_sub(title.len().saturating_add(2));
    println!("┌─{title}{:─<dashes$}─┐", "");
    println!("│{:W$}│", "");

    // ── Bars ──────────────────────────────────────────────────────────
    for (cmd, sv) in &entries {
        let filled = (*sv as f64 / max_sv as f64 * bar_w as f64).round() as usize;
        let filled = filled.clamp(1, bar_w);
        let empty = bar_w.saturating_sub(filled);
        let pct = if total_saved > 0 {
            *sv as f64 / total_saved as f64 * 100.0
        } else {
            0.0
        };

        // Build the line — visual widths are fixed, ANSI codes are zero-width.
        let line = format!(
            "  {cmd:<cmd_w$}  {bar}{fill}{rst}{empty}  {val}{sv:>5}{rst} tok  {dim}{pct:>3.0}%{rst}  ",
            cmd    = cmd,
            cmd_w  = cmd_w,
            bar    = p.bar,
            fill   = "█".repeat(filled),
            rst    = p.rst,
            empty  = " ".repeat(empty),
            val    = p.val,
            sv     = sv,
            dim    = p.dim,
            pct    = pct,
        );
        println!("│{}│", pad_to(&line, W));
    }

    // ── Layer distribution ────────────────────────────────────────────
    let total_l: u64 = layer_counts.iter().sum();
    if total_l > 0 {
        println!("│{:W$}│", "");

        // Each segment: "Lx ████ nn%" — bar capped at 12 chars, pct 3 chars
        // 3 segments: "  Layers  " (10) + seg*3 + "   "*2 (6) = 10 + 3*seg + 6
        // seg visual = 2 + 1 + bar + 1 + 3 = 7 + bar  (bar ≤ 12)
        // max total = 10 + 3*(7+12) + 6 = 73 — slightly > W so use bar ≤ 10
        const LBAR: usize = 10;
        let layer_colors = [p.l1, p.l2, p.l3];
        let layer_labels = ["L1", "L2", "L3"];

        let mut segs: Vec<String> = Vec::new();
        for i in 0..3 {
            let frac = layer_counts[i] as f64 / total_l as f64;
            let bl = if layer_counts[i] > 0 {
                (frac * LBAR as f64).round().max(1.0) as usize
            } else {
                0
            };
            let pi = (frac * 100.0).round() as u64;
            segs.push(format!(
                "{c}{lbl} {fill}{rst} {dim}{pi:>3}%{rst}",
                c = layer_colors[i],
                lbl = layer_labels[i],
                fill = "█".repeat(bl),
                rst = p.rst,
                dim = p.dim,
                pi = pi,
            ));
        }

        let layer_line = format!("  Layers  {}   {}   {}  ", segs[0], segs[1], segs[2]);
        println!("│{}│", pad_to(&layer_line, W));
    }

    // ── Footer ────────────────────────────────────────────────────────
    println!("│{:W$}│", "");
    let footer = format!(
        "  {val}{n}{rst} compressions · {val}{total_saved}{rst} tokens saved · {dim}{avg_pct:.0}% avg{rst}  ",
        val=p.val, n=n, rst=p.rst, total_saved=total_saved, dim=p.dim, avg_pct=avg_pct,
    );
    println!("│{}│", pad_to(&footer, W));
    println!("│{:W$}│", "");
    println!("└{:─<W$}┘", "");
}

// ---------------------------------------------------------------------------
// print_sparkline_weekly
// ---------------------------------------------------------------------------

/// Struct carrying last-7-days data for the sparkline (passed in by caller).
pub struct WeeklySummary {
    /// Token savings per day, oldest first (up to 7 entries).
    pub daily_savings: Vec<u64>,
    /// Day labels (e.g. "Mon", "Tue"), same length as daily_savings.
    pub day_labels: Vec<String>,
}

/// Print a sparkline of token savings for the last 7 days.
pub fn print_sparkline_weekly(summary: &WeeklySummary) {
    if summary.daily_savings.is_empty() {
        println!("[ntk graph] No weekly data available.");
        return;
    }

    let max = summary
        .daily_savings
        .iter()
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);

    let spark_last_idx = SPARKS.len().saturating_sub(1);
    let spark: String = summary
        .daily_savings
        .iter()
        .map(|&v| {
            let idx = ((v as f64 / max as f64) * spark_last_idx as f64).round() as usize;
            SPARKS[idx.min(spark_last_idx)]
        })
        .collect();

    let labels = summary.day_labels.join("  ");
    println!("Token savings (7 days):  {spark}");
    println!("                         {labels}");
    println!("Peak: {} tokens saved", max);
}

// ---------------------------------------------------------------------------
// print_layer_distribution  (standalone, used by ntk metrics)
// ---------------------------------------------------------------------------

/// Print a simple ASCII distribution of compressions by layer.
pub fn print_layer_distribution(summary: &SessionSummary) {
    let total = summary.total_compressions;
    if total == 0 {
        println!("No compression data yet.");
        return;
    }

    println!("Layer distribution ({total} compressions):");
    for (i, &count) in summary.layer_counts.iter().enumerate() {
        let pct = (count as f64 / total as f64 * 100.0).round() as usize;
        let bar_len = pct.saturating_div(2);
        let bar = "█".repeat(bar_len);
        let layer_num = i.saturating_add(1);
        println!("  L{layer_num}  {count:>4} ({pct:>3}%)  {bar}");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::OutputType;
    use chrono::Utc;

    fn rec(cmd: &str, orig: usize, comp: usize, layer: u8) -> CompressionRecord {
        CompressionRecord {
            command: cmd.to_owned(),
            output_type: OutputType::Test,
            original_tokens: orig,
            compressed_tokens: comp,
            layer_used: layer,
            latency_ms: 10,
            rtk_pre_filtered: false,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_bar_chart_empty() {
        print_bar_chart(&[]);
    }

    #[test]
    fn test_bar_chart_with_records() {
        let records = vec![
            rec("cargo test", 2000, 300, 3),
            rec("cargo build", 1800, 450, 3),
            rec("tsc", 1000, 250, 2),
            rec("cargo clippy", 500, 150, 1),
        ];
        print_bar_chart(&records);
    }

    #[test]
    fn test_bar_chart_aggregates_same_command() {
        let records = vec![
            rec("cargo test", 1000, 200, 3),
            rec("cargo build", 800, 300, 2),
            rec("cargo test", 600, 100, 3), // should merge with first cargo
        ];
        print_bar_chart(&records);
    }

    #[test]
    fn test_vis_chars_strips_ansi() {
        assert_eq!(vis_chars("\x1b[34mhello\x1b[0m"), 5);
        assert_eq!(vis_chars("hello"), 5);
    }

    #[test]
    fn test_sparkline_weekly_empty() {
        let ws = WeeklySummary {
            daily_savings: vec![],
            day_labels: vec![],
        };
        print_sparkline_weekly(&ws);
    }

    #[test]
    fn test_sparkline_weekly_data() {
        let ws = WeeklySummary {
            daily_savings: vec![0, 500, 1200, 800, 300, 1500, 700],
            day_labels: vec![
                "Mon".into(),
                "Tue".into(),
                "Wed".into(),
                "Thu".into(),
                "Fri".into(),
                "Sat".into(),
                "Sun".into(),
            ],
        };
        print_sparkline_weekly(&ws);
    }

    #[test]
    fn test_layer_distribution_with_data() {
        let summary = SessionSummary {
            total_compressions: 10,
            total_original_tokens: 20000,
            total_compressed_tokens: 15000,
            total_tokens_saved: 5000,
            average_ratio: 0.25,
            layer_counts: [2, 5, 3],
            rtk_pre_filtered_count: 1,
        };
        print_layer_distribution(&summary);
    }
}
