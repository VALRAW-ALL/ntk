// ---------------------------------------------------------------------------
// Live TUI dashboard for `ntk start`
//
// Renders a three-panel view in the alternate screen:
//   • Header  — ASCII-art "NTK" logo + version / addr / uptime / backend
//   • Metrics — session stats + per-layer bar chart
//   • Logs    — last 3 commands with token in→out and ratio
//
// Falls back to a single println when stdout is not a TTY.
// Restores the terminal unconditionally on exit (even if rendering panics).
// ---------------------------------------------------------------------------

use std::collections::VecDeque;
use std::io::Stdout;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{poll as event_poll, read as event_read, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    tty::IsTty,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use tokio::sync::watch;

use crate::metrics::{CompressionRecord, MetricsStore, SessionSummary};
use serde::{Deserialize, Serialize};

// reqwest is already a dependency (used in main.rs via ureq_get).
// Re-use it here for the attach-mode polling loop.

// ---------------------------------------------------------------------------
// Warn log — captures WARN/ERROR tracing events for the dashboard panel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WarnLevel {
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarnEntry {
    pub level: WarnLevel,
    pub message: String,
    pub timestamp: chrono::DateTime<chrono::Local>,
}

/// Shared buffer passed to both the tracing layer and the dashboard renderer.
pub type WarnBuffer = Arc<Mutex<VecDeque<WarnEntry>>>;

/// Tracing subscriber layer that captures WARN and ERROR events into a `WarnBuffer`.
pub struct WarnCaptureLayer {
    buffer: WarnBuffer,
}

impl WarnCaptureLayer {
    pub fn new(buffer: WarnBuffer) -> Self {
        Self { buffer }
    }
}

struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // fmt::Arguments Debug == Display, so no surrounding quotes.
            self.message = format!("{value:?}");
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for WarnCaptureLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = *event.metadata().level();
        if level > tracing::Level::WARN {
            return;
        }
        let mut visitor = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut visitor);
        if visitor.message.is_empty() {
            return;
        }
        let entry = WarnEntry {
            level: if level == tracing::Level::ERROR {
                WarnLevel::Error
            } else {
                WarnLevel::Warn
            },
            message: visitor.message,
            timestamp: chrono::Local::now(),
        };
        if let Ok(mut buf) = self.buffer.lock() {
            if buf.len() >= 50 {
                buf.pop_front();
            }
            buf.push_back(entry);
        }
    }
}

// ---------------------------------------------------------------------------
// ASCII art "NTK" (6 rows × 26 cols).  Each entry: (text, color_index).
// ---------------------------------------------------------------------------

const NTK_ART: [(&str, u8); 6] = [
    ("██╗  ██╗████████╗██╗  ██╗", 0),
    ("████╗ ██║╚══██╔══╝██║ ██╔╝", 1),
    ("██╔██╗██║   ██║   █████╔╝ ", 2),
    ("██║╚████║   ██║   ██╔═██╗ ", 1),
    ("██║ ╚███║   ██║   ██║  ██╗", 0),
    ("╚═╝  ╚══╝   ╚═╝   ╚═╝  ╚═╝", 3),
];

fn art_color(idx: u8) -> Color {
    match idx {
        0 => Color::Rgb(15, 80, 25),  // very dark green
        1 => Color::Rgb(27, 107, 44), // dark green
        2 => Color::Rgb(36, 126, 56), // medium dark green
        _ => Color::Rgb(8, 45, 14),   // near-black green shadow
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Launch the live dashboard and block until `shutdown` fires.
///
/// If stdout is not a TTY (e.g. piped / CI), falls back to a plain one-liner
/// and waits for the shutdown signal without touching the terminal.
pub async fn run_live_dashboard(
    metrics: Arc<Mutex<MetricsStore>>,
    warn_log: WarnBuffer,
    started_at: Instant,
    addr: String,
    backend_name: String,
    shutdown: watch::Receiver<bool>,
    // Sender so the dashboard can trigger server shutdown when Ctrl+C is
    // pressed as a key event (raw mode suppresses the normal SIGINT path).
    shutdown_trigger: watch::Sender<bool>,
) -> Result<()> {
    if !std::io::stdout().is_tty() {
        println!("NTK daemon started on {addr}  (Ctrl-C to stop)");
        let mut rx = shutdown;
        loop {
            match rx.changed().await {
                Ok(()) if *rx.borrow() => break,
                Err(_) => break,
                _ => {}
            }
        }
        return Ok(());
    }

    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let res = dashboard_loop(
        &mut terminal,
        metrics,
        warn_log,
        started_at,
        addr,
        backend_name,
        shutdown,
        shutdown_trigger,
    )
    .await;

    // Restore terminal unconditionally — ignore secondary errors.
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    res
}

// ---------------------------------------------------------------------------
// Render loop (500 ms tick)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn dashboard_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    metrics: Arc<Mutex<MetricsStore>>,
    warn_log: WarnBuffer,
    started_at: Instant,
    addr: String,
    backend_name: String,
    mut shutdown: watch::Receiver<bool>,
    shutdown_trigger: watch::Sender<bool>,
) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_millis(500));

    loop {
        tokio::select! {
            biased;
            res = shutdown.changed() => {
                // Sender sent `true` OR sender was dropped → stop.
                if res.is_err() || *shutdown.borrow() { break; }
            }
            _ = interval.tick() => {}
        }

        // In raw mode, Ctrl+C is not converted to SIGINT — it arrives as a
        // key event.  Poll once (non-blocking) and handle it here.
        let ctrl_c = tokio::task::block_in_place(|| {
            event_poll(Duration::ZERO).unwrap_or(false)
                && matches!(
                    event_read(),
                    Ok(Event::Key(k))
                        if k.code == KeyCode::Char('c')
                            && k.modifiers.contains(KeyModifiers::CONTROL)
                )
        });
        if ctrl_c {
            // Signal the HTTP server to shut down gracefully.
            let _ = shutdown_trigger.send(true);
            break;
        }

        let (summary, recent) = {
            match metrics.lock() {
                Ok(m) => (m.session_summary(), m.recent(3).to_vec()),
                Err(_) => continue,
            }
        };

        let warns = warn_log
            .lock()
            .map(|b| b.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let uptime = started_at.elapsed();
        let addr = addr.as_str();
        let backend_name = backend_name.as_str();

        terminal.draw(|f| render(f, &summary, &recent, &warns, uptime, addr, backend_name))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Top-level layout: header | metrics | logs
// ---------------------------------------------------------------------------

fn render(
    f: &mut Frame,
    summary: &SessionSummary,
    recent: &[CompressionRecord],
    warns: &[WarnEntry],
    uptime: Duration,
    addr: &str,
    backend_name: &str,
) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // header: 6 art rows + 2 border rows
            Constraint::Length(9), // metrics panel
            Constraint::Length(7), // recent commands (3 entries + borders)
            Constraint::Min(4),    // warnings & errors console
        ])
        .split(area);

    render_header(f, chunks[0], uptime, addr, backend_name);
    render_metrics(f, chunks[1], summary);
    render_logs(f, chunks[2], recent);
    render_warn_log(f, chunks[3], warns);
}

// ---------------------------------------------------------------------------
// Header panel
// ---------------------------------------------------------------------------

fn render_header(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    uptime: Duration,
    addr: &str,
    backend_name: &str,
) {
    let version = env!("CARGO_PKG_VERSION");
    let uptime_str = fmt_uptime(uptime);

    // Side-by-side: ASCII art (left) + info text (right).
    // The art lines are padded to a fixed width so the info column is stable.
    let info: [String; 6] = [
        String::new(),
        "  Neural Token Killer".to_string(),
        format!("  v{version}  •  {addr}"),
        format!("  Uptime: {uptime_str}"),
        format!("  Backend: {backend_name}"),
        String::new(),
    ];

    let lines: Vec<Line> = NTK_ART
        .iter()
        .zip(info.iter())
        .map(|((art_text, color_idx), info_text)| {
            // Pad art to 28 chars so info always starts at the same column.
            let padded = format!("{art_text:<28}");
            Line::from(vec![
                Span::styled(
                    padded,
                    Style::default()
                        .fg(art_color(*color_idx))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    info_text.clone(),
                    Style::default().fg(Color::Rgb(200, 200, 200)),
                ),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(27, 107, 44)));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

// ---------------------------------------------------------------------------
// Metrics panel
// ---------------------------------------------------------------------------

fn render_metrics(f: &mut Frame, area: ratatui::layout::Rect, s: &SessionSummary) {
    let total_out = s.total_compressed_tokens;
    let total_in = s.total_original_tokens;
    let avg_pct = if s.total_compressions > 0 {
        (s.average_ratio * 100.0) as u64
    } else {
        0
    };

    let total_layers = s.layer_counts.iter().sum::<usize>().max(1);
    let [l1, l2, l3] = s.layer_counts;

    const BAR_W: usize = 20;

    let lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("  Compressions: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                fmt_num(s.total_compressions),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("     Tokens In: ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt_num(total_in), Style::default().fg(Color::White)),
            Span::styled("  →  Out: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                fmt_num(total_out),
                Style::default().fg(Color::Rgb(0, 230, 118)),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Saved: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                fmt_num(s.total_tokens_saved),
                Style::default()
                    .fg(Color::Rgb(0, 230, 118))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " tokens  •  Avg ratio: ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{avg_pct}%"),
                Style::default()
                    .fg(Color::Rgb(46, 155, 71))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        layer_line("L1", l1, total_layers, BAR_W, Color::Rgb(46, 155, 71)),
        layer_line("L2", l2, total_layers, BAR_W, Color::Rgb(27, 107, 44)),
        layer_line("L3", l3, total_layers, BAR_W, Color::Rgb(30, 140, 85)),
    ];

    let block = Block::default()
        .title(Span::styled(
            " SESSION METRICS ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(27, 107, 44)));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn layer_line<'a>(
    label: &'a str,
    count: usize,
    total: usize,
    bar_w: usize,
    color: Color,
) -> Line<'a> {
    let filled = count
        .saturating_mul(bar_w)
        .checked_div(total)
        .unwrap_or(0)
        .min(bar_w);
    let bar = format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(bar_w.saturating_sub(filled))
    );
    Line::from(vec![
        Span::styled(format!("  {label}  "), Style::default().fg(Color::DarkGray)),
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(
            format!("  {}", fmt_num(count)),
            Style::default().fg(Color::White),
        ),
        Span::styled(" runs", Style::default().fg(Color::DarkGray)),
    ])
}

// ---------------------------------------------------------------------------
// Logs panel
// ---------------------------------------------------------------------------

fn render_logs(f: &mut Frame, area: ratatui::layout::Rect, recent: &[CompressionRecord]) {
    let mut lines: Vec<Line> = Vec::new();

    if recent.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Waiting for commands…",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Most-recent first.
        for r in recent.iter().rev() {
            let ts = r.timestamp.format("%H:%M:%S").to_string();
            let cmd = if r.command.len() > 24 {
                format!("{:.21}…", &r.command[..21.min(r.command.len())])
            } else {
                format!("{:<24}", r.command)
            };
            let saved_pct = (r.ratio() * 100.0) as u64;
            let layer_color = match r.layer_used {
                1 => Color::Rgb(46, 155, 71),
                2 => Color::Rgb(27, 107, 44),
                _ => Color::Rgb(30, 140, 85),
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {ts}  "), Style::default().fg(Color::DarkGray)),
                Span::styled(cmd, Style::default().fg(Color::White)),
                Span::styled(
                    format!("  {:>7}", fmt_num(r.original_tokens)),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(" → ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<7}", fmt_num(r.compressed_tokens)),
                    Style::default().fg(Color::Rgb(0, 230, 118)),
                ),
                Span::styled("tok  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("L{}", r.layer_used),
                    Style::default()
                        .fg(layer_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {saved_pct}% saved"),
                    Style::default().fg(Color::Rgb(0, 230, 118)),
                ),
            ]));
        }
    }

    let block = Block::default()
        .title(Span::styled(
            " RECENT COMMANDS ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(27, 107, 44)));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

// ---------------------------------------------------------------------------
// Warn log panel
// ---------------------------------------------------------------------------

fn render_warn_log(f: &mut Frame, area: ratatui::layout::Rect, warns: &[WarnEntry]) {
    let mut lines: Vec<Line> = Vec::new();

    if warns.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No warnings or errors",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let max_rows = area.height.saturating_sub(2) as usize;
        let max_msg = area.width.saturating_sub(22) as usize; // 22 = timestamp + level + padding

        for entry in warns.iter().rev().take(max_rows) {
            let ts = entry.timestamp.format("%H:%M:%S").to_string();
            let (level_str, level_color) = match entry.level {
                WarnLevel::Warn => ("WARN ", Color::Rgb(255, 180, 0)),
                WarnLevel::Error => ("ERROR", Color::Rgb(220, 60, 60)),
            };
            let msg = if entry.message.len() > max_msg && max_msg > 1 {
                format!("{:.len$}…", &entry.message, len = max_msg.saturating_sub(1))
            } else {
                entry.message.clone()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {ts}  "), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{level_str}  "),
                    Style::default()
                        .fg(level_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(msg, Style::default().fg(Color::Rgb(200, 200, 200))),
            ]));
        }
    }

    let block = Block::default()
        .title(Span::styled(
            " WARNINGS & ERRORS ",
            Style::default()
                .fg(Color::Rgb(255, 180, 0))
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(180, 120, 0)));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn fmt_uptime(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}h {}m {}s", s / 3600, (s % 3600) / 60, s % 60)
    }
}

// ---------------------------------------------------------------------------
// Attach mode — remote TUI (for `ntk start` when daemon already running)
// ---------------------------------------------------------------------------

/// The JSON payload returned by `GET /state`.
#[derive(Debug, Deserialize)]
pub struct DaemonState {
    pub summary: SessionSummary,
    pub recent: Vec<CompressionRecord>,
    pub warns: Vec<WarnEntry>,
    pub uptime_secs: u64,
    pub addr: String,
    pub backend_name: String,
}

/// Connect to a running daemon and render the live TUI by polling `/state`.
/// Ctrl+C exits the TUI without stopping the daemon.
pub async fn run_attach_dashboard(url: String) -> Result<()> {
    if !std::io::stdout().is_tty() {
        println!("NTK daemon is running at {url}  (attach mode — Ctrl-C to detach)");
        // Block until user presses Ctrl+C.
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }

    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let res = attach_loop(&mut terminal, url).await;

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    res
}

async fn attach_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, url: String) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()?;
    let state_url = format!("{url}/state");
    let mut interval = tokio::time::interval(Duration::from_millis(500));

    // Last known state — displayed even if a fetch temporarily fails.
    let mut last: Option<DaemonState> = None;

    loop {
        interval.tick().await;

        // Non-blocking check for Ctrl+C key event (raw mode suppresses SIGINT).
        let quit = tokio::task::block_in_place(|| {
            event_poll(Duration::ZERO).unwrap_or(false)
                && matches!(
                    event_read(),
                    Ok(Event::Key(k))
                        if k.code == KeyCode::Char('c')
                            && k.modifiers.contains(KeyModifiers::CONTROL)
                )
        });
        if quit {
            break;
        }

        // Fetch state from daemon.
        if let Ok(resp) = client.get(&state_url).send().await {
            if let Ok(state) = resp.json::<DaemonState>().await {
                last = Some(state);
            }
        }

        if let Some(ref state) = last {
            let uptime = Duration::from_secs(state.uptime_secs);
            terminal.draw(|f| {
                render(
                    f,
                    &state.summary,
                    &state.recent,
                    &state.warns,
                    uptime,
                    &state.addr,
                    &state.backend_name,
                )
            })?;
        } else {
            // Daemon not yet responding — show a waiting screen.
            terminal.draw(|f| {
                let area = f.area();
                let p = Paragraph::new("  Connecting to NTK daemon…")
                    .block(Block::default().borders(Borders::ALL).title(" NTK "));
                f.render_widget(p, area);
            })?;
        }
    }
    Ok(())
}

/// Format a number with thousands separators: 1234567 → "1,234,567".
fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len().saturating_add(s.len() / 3));
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}
