//! Terminal presentation layer: color policy, styling, structured
//! diagnostics, Cargo-style status lines, and download progress.
//!
//! This module only *renders*. Execution code (build, source acquisition)
//! reports plain data, such as source acquisition events or a
//! `BuildError`, and the CLI turns it into output through these helpers.
//! Output is ASCII-only, so it needs no Unicode-capable terminal.

pub mod color;
pub mod diagnostic;
pub mod progress;
pub mod shell;
pub mod style;
pub mod writer;

pub use color::{ColorChoice, ColorEnv, should_colorize};
pub use diagnostic::{Diagnostic, Level, display_command};
pub use progress::{
    LiveLine, format_bytes, format_download_progress, format_download_progress_with,
};
pub use shell::{
    ConfigGuard, UiConfig, Verbosity, config, emit, format_status, is_verbose,
    live_progress_enabled, set_config, status, style, verbose_status, verbosity,
};
pub use style::{Style, strip_ansi};
pub use writer::{PipeTolerantWriter, StyledWriter};
