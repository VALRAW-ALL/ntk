// ---------------------------------------------------------------------------
// Terminal utilities: ANSI colors + braille spinner
//
// Colors are emitted only when stdout is a TTY and NO_COLOR is not set.
// On Windows, ENABLE_VIRTUAL_TERMINAL_PROCESSING is enabled on first use.
// ---------------------------------------------------------------------------

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

// ---------------------------------------------------------------------------
// TTY detection + Windows ANSI init
// ---------------------------------------------------------------------------

fn color_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        #[cfg(windows)]
        {
            enable_windows_ansi();
        }
        is_stdout_tty()
    })
}

fn is_stdout_tty() -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: isatty is always safe to call with a valid fd.
        libc_isatty(std::io::stdout().as_raw_fd())
    }
    #[cfg(not(unix))]
    {
        windows_is_tty()
    }
}

#[cfg(unix)]
fn libc_isatty(fd: i32) -> bool {
    extern "C" {
        fn isatty(fd: i32) -> i32;
    }
    unsafe { isatty(fd) != 0 }
}

#[cfg(windows)]
fn windows_is_tty() -> bool {
    use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE};
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle == 0 || handle == usize::MAX as isize {
            return false;
        }
        let mut mode = 0u32;
        GetConsoleMode(handle, &mut mode) != 0
    }
}

#[cfg(windows)]
fn enable_windows_ansi() {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        STD_OUTPUT_HANDLE,
    };
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle == 0 {
            return;
        }
        let mut mode = 0u32;
        if GetConsoleMode(handle, &mut mode) != 0 {
            SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

// ---------------------------------------------------------------------------
// Color codes — return empty string when colors are disabled
// ---------------------------------------------------------------------------

macro_rules! color {
    ($name:ident, $code:expr) => {
        pub fn $name() -> &'static str {
            if color_enabled() {
                $code
            } else {
                ""
            }
        }
    };
}

color!(reset, "\x1b[0m");
color!(bold, "\x1b[1m");
color!(dim, "\x1b[2m");
color!(cyan, "\x1b[36m");
color!(bright_cyan, "\x1b[96m");
color!(green, "\x1b[32m");
color!(bright_green, "\x1b[92m");
color!(yellow, "\x1b[33m");
color!(bright_yellow, "\x1b[93m");
color!(red, "\x1b[31m");
color!(bright_red, "\x1b[91m");
color!(blue, "\x1b[34m");
color!(bright_blue, "\x1b[94m");
color!(magenta, "\x1b[35m");
color!(white, "\x1b[97m");
color!(gray, "\x1b[90m");

/// Warning color — bright yellow. Semantic alias for diagnostics output.
pub fn warn_color() -> &'static str {
    bright_yellow()
}

/// Color a ratio value: ≤40% green, ≤80% yellow, >80% red.
pub fn ratio_color(ratio_pct: usize) -> &'static str {
    if !color_enabled() {
        return "";
    }
    if ratio_pct <= 40 {
        "\x1b[92m" // bright green
    } else if ratio_pct <= 80 {
        "\x1b[93m" // bright yellow
    } else {
        "\x1b[91m" // bright red
    }
}

/// Color a latency value: ≤1s green, ≤5s yellow, >5s red.
pub fn latency_color(ms: u64) -> &'static str {
    if !color_enabled() {
        return "";
    }
    if ms <= 1_000 {
        "\x1b[92m"
    } else if ms <= 5_000 {
        "\x1b[93m"
    } else {
        "\x1b[91m"
    }
}

/// Erase the current terminal line (move to column 0, then clear to end).
pub fn clear_line() {
    if color_enabled() {
        print!("\r\x1b[2K");
    } else {
        print!("\r");
    }
    let _ = std::io::stdout().flush();
}

// ---------------------------------------------------------------------------
// Spinner — animated braille dots, runs in a background thread
// ---------------------------------------------------------------------------

pub struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Spinner {
    /// Start a spinner with the given label on the current line.
    pub fn start(label: &str) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let label = label.to_owned();

        let handle = std::thread::spawn(move || {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut frame_iter = frames.iter().cycle();
            while !stop_clone.load(Ordering::Relaxed) {
                let frame = frame_iter.next().unwrap_or(&"⠋");
                if color_enabled() {
                    print!("\r\x1b[96m{frame}\x1b[0m {label}");
                } else {
                    print!("\r{frame} {label}");
                }
                let _ = std::io::stdout().flush();
                std::thread::sleep(Duration::from_millis(80));
            }
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }

    /// Stop the spinner and clear the line. Call before printing the result.
    pub fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        clear_line();
    }

    /// Stop and print a success line.
    pub fn finish_ok(self, msg: &str) {
        self.finish();
        println!("{}{}✓{} {msg}", bold(), green(), reset());
    }

    /// Stop and print an error line.
    pub fn finish_err(self, msg: &str) {
        self.finish();
        println!("{}{}✗{} {msg}", bold(), bright_red(), reset());
    }

    /// Stop and print a warning line.
    pub fn finish_warn(self, msg: &str) {
        self.finish();
        println!("{}{}◌{} {msg}", bold(), bright_yellow(), reset());
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        clear_line();
    }
}

// ---------------------------------------------------------------------------
// BenchSpinner — spinner that also shows elapsed time in real time
// ---------------------------------------------------------------------------

pub struct BenchSpinner {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl BenchSpinner {
    /// Start a bench spinner. Shows label + elapsed time updating every 250ms.
    pub fn start(label: &str, input_chars: usize) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let label = label.to_owned();

        let handle = std::thread::spawn(move || {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let start = std::time::Instant::now();
            let mut frame_iter = frames.iter().cycle();

            while !stop_clone.load(Ordering::Relaxed) {
                let elapsed = start.elapsed().as_secs_f64();
                let frame = frame_iter.next().unwrap_or(&"⠋");
                let chars_label = if input_chars > 0 {
                    format!("  {}[{} chars]{}", "\x1b[90m", input_chars, "\x1b[0m")
                } else {
                    String::new()
                };
                if color_enabled() {
                    print!(
                        "\r\x1b[96m{frame}\x1b[0m \x1b[1m{label:<28}\x1b[0m  \x1b[93m{elapsed:>7.1}s\x1b[0m{chars_label}",
                    );
                } else {
                    print!("\r{frame} {label:<28}  {elapsed:>7.1}s");
                }
                let _ = std::io::stdout().flush();
                std::thread::sleep(Duration::from_millis(250));
            }
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }

    /// Stop spinner and clear the line.
    pub fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        clear_line();
    }
}

impl Drop for BenchSpinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        clear_line();
    }
}

// ---------------------------------------------------------------------------
// Convenience: print a section header
// ---------------------------------------------------------------------------

pub fn print_header(title: &str, separator: &str) {
    println!("{}{}{}{}", bold(), bright_cyan(), title, reset());
    println!("{}{}{}", dim(), separator, reset());
}

pub fn ok_mark() -> String {
    format!("{}{}✓{}", bold(), bright_green(), reset())
}

pub fn err_mark() -> String {
    format!("{}{}✗{}", bold(), bright_red(), reset())
}

pub fn warn_mark() -> String {
    format!("{}{}◌{}", bold(), bright_yellow(), reset())
}
