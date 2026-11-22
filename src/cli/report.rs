//! CLI-side presentation glue: turns build and acquisition outcomes into
//! diagnostics and status lines. Execution code never formats output; it
//! returns structured errors and events that are rendered here.

use crate::build::BuildError;
use crate::cli::CliError;
use crate::package::PackageId;
use crate::ui::{self, Diagnostic};
use std::io::Write;
use std::path::Path;

/// Formats a package as `name vVERSION`, the style used in status lines.
pub fn package_label(id: &PackageId) -> String {
    format!("{} v{}", id.name(), id.version())
}

/// Lines of original tool output shown on failure before eliding the middle
/// (the complete log is always kept on disk; `-v` prints it all).
pub const FAILURE_OUTPUT_HEAD_LINES: usize = 60;
/// Lines shown from the end of the original output on failure.
pub const FAILURE_OUTPUT_TAIL_LINES: usize = 140;

/// Builds the structured diagnostic for a CMake configure/build failure.
/// Returns `None` for other errors, which keep their existing message.
pub fn build_failure_diagnostic(err: &BuildError, package: &str) -> Option<Diagnostic> {
    let (stage, title, tool, exit_code, log_path, command, build_dir, summary) = match err {
        BuildError::ConfigureFailed {
            exit_code,
            log_path,
            command,
            build_dir,
            summary,
        } => (
            "CMake configuration",
            "package configuration failed",
            "CMake",
            exit_code,
            log_path,
            command,
            build_dir,
            summary,
        ),
        BuildError::BuildFailed {
            exit_code,
            log_path,
            command,
            build_dir,
            summary,
        } => (
            "compilation and linking (cmake --build)",
            "package compilation failed",
            "The build tool",
            exit_code,
            log_path,
            command,
            build_dir,
            summary,
        ),
        _ => return None,
    };

    let status = match exit_code {
        Some(code) => code.to_string(),
        None => "terminated by a signal".to_string(),
    };
    let explanation = match exit_code {
        Some(0) => summary.clone(),
        Some(code) => format!("{tool} exited with status {code}."),
        None => format!("{tool} was terminated by a signal."),
    };
    let mut diagnostic = Diagnostic::error(title)
        .field("package", package)
        .field("stage", stage)
        .field("exit status", status)
        .field("build dir", build_dir.display().to_string())
        .field("log", log_path.display().to_string())
        .message(explanation);
    if !command.is_empty() {
        diagnostic = diagnostic.command(command.clone());
    }
    diagnostic = diagnostic
        .note("the original CMake and compiler output follows, unmodified")
        .help(format!(
            "inspect the build directory or the complete log at {}",
            log_path.display()
        ));
    if ui::verbosity() < ui::Verbosity::VeryVerbose {
        diagnostic = diagnostic.help("re-run with -vv to stream CMake and compiler output live");
    }
    Some(diagnostic)
}

/// Writes a build failure: a structured diagnostic, then the original tool
/// output verbatim. Long output is shortened in the middle unless `-v` is
/// active; the complete log always stays on disk. Other errors are printed
/// with their existing message.
pub fn write_build_failure<E: Write + ?Sized>(
    stderr: &mut E,
    err: &BuildError,
    package: &str,
) -> Result<(), CliError> {
    let Some(diagnostic) = build_failure_diagnostic(err, package) else {
        writeln!(stderr, "error: {err}")?;
        return Ok(());
    };
    ui::emit(stderr, &diagnostic)?;
    if let BuildError::ConfigureFailed { log_path, .. } | BuildError::BuildFailed { log_path, .. } =
        err
    {
        write_original_output(stderr, log_path)?;
    }
    Ok(())
}

/// Prints a tool log framed by markers. Content is never reformatted.
pub fn write_original_output<E: Write + ?Sized>(
    stderr: &mut E,
    log_path: &Path,
) -> Result<(), CliError> {
    let style = ui::style();
    let content = match std::fs::read(log_path) {
        Ok(content) => content,
        Err(error) => {
            writeln!(
                stderr,
                "warning: failed to read log '{}': {error}",
                log_path.display()
            )?;
            return Ok(());
        }
    };
    let text = String::from_utf8_lossy(&content);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    writeln!(
        stderr,
        "\n{}",
        style.dim(&format!("--- original output ({}) ---", log_path.display()))
    )?;
    let limit = FAILURE_OUTPUT_HEAD_LINES + FAILURE_OUTPUT_TAIL_LINES;
    if ui::is_verbose() || lines.len() <= limit {
        stderr.write_all(text.as_bytes())?;
    } else {
        for line in &lines[..FAILURE_OUTPUT_HEAD_LINES] {
            stderr.write_all(line.as_bytes())?;
        }
        let omitted = lines.len() - limit;
        writeln!(
            stderr,
            "{}",
            style.dim(&format!(
                "... {omitted} lines omitted (complete log: {}; or re-run with -v) ...",
                log_path.display()
            ))
        )?;
        for line in &lines[lines.len() - FAILURE_OUTPUT_TAIL_LINES..] {
            stderr.write_all(line.as_bytes())?;
        }
    }
    if !text.ends_with('\n') {
        writeln!(stderr)?;
    }
    writeln!(stderr, "{}", style.dim("--- end of original output ---"))?;
    Ok(())
}

/// With `-vv`, returns an observer that streams CMake and compiler output to
/// the process's stderr as it is produced; otherwise `None`.
pub fn live_output_observer() -> Option<crate::core::process::OutputObserver> {
    (ui::verbosity() >= ui::Verbosity::VeryVerbose).then(|| {
        crate::core::process::OutputObserver(std::sync::Arc::new(|bytes: &[u8]| {
            let mut err = std::io::stderr();
            let _ = err.write_all(bytes);
            let _ = err.flush();
        }))
    })
}

/// Builds one package with consistent reporting: `Compiling name vX` before
/// a real build (or `Fresh name vX` with `-v` when the cache is reused),
/// live output with `-vv`, and a structured diagnostic plus the original
/// output on failure.
pub fn build_package<E: Write + ?Sized>(
    engine: &crate::build::CMakeBuildEngine,
    request: crate::build::BuildRequest,
    stderr: &mut E,
) -> Result<crate::build::BuildResult, CliError> {
    use crate::build::BuildEngine;

    let label = package_label(&request.package_id);
    let request = match live_output_observer() {
        Some(observer) => request.with_output_observer(observer),
        None => request,
    };
    if engine.is_fresh(&request) {
        ui::verbose_status(stderr, "Fresh", &label)?;
    } else {
        ui::status(stderr, "Compiling", &label)?;
        ui::verbose_status(
            stderr,
            "Source",
            &ui::style().dim(&request.source_dir.display().to_string()),
        )?;
    }
    match engine.build(&request) {
        Ok(result) => {
            if !result.reused_cache {
                ui::verbose_status(
                    stderr,
                    "Built",
                    &format!(
                        "{label} in {:.2}s {}",
                        result.duration.as_secs_f64(),
                        ui::style().dim(&format!("({})", result.build_dir.display()))
                    ),
                )?;
            }
            Ok(result)
        }
        Err(err) => {
            write_build_failure(stderr, &err, &label)?;
            Err(CliError::Build(Box::new(err)))
        }
    }
}

/// Writes `    Finished `debug` profile in 1.23s`.
pub fn finished<E: Write + ?Sized>(
    stderr: &mut E,
    profile: crate::build::BuildProfile,
    started: std::time::Instant,
) -> Result<(), CliError> {
    ui::status(
        stderr,
        "Finished",
        &format!(
            "`{}` profile in {:.2}s",
            profile,
            started.elapsed().as_secs_f64()
        ),
    )?;
    Ok(())
}

/// Renders [`crate::source::AcquisitionEvent`]s.
///
/// On an interactive stderr the download is shown as an in-place progress
/// line, drawn directly on the terminal and cleared when the download ends.
/// Everything else is queued as status lines and written through the
/// command's normal stderr writer by [`AcquisitionReporter::flush_into`],
/// which commands call right after each acquisition returns. This keeps
/// output ordering, color policy, and test capture identical whether or not
/// stderr is a terminal, and non-interactive output contains no redraws.
pub struct AcquisitionReporter {
    state: std::sync::Mutex<ReporterState>,
}

struct ReporterState {
    live: ui::LiveLine,
    width: usize,
    pending: Vec<(&'static str, String, bool)>,
    downloaded: Option<u64>,
    git_url: Option<String>,
}

impl AcquisitionReporter {
    /// Creates a reporter for the current invocation's terminal settings.
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            state: std::sync::Mutex::new(ReporterState {
                live: ui::LiveLine::new(ui::live_progress_enabled()),
                width: ui::config().width,
                pending: Vec::new(),
                downloaded: None,
                git_url: None,
            }),
        })
    }

    /// Clears any live progress line and writes queued status lines.
    pub fn flush_into<W: Write + ?Sized>(&self, w: &mut W) -> Result<(), CliError> {
        let pending = match self.state.lock() {
            Ok(mut state) => {
                state.live.clear();
                std::mem::take(&mut state.pending)
            }
            Err(_) => Vec::new(),
        };
        for (verb, message, verbose_only) in pending {
            if verbose_only {
                ui::verbose_status(w, verb, &message)?;
            } else {
                ui::status(w, verb, &message)?;
            }
        }
        Ok(())
    }
}

impl crate::source::AcquisitionObserver for AcquisitionReporter {
    fn on_event(&self, package: &PackageId, event: &crate::source::AcquisitionEvent) {
        use crate::source::AcquisitionEvent as E;
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let label = package_label(package);
        let dim = |text: &str| ui::style().dim(text);
        match event {
            E::SourceCacheHit => state.pending.push((
                "Cached",
                format!("{label} {}", dim("(verified source cache)")),
                true,
            )),
            E::ArchiveCacheHit => state.pending.push((
                "Cached",
                format!("{label} {}", dim("(verified archive cache)")),
                true,
            )),
            E::DownloadStarted { url, total } => {
                let size = total
                    .map(|t| format!(", {}", ui::format_bytes(t)))
                    .unwrap_or_default();
                state.pending.push((
                    "Downloading",
                    format!("{label} {}", dim(&format!("({url}{size})"))),
                    true,
                ));
                let line = ui::format_download_progress(&label, 0, *total, state.width);
                let text = ui::format_status(&ui::style(), "Downloading", &line);
                state.live.draw(&text, true);
            }
            E::DownloadProgress { downloaded, total } => {
                let line = ui::format_download_progress(&label, *downloaded, *total, state.width);
                let text = ui::format_status(&ui::style(), "Downloading", &line);
                state.live.draw(&text, false);
            }
            E::DownloadFinished { bytes } => {
                state.live.clear();
                state.downloaded = Some(*bytes);
            }
            E::ChecksumVerified { checksum } => {
                let size = state
                    .downloaded
                    .take()
                    .map(|b| format!("{}, ", ui::format_bytes(b)))
                    .unwrap_or_default();
                state.pending.push((
                    "Downloaded",
                    format!("{label} {}", dim(&format!("({size}checksum verified)"))),
                    false,
                ));
                state
                    .pending
                    .push(("Verified", format!("{label} {}", dim(checksum)), true));
            }
            E::Extracted => state.pending.push(("Extracted", label, true)),
            E::GitCloning { url } => {
                state.git_url = Some(url.clone());
                let text = ui::format_status(&ui::style(), "Cloning", &label);
                state.live.draw(&text, true);
            }
            E::GitCommit { commit, cached } => {
                state.live.clear();
                let short = &commit[..commit.len().min(12)];
                let source = state.git_url.take();
                if *cached {
                    state.pending.push((
                        "Cached",
                        format!("{label} {}", dim(&format!("(git commit {short})"))),
                        true,
                    ));
                } else {
                    state.pending.push((
                        "Fetched",
                        format!("{label} {}", dim(&format!("(git commit {short})"))),
                        false,
                    ));
                    if let Some(url) = source {
                        state
                            .pending
                            .push(("Source", format!("{label} {}", dim(&url)), true));
                    }
                }
            }
            E::PathSnapshot { reused } => state.pending.push((
                "Snapshot",
                format!(
                    "{label} {}",
                    dim(if *reused {
                        "(local path, unchanged)"
                    } else {
                        "(local path, copied to cache)"
                    })
                ),
                true,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn failure(dir: &Path, lines: usize) -> BuildError {
        let log = dir.join("build.log");
        let mut text = String::from("$ cmake --build /b\n\n");
        text.push_str("src/format.cc:12:3: error: 'foo' was not declared in this scope\n");
        text.push_str("   12 |   foo();\n      |   ^~~\n");
        for i in 0..lines {
            text.push_str(&format!("line {i}\n"));
        }
        std::fs::write(&log, text).unwrap();
        BuildError::BuildFailed {
            exit_code: Some(2),
            log_path: log,
            summary: String::new(),
            command: "cmake --build /b --config Debug".to_string(),
            build_dir: PathBuf::from("/b"),
        }
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crux_report_{}",
            crate::core::filesystem::unique_temp_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn build_failure_reports_package_stage_command_and_preserves_output() {
        let _ui = ui::set_config(ui::UiConfig::default());
        let dir = temp();
        let mut out = Vec::new();
        write_build_failure(&mut out, &failure(&dir, 3), "fmt v11.1.1").unwrap();
        let text = String::from_utf8(out).unwrap();
        for expected in [
            "error: package compilation failed\n",
            "  package:     fmt v11.1.1\n",
            "  stage:       compilation and linking (cmake --build)\n",
            "  exit status: 2\n",
            "  build dir:   /b\n",
            "  The build tool exited with status 2.\n",
            "  command:\n    cmake --build /b --config Debug\n",
            "note: the original CMake and compiler output follows, unmodified",
            "help: re-run with -vv",
            // Original compiler diagnostics, verbatim, including the caret.
            "src/format.cc:12:3: error: 'foo' was not declared in this scope\n   12 |   foo();\n      |   ^~~\n",
            "--- end of original output ---\n",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(!text.contains("lines omitted"));
        assert!(!text.contains('\x1b'), "no ANSI without color");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_output_is_shortened_by_default_and_complete_with_verbose() {
        let dir = temp();
        let err = failure(&dir, 1000);
        {
            let _ui = ui::set_config(ui::UiConfig::default());
            let mut out = Vec::new();
            write_build_failure(&mut out, &err, "zlib v1.3.1").unwrap();
            let text = String::from_utf8(out).unwrap();
            assert!(
                text.contains("src/format.cc:12:3: error:"),
                "first error is kept"
            );
            assert!(text.contains("line 999\n"), "tail is kept");
            assert!(text.contains("lines omitted (complete log:"));
            assert!(!text.contains("line 500\n"));
        }
        {
            let _ui = ui::set_config(ui::UiConfig {
                verbosity: ui::Verbosity::Verbose,
                ..ui::UiConfig::default()
            });
            let mut out = Vec::new();
            write_build_failure(&mut out, &err, "zlib v1.3.1").unwrap();
            let text = String::from_utf8(out).unwrap();
            assert!(text.contains("line 500\n") && !text.contains("lines omitted"));
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn quiet_mode_still_reports_failures() {
        let _ui = ui::set_config(ui::UiConfig {
            verbosity: ui::Verbosity::Quiet,
            ..ui::UiConfig::default()
        });
        let dir = temp();
        let mut out = Vec::new();
        write_build_failure(&mut out, &failure(&dir, 1), "fmt v11.1.1").unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("error: package compilation failed"));
        assert!(text.contains("src/format.cc:12:3: error:"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn configure_failure_names_the_cmake_stage() {
        let err = BuildError::ConfigureFailed {
            exit_code: Some(1),
            log_path: PathBuf::from("/b/logs/configure.log"),
            summary: String::new(),
            command: "cmake -S /src -B /b".to_string(),
            build_dir: PathBuf::from("/b"),
        };
        let d = build_failure_diagnostic(&err, "fmt v11.1.1").unwrap();
        assert_eq!(d.title, "package configuration failed");
        assert!(
            d.fields
                .contains(&("stage".to_string(), "CMake configuration".to_string()))
        );
        assert!(
            d.message
                .contains(&"CMake exited with status 1.".to_string())
        );
        assert!(build_failure_diagnostic(&BuildError::MetadataError("x".into()), "p").is_none());
    }

    fn id(name: &str, version: &str) -> PackageId {
        PackageId::new(
            crate::package::PackageName::new(name).unwrap(),
            semver::Version::parse(version).unwrap(),
        )
    }

    fn render_events(
        verbosity: ui::Verbosity,
        events: &[crate::source::AcquisitionEvent],
    ) -> String {
        use crate::source::AcquisitionObserver;
        let _ui = ui::set_config(ui::UiConfig {
            verbosity,
            ..ui::UiConfig::default()
        });
        let reporter = AcquisitionReporter::new();
        let package = id("fmt", "11.1.1");
        for event in events {
            reporter.on_event(&package, event);
        }
        let mut out = Vec::new();
        reporter.flush_into(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn download_events(total: Option<u64>) -> Vec<crate::source::AcquisitionEvent> {
        use crate::source::AcquisitionEvent as E;
        vec![
            E::DownloadStarted {
                url: "https://***@example.com/fmt-11.1.1.zip".to_string(),
                total,
            },
            E::DownloadProgress {
                downloaded: 1024,
                total,
            },
            E::DownloadProgress {
                downloaded: 3 * 1024 * 1024,
                total,
            },
            E::DownloadFinished {
                bytes: 3 * 1024 * 1024,
            },
            E::ChecksumVerified {
                checksum: "sha256:abc".to_string(),
            },
            E::Extracted,
        ]
    }

    #[test]
    fn download_reporting_by_verbosity() {
        let normal = render_events(
            ui::Verbosity::Normal,
            &download_events(Some(3 * 1024 * 1024)),
        );
        assert_eq!(
            normal, "  Downloaded fmt v11.1.1 (3.0 MiB, checksum verified)\n",
            "default output: one line per download, no progress redraws"
        );
        let verbose = render_events(
            ui::Verbosity::Verbose,
            &download_events(Some(3 * 1024 * 1024)),
        );
        for expected in [
            " Downloading fmt v11.1.1 (https://***@example.com/fmt-11.1.1.zip, 3.0 MiB)\n",
            "  Downloaded fmt v11.1.1 (3.0 MiB, checksum verified)\n",
            "    Verified fmt v11.1.1 sha256:abc\n",
            "   Extracted fmt v11.1.1\n",
        ] {
            assert!(
                verbose.contains(expected),
                "missing {expected:?} in:\n{verbose}"
            );
        }
        assert!(render_events(ui::Verbosity::Quiet, &download_events(None)).is_empty());
    }

    #[test]
    fn unknown_download_size_is_not_invented() {
        let verbose = render_events(ui::Verbosity::Verbose, &download_events(None));
        assert!(
            verbose.contains("(https://***@example.com/fmt-11.1.1.zip)\n"),
            "{verbose}"
        );
        assert!(!verbose.contains('%'));
        assert!(
            !verbose.contains('\r'),
            "non-interactive output never redraws"
        );
    }

    #[test]
    fn cache_hits_are_reported_with_verbose() {
        use crate::source::AcquisitionEvent as E;
        assert!(render_events(ui::Verbosity::Normal, &[E::SourceCacheHit]).is_empty());
        assert_eq!(
            render_events(ui::Verbosity::Verbose, &[E::SourceCacheHit]),
            "      Cached fmt v11.1.1 (verified source cache)\n"
        );
        assert_eq!(
            render_events(ui::Verbosity::Verbose, &[E::ArchiveCacheHit, E::Extracted]),
            "      Cached fmt v11.1.1 (verified archive cache)\n   Extracted fmt v11.1.1\n"
        );
        let git = render_events(
            ui::Verbosity::Normal,
            &[
                E::GitCloning {
                    url: "https://example.com/fmt.git".to_string(),
                },
                E::GitCommit {
                    commit: "0123456789abcdef0123".to_string(),
                    cached: false,
                },
            ],
        );
        assert_eq!(git, "     Fetched fmt v11.1.1 (git commit 0123456789ab)\n");
    }
}
