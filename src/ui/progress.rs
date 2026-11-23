//! Download progress formatting and an in-place progress line for terminals.
//!
//! Formatting is pure (testable). The live line is only drawn when stderr is
//! an interactive terminal; it uses a carriage return and space padding (no
//! ANSI cursor sequences) and is cleared before any other output is written.
//! Non-interactive output never contains progress redraws.

use std::io::Write;
use std::time::{Duration, Instant};

/// Formats a byte count with binary units (`512 B`, `1.5 KiB`, `12.0 MiB`).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Formats one progress line (without the status verb).
///
/// With a known total: `name [=====>     ]  45%  1.2 MiB/2.7 MiB`.
/// With an unknown total, no bar or percentage is shown, because either
/// would be invented: `name  1.2 MiB`.
pub fn format_download_progress(
    name: &str,
    downloaded: u64,
    total: Option<u64>,
    width: usize,
) -> String {
    format_download_progress_with(name, downloaded, total, width, false)
}

/// Like [`format_download_progress`]; with `unicode` the bar is drawn with
/// block characters (`\u{2588}\u{2591}`) instead of ASCII (`=>`).
pub fn format_download_progress_with(
    name: &str,
    downloaded: u64,
    total: Option<u64>,
    width: usize,
    unicode: bool,
) -> String {
    match total.filter(|t| *t > 0) {
        None => format!("{name}  {}", format_bytes(downloaded)),
        Some(total) => {
            let done = downloaded.min(total);
            let percent = (done.saturating_mul(100) / total).min(100);
            let sizes = format!("{}/{}", format_bytes(done), format_bytes(total));
            // 13 = status column + space; fixed parts: " [" "] " " 100%  " ...
            let fixed = 13 + name.len() + 2 + 2 + 5 + 2 + sizes.len();
            let bar_width = width.saturating_sub(fixed).clamp(0, 40);
            if bar_width < 5 {
                return format!("{name} {percent:>3}%  {sizes}");
            }
            let filled = (bar_width as u64 * done / total) as usize;
            let bar = if unicode {
                format!(
                    "{}{}",
                    "\u{2588}".repeat(filled),
                    "\u{2591}".repeat(bar_width - filled)
                )
            } else {
                let mut bar = "=".repeat(filled);
                if filled < bar_width {
                    bar.push('>');
                    bar.push_str(&" ".repeat(bar_width - filled - 1));
                }
                bar
            };
            format!("{name} [{bar}] {percent:>3}%  {sizes}")
        }
    }
}

/// An in-place progress line drawn on an interactive stderr.
pub struct LiveLine {
    enabled: bool,
    last_draw: Option<Instant>,
    last_len: usize,
    min_interval: Duration,
}

impl LiveLine {
    /// Creates a live line; `enabled` must only be true for an interactive
    /// terminal (see [`crate::ui::live_progress_enabled`]).
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_draw: None,
            last_len: 0,
            min_interval: Duration::from_millis(100),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Redraws the line, throttled to at most ten updates per second unless
    /// `force` is set.
    pub fn draw(&mut self, text: &str, force: bool) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        if !force
            && self
                .last_draw
                .is_some_and(|last| now.duration_since(last) < self.min_interval)
        {
            return;
        }
        self.last_draw = Some(now);
        let visible = super::style::strip_ansi(text).chars().count();
        let pad = self.last_len.saturating_sub(visible);
        let mut err = std::io::stderr();
        let _ = write!(err, "\r{text}{}", " ".repeat(pad));
        let _ = err.flush();
        self.last_len = visible;
    }

    /// Erases the line so subsequent output starts at column 0.
    pub fn clear(&mut self) {
        if self.enabled && self.last_len > 0 {
            let mut err = std::io::stderr();
            let _ = write!(err, "\r{}\r", " ".repeat(self.last_len));
            let _ = err.flush();
            self.last_len = 0;
        }
    }
}

impl Drop for LiveLine {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_use_binary_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn known_total_shows_bar_and_percentage() {
        let line = format_download_progress("fmt v11.1.1", 512, Some(1024), 80);
        assert!(line.starts_with("fmt v11.1.1 ["), "{line}");
        assert!(line.contains(" 50%"), "{line}");
        assert!(line.ends_with("512 B/1.0 KiB"), "{line}");
        assert!(
            13 + line.len() <= 80,
            "line must fit the width: {}",
            line.len()
        );
        let done = format_download_progress("fmt v11.1.1", 1024, Some(1024), 80);
        assert!(done.contains("100%") && !done.contains('>'), "{done}");
    }

    #[test]
    fn unicode_bar_uses_blocks_and_keeps_width() {
        let ascii = format_download_progress_with("pkg", 1, Some(2), 80, false);
        let blocks = format_download_progress_with("pkg", 1, Some(2), 80, true);
        assert!(
            blocks.contains('\u{2588}') && blocks.contains('\u{2591}'),
            "{blocks}"
        );
        assert!(!blocks.contains('=') && !blocks.contains('>'));
        assert_eq!(ascii.chars().count(), blocks.chars().count());
        // Unknown totals never draw a bar in either mode.
        assert!(!format_download_progress_with("pkg", 5, None, 80, true).contains('\u{2588}'));
    }

    #[test]
    fn unknown_total_never_shows_percentage_or_bar() {
        let line = format_download_progress("zlib v1.3.1", 2 * 1024 * 1024, None, 80);
        assert_eq!(line, "zlib v1.3.1  2.0 MiB");
        assert!(!line.contains('%') && !line.contains('['));
        let zero = format_download_progress("zlib v1.3.1", 10, Some(0), 80);
        assert!(
            !zero.contains('%'),
            "a zero Content-Length is treated as unknown"
        );
    }

    #[test]
    fn narrow_terminal_drops_the_bar() {
        let line = format_download_progress("a-very-long-package-name v1.0.0", 1, Some(2), 40);
        assert!(!line.contains('['), "{line}");
        assert!(line.contains(" 50%"));
    }

    #[test]
    fn disabled_live_line_writes_nothing() {
        let mut line = LiveLine::new(false);
        line.draw("anything", true);
        line.clear();
        assert!(!line.is_enabled());
    }
}
