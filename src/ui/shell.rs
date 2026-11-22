//! Per-run terminal configuration and Cargo-style status lines.
//!
//! The configuration is set once per CLI invocation (`crux::cli::run`) and
//! stored per thread, so in-process tests running in parallel never see each
//! other's settings. Status lines are written to the writer the caller passes
//! (normally stderr), so `stdout` stays clean for command results.

use super::diagnostic::{Diagnostic, Level};
use super::style::Style;
use std::cell::RefCell;
use std::io::{self, Write};

/// Output verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Verbosity {
    /// `-q`: no routine progress; errors and warnings remain.
    Quiet,
    #[default]
    Normal,
    /// `-v`: extra package, source, and build information.
    Verbose,
    /// `-vv`: executed commands and full subprocess output.
    VeryVerbose,
}

/// Terminal settings for one CLI invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiConfig {
    pub color: bool,
    pub verbosity: Verbosity,
    pub stderr_is_terminal: bool,
    pub width: usize,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            color: false,
            verbosity: Verbosity::Normal,
            stderr_is_terminal: false,
            width: 80,
        }
    }
}

thread_local! {
    static CONFIG: RefCell<UiConfig> = RefCell::new(UiConfig::default());
}

/// Returns the configuration of the current invocation.
pub fn config() -> UiConfig {
    CONFIG.with(|c| *c.borrow())
}

/// Installs `config` for the current thread until the guard is dropped.
pub fn set_config(config: UiConfig) -> ConfigGuard {
    let previous = CONFIG.with(|c| std::mem::replace(&mut *c.borrow_mut(), config));
    ConfigGuard { previous }
}

/// Restores the previous configuration on drop.
pub struct ConfigGuard {
    previous: UiConfig,
}

impl Drop for ConfigGuard {
    fn drop(&mut self) {
        CONFIG.with(|c| *c.borrow_mut() = self.previous);
    }
}

/// The active styling palette.
pub fn style() -> Style {
    Style::new(config().color)
}

pub fn verbosity() -> Verbosity {
    config().verbosity
}

pub fn is_verbose() -> bool {
    verbosity() >= Verbosity::Verbose
}

/// Whether an in-place progress line may be drawn: only on an interactive
/// stderr and outside quiet mode.
pub fn live_progress_enabled() -> bool {
    let c = config();
    c.stderr_is_terminal && c.verbosity > Verbosity::Quiet
}

/// Terminal width from `COLUMNS`, defaulting to 80 and clamped to [40, 400].
pub fn detect_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(80)
        .clamp(40, 400)
}

/// Formats `   Compiling fmt v11.1.1` (verb right-aligned in 12 columns).
pub fn format_status(style: &Style, verb: &str, message: &str) -> String {
    format!("{} {message}", style.success(&format!("{verb:>12}")))
}

/// Writes a routine status line (suppressed by `--quiet`).
pub fn status<W: Write + ?Sized>(w: &mut W, verb: &str, message: &str) -> io::Result<()> {
    if verbosity() == Verbosity::Quiet {
        return Ok(());
    }
    writeln!(w, "{}", format_status(&style(), verb, message))
}

/// Writes a status line shown only with `-v` or more.
pub fn verbose_status<W: Write + ?Sized>(w: &mut W, verb: &str, message: &str) -> io::Result<()> {
    if !is_verbose() {
        return Ok(());
    }
    writeln!(w, "{}", format_status(&style(), verb, message))
}

/// Writes a diagnostic. Errors and warnings are always shown; notes, info,
/// and success diagnostics are suppressed by `--quiet`.
pub fn emit<W: Write + ?Sized>(w: &mut W, diagnostic: &Diagnostic) -> io::Result<()> {
    if verbosity() == Verbosity::Quiet && diagnostic.level > Level::Warning {
        return Ok(());
    }
    w.write_all(diagnostic.render(&style()).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_alignment_and_quiet_suppression() {
        let _guard = set_config(UiConfig::default());
        let mut out = Vec::new();
        status(&mut out, "Compiling", "fmt v11.1.1").unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "   Compiling fmt v11.1.1\n"
        );

        let _quiet = set_config(UiConfig {
            verbosity: Verbosity::Quiet,
            ..UiConfig::default()
        });
        let mut out = Vec::new();
        status(&mut out, "Compiling", "fmt").unwrap();
        emit(&mut out, &Diagnostic::new(Level::Note, "hidden")).unwrap();
        emit(&mut out, &Diagnostic::error("shown")).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "error: shown\n");
    }

    #[test]
    fn verbose_status_requires_verbose() {
        let _guard = set_config(UiConfig::default());
        let mut out = Vec::new();
        verbose_status(&mut out, "Fresh", "zlib").unwrap();
        assert!(out.is_empty());
        let _v = set_config(UiConfig {
            verbosity: Verbosity::Verbose,
            ..UiConfig::default()
        });
        verbose_status(&mut out, "Fresh", "zlib").unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "       Fresh zlib\n");
    }

    #[test]
    fn config_guard_restores_previous_settings() {
        assert!(!config().color);
        {
            let _g = set_config(UiConfig {
                color: true,
                ..UiConfig::default()
            });
            assert!(config().color);
        }
        assert!(!config().color);
    }

    #[test]
    fn live_progress_requires_terminal_and_not_quiet() {
        let _g = set_config(UiConfig {
            stderr_is_terminal: true,
            ..UiConfig::default()
        });
        assert!(live_progress_enabled());
        let _q = set_config(UiConfig {
            stderr_is_terminal: true,
            verbosity: Verbosity::Quiet,
            ..UiConfig::default()
        });
        assert!(!live_progress_enabled());
    }
}
