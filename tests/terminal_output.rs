//! End-to-end tests for the terminal presentation layer: color policy,
//! verbosity, structured build diagnostics, and piped output.
//!
//! The `crux` binary is run as a subprocess with piped streams, so stderr is
//! never a terminal. Compiler and CMake failures use the real toolchain on
//! deliberately broken inputs; no output depends on a terminal emulator.

use crux::build::{BuildEngine, BuildError, BuildRequest, CMakeBuildEngine};
use crux::lock::Checksum;
use crux::package::{PackageId, PackageName};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(1);
const ESC: char = '\x1b';

struct Sandbox {
    path: PathBuf,
}

impl Sandbox {
    fn new(prefix: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("crux_term_{prefix}_{}_{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn crux(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crux"));
        command
            .args(args)
            .current_dir(&self.path)
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env("TERM", "xterm-256color")
            .env("CRUX_HOME", self.path.join(".crux-home"))
            .env("CRUX_CONFIG_PATH", self.path.join(".crux-home/config.toml"));
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// `crux remove x` without a manifest fails deterministically with `error:`.
fn failing_command(sandbox: &Sandbox, extra: &[&str], env: &[(&str, &str)]) -> Output {
    let mut args = extra.to_vec();
    args.extend(["remove", "missing-package"]);
    let output = sandbox.crux(&args, env);
    assert!(!output.status.success());
    output
}

#[test]
fn piped_output_has_no_escape_sequences_by_default() {
    let sandbox = Sandbox::new("piped");
    let output = failing_command(&sandbox, &[], &[]);
    let err = stderr(&output);
    assert!(err.contains("error:"), "{err}");
    assert!(
        !err.contains(ESC),
        "auto mode must not color piped output: {err:?}"
    );
    assert!(!stdout(&output).contains(ESC));
}

#[test]
fn color_always_styles_error_labels_without_changing_text_or_status() {
    let sandbox = Sandbox::new("always");
    let plain = failing_command(&sandbox, &["--color", "never"], &[]);
    let colored = failing_command(&sandbox, &["--color", "always"], &[]);
    let colored_err = stderr(&colored);
    assert!(
        colored_err.contains("\x1b[1;31merror\x1b[0m:"),
        "{colored_err:?}"
    );
    assert_eq!(
        crux::ui::strip_ansi(&colored_err),
        stderr(&plain),
        "color only adds escape sequences"
    );
    assert_eq!(
        plain.status.code(),
        colored.status.code(),
        "styling never changes the exit status"
    );
    // `--color=always` and a trailing position work too.
    let trailing = sandbox.crux(&["remove", "missing-package", "--color=always"], &[]);
    assert!(stderr(&trailing).contains(ESC));
}

#[test]
fn environment_variables_and_their_precedence() {
    let sandbox = Sandbox::new("env");
    // CLICOLOR_FORCE enables color in auto mode even when piped.
    let forced = failing_command(&sandbox, &[], &[("CLICOLOR_FORCE", "1")]);
    assert!(stderr(&forced).contains(ESC));
    // NO_COLOR is respected in auto mode...
    let no_color = failing_command(&sandbox, &[], &[("NO_COLOR", "1")]);
    assert!(!stderr(&no_color).contains(ESC));
    // ...but an explicit --color always wins over NO_COLOR,
    let explicit = failing_command(&sandbox, &["--color", "always"], &[("NO_COLOR", "1")]);
    assert!(stderr(&explicit).contains(ESC));
    // and --color never wins over CLICOLOR_FORCE.
    let never = failing_command(&sandbox, &["--color", "never"], &[("CLICOLOR_FORCE", "1")]);
    assert!(!stderr(&never).contains(ESC));
}

#[test]
fn invalid_global_options_are_reported() {
    let sandbox = Sandbox::new("invalid");
    let bad = sandbox.crux(&["--color", "sometimes", "check"], &[]);
    assert!(!bad.status.success());
    assert!(
        stderr(&bad).contains("invalid value 'sometimes' for '--color'"),
        "{}",
        stderr(&bad)
    );
    let conflict = sandbox.crux(&["-q", "-v", "check"], &[]);
    assert!(!conflict.status.success());
    assert!(
        stderr(&conflict).contains("cannot be combined"),
        "{}",
        stderr(&conflict)
    );
    let missing = sandbox.crux(&["check", "--color"], &[]);
    assert!(!missing.status.success());
}

#[test]
fn global_option_extraction_stops_at_double_dash() {
    use crux::ui::{ColorChoice, Verbosity};
    let (globals, rest) = crux::cli::extract_global_options(&[
        "-v", "run", "--color", "never", "--", "-v", "--color", "app-arg",
    ])
    .unwrap();
    assert_eq!(globals.verbosity, Verbosity::Verbose);
    assert_eq!(globals.color, ColorChoice::Never);
    assert_eq!(rest, ["run", "--", "-v", "--color", "app-arg"]);
    let (vv, _) = crux::cli::extract_global_options(&["-vv", "build"]).unwrap();
    assert_eq!(vv.verbosity, Verbosity::VeryVerbose);
    let (q, _) = crux::cli::extract_global_options(&["build", "--quiet"]).unwrap();
    assert_eq!(q.verbosity, Verbosity::Quiet);
}

#[test]
fn help_documents_global_options() {
    let sandbox = Sandbox::new("help");
    let help = sandbox.crux(&["--help"], &[]);
    let out = stdout(&help);
    for option in ["--color <WHEN>", "-q, --quiet", "-v, --verbose", "-vv"] {
        assert!(out.contains(option), "missing {option} in help");
    }
}

fn adhoc_failure(sandbox: &Sandbox, extra: &[&str], env: &[(&str, &str)]) -> Output {
    fs::write(
        sandbox.path.join("broken.c"),
        "int main(void) {\n    return undeclared_value;\n}\n",
    )
    .unwrap();
    let mut args = extra.to_vec();
    args.extend(["run", "--bin", "broken.c"]);
    sandbox.crux(&args, env)
}

/// Compiler failure: Crux identifies the package and stage, shows the
/// command and exit status, and preserves the compiler's own diagnostics
/// (location and caret line) verbatim.
#[test]
fn compiler_failure_is_reported_with_context_and_original_output() {
    let sandbox = Sandbox::new("compile_fail");
    let output = adhoc_failure(&sandbox, &[], &[]);
    assert!(!output.status.success());
    let err = stderr(&output);
    for expected in [
        "   Compiling broken",
        "error: package compilation failed\n",
        "  package:",
        "  stage:       compilation and linking (cmake --build)\n",
        "  exit status:",
        "  build dir:",
        "  command:\n    ",
        "--build",
        "note: the original CMake and compiler output follows, unmodified",
        "--- original output (",
        "--- end of original output ---",
    ] {
        assert!(err.contains(expected), "missing {expected:?} in:\n{err}");
    }
    // The compiler's diagnostic: file:line:column and the caret marker.
    assert!(err.contains("broken.c:2:"), "{err}");
    assert!(err.contains("undeclared_value"), "{err}");
    assert!(err.contains('^'), "caret line preserved:\n{err}");
    assert!(!err.contains(ESC));
    assert!(
        !stdout(&output).contains("--- original output"),
        "diagnostics stay on stderr"
    );
}

#[test]
fn quiet_mode_keeps_failure_information_and_drops_progress() {
    let sandbox = Sandbox::new("quiet_fail");
    let output = adhoc_failure(&sandbox, &["-q"], &[]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(
        !err.contains("Compiling"),
        "routine status is suppressed:\n{err}"
    );
    assert!(err.contains("error: package compilation failed"), "{err}");
    assert!(err.contains("broken.c:2:"), "{err}");
}

#[test]
fn successful_build_reports_compiling_and_finished() {
    let sandbox = Sandbox::new("success");
    fs::write(sandbox.path.join("ok.c"), "int main(void) { return 0; }\n").unwrap();
    let output = sandbox.crux(&["run", "--bin", "ok.c"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("   Compiling ok"), "{err}");
    assert!(err.contains("    Finished `debug` profile in "), "{err}");
    assert!(err.contains("     Running "), "{err}");
    let quiet = sandbox.crux(&["--quiet", "run", "--bin", "ok.c"], &[]);
    assert!(quiet.status.success());
    assert!(!stderr(&quiet).contains("Compiling"));
    assert!(!stderr(&quiet).contains("Finished"));
}

#[test]
fn very_verbose_streams_tool_output_live() {
    let sandbox = Sandbox::new("vv");
    fs::write(sandbox.path.join("ok.c"), "int main(void) { return 0; }\n").unwrap();
    let output = sandbox.crux(&["-vv", "run", "--bin", "ok.c"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    // CMake's own configure output is streamed to stderr while it runs.
    assert!(
        stderr(&output).contains("-- Configuring done"),
        "{}",
        stderr(&output)
    );
}

fn checksum() -> Checksum {
    Checksum::new_sha256("0".repeat(64)).unwrap()
}

/// CMake configure failure: the structured error records the exact command
/// and build directory, and the log starts with the command line and keeps
/// CMake's original message.
#[test]
fn cmake_configure_failure_records_command_build_dir_and_log() {
    let sandbox = Sandbox::new("configure_fail");
    let source = sandbox.path.join("src");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.15)\nproject(broken C)\nnot_a_cmake_command(x)\n",
    )
    .unwrap();
    let engine = CMakeBuildEngine::new(sandbox.path.join("builds"));
    let id = PackageId::new(
        PackageName::new("broken").unwrap(),
        semver::Version::new(1, 0, 0),
    );
    let request = BuildRequest::new(id, source.clone(), checksum());
    assert!(!engine.is_fresh(&request));
    match engine.build(&request).unwrap_err() {
        BuildError::ConfigureFailed {
            exit_code,
            log_path,
            command,
            build_dir,
            ..
        } => {
            assert_ne!(exit_code, Some(0));
            assert!(
                command.contains("-S") && command.contains(&*source.to_string_lossy()),
                "{command}"
            );
            assert!(build_dir.ends_with("build"), "{}", build_dir.display());
            let log = fs::read_to_string(&log_path).unwrap();
            assert!(log.starts_with("$ "), "log header: {log}");
            assert!(
                log.contains("not_a_cmake_command"),
                "original CMake output kept"
            );
            assert!(log.contains("=== exit status:"), "{log}");

            let diagnostic = crux::cli::report::build_failure_diagnostic(
                &BuildError::ConfigureFailed {
                    exit_code,
                    log_path: log_path.clone(),
                    summary: String::new(),
                    command: command.clone(),
                    build_dir: build_dir.clone(),
                },
                "broken v1.0.0",
            )
            .unwrap();
            let text = diagnostic.render(&crux::ui::Style::plain());
            assert!(
                text.starts_with("error: package configuration failed\n"),
                "{text}"
            );
            assert!(
                text.contains("  stage:       CMake configuration\n"),
                "{text}"
            );
            assert!(text.contains(&command), "{text}");
        }
        other => panic!("expected ConfigureFailed, got {other:?}"),
    }
}

/// A successful package build is fresh on the next request, so the CLI can
/// report `Fresh` instead of `Compiling`.
#[test]
fn successful_package_build_is_reported_fresh_afterwards() {
    let sandbox = Sandbox::new("fresh");
    let source = sandbox.path.join("src");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.15)\nproject(tiny C)\nadd_library(tiny STATIC tiny.c)\n",
    )
    .unwrap();
    fs::write(source.join("tiny.c"), "int tiny(void) { return 1; }\n").unwrap();
    let engine = CMakeBuildEngine::new(sandbox.path.join("builds"));
    let id = PackageId::new(
        PackageName::new("tiny").unwrap(),
        semver::Version::new(1, 0, 0),
    );
    let request = BuildRequest::new(id, source, checksum());
    assert!(!engine.is_fresh(&request));
    let mut stderr_buf = Vec::new();
    let _ui = crux::ui::set_config(crux::ui::UiConfig::default());
    let result =
        crux::cli::report::build_package(&engine, request.clone(), &mut stderr_buf).unwrap();
    assert!(!result.reused_cache);
    assert_eq!(
        String::from_utf8_lossy(&stderr_buf),
        "   Compiling tiny v1.0.0\n"
    );
    assert!(engine.is_fresh(&request));
    let mut again = Vec::new();
    let _v = crux::ui::set_config(crux::ui::UiConfig {
        verbosity: crux::ui::Verbosity::Verbose,
        ..crux::ui::UiConfig::default()
    });
    crux::cli::report::build_package(&engine, request, &mut again).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&again),
        "       Fresh tiny v1.0.0\n"
    );
}

/// Cache-hit reporting through the real `crux fetch` command, fully offline:
/// a private registry declares an archive package whose (checksum-valid)
/// archive is pre-seeded in an isolated cache.
#[test]
fn fetch_reports_cache_hits_by_verbosity_without_redraws() {
    use sha2::{Digest, Sha256};
    use std::io::Write as _;

    let sandbox = Sandbox::new("fetch_cache");
    // A small zip archive with a CMake project inside a top-level directory.
    let mut zip_bytes = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut zip_bytes);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("tiny-1.0.0/CMakeLists.txt", options)
            .unwrap();
        zip.write_all(b"project(tiny C)\n").unwrap();
        zip.finish().unwrap();
    }
    let zip_bytes = zip_bytes.into_inner();
    let digest = format!("{:x}", Sha256::digest(&zip_bytes));

    let registry = sandbox.path.join("registry");
    let package_dir = registry.join("packages/tiny/1.0.0");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        registry.join("registry.toml"),
        "format_version = 1\nname = \"default\"\ndescription = \"fixture\"\n",
    )
    .unwrap();
    fs::write(
        package_dir.join("package.toml"),
        format!(
            "[package]\nname = \"tiny\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\n\n\
             [source]\ntype = \"archive\"\nurl = \"https://example.invalid/tiny-1.0.0.zip\"\n\
             checksum = \"sha256:{digest}\"\n\n[build]\nsystem = \"cmake\"\n\n[dependencies]\n"
        ),
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&registry)
            .status()
            .unwrap()
            .success()
    );

    let cache_dir = sandbox.path.join("cache");
    let archive = cache_dir.join("archives/sha256").join(&digest);
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    fs::write(&archive, &zip_bytes).unwrap();
    fs::create_dir_all(sandbox.path.join(".crux-home")).unwrap();
    fs::write(
        sandbox.path.join(".crux-home/config.toml"),
        format!("cache_dir = \"{}\"\n", cache_dir.display()),
    )
    .unwrap();
    fs::write(
        sandbox.path.join("crux.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ntiny = \"^1.0\"\n",
    )
    .unwrap();
    fs::write(
        sandbox.path.join("crux.lock"),
        format!(
            "lockfile_version = 1\n\n[[package]]\nname = \"tiny\"\nversion = \"1.0.0\"\n\
             source = \"registry+default\"\nchecksum = \"sha256:{digest}\"\ndependencies = []\n"
        ),
    )
    .unwrap();
    let env = [("CRUX_REGISTRY_PATH", registry.to_str().unwrap())];

    // First fetch: archive cache hit, then extraction (both -v details).
    let first = sandbox.crux(&["-v", "fetch"], &env);
    assert!(first.status.success(), "{}", stderr(&first));
    let err = stderr(&first);
    assert!(
        err.contains("      Cached tiny v1.0.0 (verified archive cache)\n"),
        "{err}"
    );
    assert!(err.contains("   Extracted tiny v1.0.0\n"), "{err}");
    assert!(
        !err.contains('\r'),
        "piped output never contains progress redraws"
    );
    assert!(!err.contains(ESC));

    // Second fetch: the extracted source tree itself is reused.
    let second = sandbox.crux(&["-v", "fetch"], &env);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(
        stderr(&second).contains("      Cached tiny v1.0.0 (verified source cache)\n"),
        "{}",
        stderr(&second)
    );

    // Default verbosity hides cache hits; quiet prints nothing to stderr.
    let default = sandbox.crux(&["fetch"], &env);
    assert!(default.status.success());
    assert!(!stderr(&default).contains("Cached"), "{}", stderr(&default));
    let quiet = sandbox.crux(&["-q", "fetch"], &env);
    assert!(quiet.status.success());
    assert!(
        stderr(&quiet).trim().is_empty(),
        "quiet:\n{}",
        stderr(&quiet)
    );
    // stdout (the command's result) is unaffected by verbosity.
    assert_eq!(stdout(&default), stdout(&quiet));
}
