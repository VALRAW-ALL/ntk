// ---------------------------------------------------------------------------
// Etapa 19 — ASCII bar charts and sparklines printed to stdout (non-interactive)
//
// Uses ratatui to render widgets into an in-memory buffer, then prints the
// buffer as plain text.  No CrosstermBackend, no alternate screen, no event
// loop — every function prints and returns immediately.
// ---------------------------------------------------------------------------

use crate::metrics::{CompressionRecord, SessionSummary};
use ratatui::{
    backend::TestBackend,
    layout::Rect,
    style::{Color, Style},
    widgets::{Bar, BarChart, BarGroup, Block, Borders},
    Terminal,
};

// Sparkline characters (ascending density).
const SPARKS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

// ---------------------------------------------------------------------------
// print_bar_chart
// ---------------------------------------------------------------------------

/// Print an ASCII bar chart of token savings per command (top-10 by savings).
pub fn print_bar_chart(records: &[CompressionRecord]) {
    if records.is_empty() {
        println!("[ntk graph] No compression data yet.");
        return;
    }

    // Aggregate savings by command (truncated to 20 chars for display).
    let mut by_cmd: Vec<(&str, u64)> = records
        .iter()
        .map(|r| {
            let cmd = if r.command.len() > 20 {
                &r.command[..20]
            } else {
                &r.command
            };
            let saved = r.original_tokens.saturating_sub(r.compressed_tokens) as u64;
            (cmd, saved)
        })
        .collect();

    // Sort by savings descending, keep top 10.
    by_cmd.sort_by(|a, b| b.1.cmp(&a.1));
    by_cmd.dedup_by(|a, b| {
        if a.0 == b.0 {
            b.1 = b.1.saturating_add(a.1);
            true
        } else {
            false
        }
    });
    by_cmd.truncate(10);

    if by_cmd.is_empty() {
        println!("[ntk graph] All outputs had zero token savings.");
        return;
    }

    let width: u16 = 72;
    let height: u16 = (by_cmd.len() as u16).saturating_mul(2).saturating_add(4); // 2 rows per bar + borders

    let bars: Vec<Bar> = by_cmd
        .iter()
        .map(|(cmd, saved)| {
            Bar::default()
                .label((*cmd).into())
                .value(*saved)
                .style(Style::default().fg(Color::Green))
        })
        .collect();

    let bar_group = BarGroup::default().bars(&bars);

    let chart = BarChart::default()
        .block(
            Block::default()
                .title("NTK Token Savings by Command")
                .borders(Borders::ALL),
        )
        .data(bar_group)
        .bar_width(3)
        .bar_gap(1)
        .value_style(Style::default().fg(Color::Yellow));

    let backend = TestBackend::new(width, height);
    let Ok(mut terminal) = Terminal::new(backend) else {
        println!("[ntk graph] Could not initialise render buffer.");
        return;
    };

    terminal
        .draw(|frame| {
            let area = Rect::new(0, 0, width, height);
            frame.render_widget(chart, area);
        })
        .ok();

    let buffer = terminal.backend().buffer().clone();
    print_buffer(&buffer, width, height);
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

    let max = summary.daily_savings.iter().copied().max().unwrap_or(1).max(1);

    let spark_last_idx = SPARKS.len().saturating_sub(1);
    let spark: String = summary
        .daily_savings
        .iter()
        .map(|&v| {
            let idx =
                ((v as f64 / max as f64) * spark_last_idx as f64).round() as usize;
            SPARKS[idx.min(spark_last_idx)]
        })
        .collect();

    let labels = summary.day_labels.join("  ");
    println!("Token savings (7 days):  {spark}");
    println!("                         {labels}");
    println!("Peak: {} tokens saved", max);
}

// ---------------------------------------------------------------------------
// print_layer_distribution
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
        let bar_len = pct.saturating_div(2); // max 50 chars for 100%
        let bar = "█".repeat(bar_len);
        let layer_num = i.saturating_add(1);
        println!("  L{layer_num}  {count:>4} ({pct:>3}%)  {bar}");
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn print_buffer(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) {
    for row in 0..height {
        let mut line = String::with_capacity(width as usize);
        for col in 0..width {
            let cell = buffer.cell((col, row)).map(|c| c.symbol()).unwrap_or(" ");
            line.push_str(cell);
        }
        println!("{}", line.trim_end());
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

    fn make_record(cmd: &str, original: usize, compressed: usize) -> CompressionRecord {
        CompressionRecord {
            command: cmd.to_owned(),
            output_type: OutputType::Test,
            original_tokens: original,
            compressed_tokens: compressed,
            layer_used: 2,
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
            make_record("cargo test", 2000, 300),
            make_record("tsc", 1000, 250),
            make_record("cargo build", 500, 150),
        ];
        print_bar_chart(&records);
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
    fn test_layer_distribution_empty() {
        let summary = SessionSummary {
            total_compressions: 0,
            total_tokens_saved: 0,
            average_ratio: 0.0,
            layer_counts: [0; 3],
            rtk_pre_filtered_count: 0,
        };
        print_layer_distribution(&summary);
    }

    #[test]
    fn test_layer_distribution_with_data() {
        let summary = SessionSummary {
            total_compressions: 10,
            total_tokens_saved: 5000,
            average_ratio: 0.75,
            layer_counts: [2, 5, 3],
            rtk_pre_filtered_count: 1,
        };
        print_layer_distribution(&summary);
    }
}
