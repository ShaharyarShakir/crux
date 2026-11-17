//! Implementation of the `crux run` command.

use crate::build::{AppBuildEngine, AppBuildRequest, BuildError, BuildProfile};
use crate::cli::CliError;
use crate::cli::args::RunArgs;
use crate::cli::commands::build::ensure_app_dependencies;
use crate::project::config::ProjectConfig;
use crate::project::manifest::{MANIFEST_FILENAME, ProjectLanguage};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Builds and runs the current Crux application project.
pub fn run_run<W: Write, E: Write>(
    project_dir: &Path,
    args: &RunArgs,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    let manifest_path = project_dir.join(MANIFEST_FILENAME);
    if !manifest_path.is_file() {
        let err = format!(
            "manifest not found at '{}': run 'crux init' to initialize a new project",
            manifest_path.display()
        );
        writeln!(stderr, "error: {err}")?;
        return Err(CliError::General(err));
    }

    let config = match ProjectConfig::load_from_dir(project_dir) {
        Ok(config) => config,
        Err(err) => {
            writeln!(stderr, "error: {err}")?;
            return Err(CliError::General(err.to_string()));
        }
    };

    if !config.is_project {
        let err = format!(
            "manifest in '{}' does not define a [project] section; 'crux run' requires a Crux application project",
            project_dir.display()
        );
        writeln!(stderr, "error: {err}")?;
        return Err(CliError::General(err));
    }

    if let Some(ProjectLanguage::Other) = config.language {
        let err = "projects with language 'other' are not supported by 'crux run'; only C and C++ applications can be executed".to_string();
        writeln!(stderr, "error: {err}")?;
        return Err(CliError::General(err));
    }

    let profile = args.profile.unwrap_or(BuildProfile::Debug);

    writeln!(stdout, "Crux Run\n")?;

    // Resolve, lock, and install declared dependencies before compiling.
    ensure_app_dependencies(project_dir, profile, stdout, stderr)?;

    let engine = AppBuildEngine::new();
    let request = AppBuildRequest::new(project_dir)
        .with_profile(profile)
        .with_force(false)
        .with_verbose(args.verbose);

    let model = match engine.prepare(&request) {
        Ok(model) => model,
        Err(err) => {
            writeln!(stderr, "error: {err}")?;
            return Err(CliError::Build(Box::new(err)));
        }
    };

    writeln!(stdout, "Project:       {}", model.project_name())?;
    writeln!(
        stdout,
        "Configuration: {}",
        model.profile.as_cmake_build_type()
    )?;
    writeln!(stdout, "Dependencies:  {}", model.dependency_count())?;
    writeln!(stdout, "\nChecking application...")?;
    writeln!(stdout, "Building...")?;

    let result = match engine.execute(&model, false, args.verbose) {
        Ok(result) => result,
        Err(err) => {
            if args.verbose
                && let Some(log_path) = error_log_path(&err)
            {
                echo_log(stderr, "CMake output", log_path)?;
            }
            writeln!(stderr, "\nBuild failed!\n{err}")?;
            return Err(CliError::Build(Box::new(err)));
        }
    };

    writeln!(stdout, "Build successful.\n")?;

    let executable_path = &result.executable_path;
    validate_executable(executable_path, &model.project_root).map_err(|msg| {
        writeln!(stderr, "error: {msg}").ok();
        CliError::General(msg)
    })?;

    let run_name = executable_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(model.project_name());
    writeln!(stdout, "Running {run_name}...\n")?;
    stdout.flush()?;
    stderr.flush()?;

    let runtime_directories = model.dependencies.runtime_directories();
    let status = execute_application(
        executable_path,
        &model.project_root,
        &args.app_args,
        &runtime_directories,
    )
    .map_err(|msg| {
        writeln!(stderr, "error: failed to launch application: {msg}").ok();
        CliError::General(format!("failed to launch application: {msg}"))
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(CliError::AppExit(exit_code_from_status(status)))
    }
}

fn error_log_path(error: &BuildError) -> Option<&Path> {
    match error {
        BuildError::ConfigureFailed { log_path, .. } | BuildError::BuildFailed { log_path, .. } => {
            Some(log_path)
        }
        _ => None,
    }
}

fn echo_log<W: Write>(writer: &mut W, label: &str, path: &Path) -> Result<(), CliError> {
    match std::fs::read(path) {
        Ok(content) => {
            writeln!(writer, "\n--- {label} ({}) ---", path.display())?;
            writer.write_all(&content)?;
            if !content.ends_with(b"\n") {
                writeln!(writer)?;
            }
            writeln!(writer, "--- end {label} ---")?;
            Ok(())
        }
        Err(error) => {
            writeln!(
                writer,
                "warning: failed to read {label} log '{}': {error}",
                path.display()
            )?;
            Ok(())
        }
    }
}

pub(crate) fn validate_executable(
    executable_path: &Path,
    project_root: &Path,
) -> Result<(), String> {
    if !executable_path.exists() {
        return Err(format!(
            "executable '{}' was not found after a successful build",
            executable_path.display()
        ));
    }
    if !executable_path.is_file() {
        return Err(format!(
            "executable path '{}' is not a regular file",
            executable_path.display()
        ));
    }

    let canonical_exe = executable_path.canonicalize().map_err(|e| {
        format!(
            "failed to resolve executable path '{}': {e}",
            executable_path.display()
        )
    })?;
    let canonical_root = project_root.canonicalize().map_err(|e| {
        format!(
            "failed to resolve project root '{}': {e}",
            project_root.display()
        )
    })?;
    let expected_build_root = canonical_root.join(".crux").join("build");
    if !canonical_exe.starts_with(&expected_build_root) {
        return Err(format!(
            "refusing to execute '{}' because it is outside the Crux build output '{}'",
            canonical_exe.display(),
            expected_build_root.display()
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(executable_path)
            .map_err(|e| {
                format!(
                    "failed to read executable metadata for '{}': {e}",
                    executable_path.display()
                )
            })?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(format!(
                "executable '{}' is not marked executable",
                executable_path.display()
            ));
        }
    }

    Ok(())
}

fn execute_application(
    executable_path: &Path,
    working_dir: &Path,
    app_args: &[String],
    runtime_directories: &[PathBuf],
) -> std::io::Result<std::process::ExitStatus> {
    let mut command = Command::new(executable_path);
    command.args(app_args);
    command.current_dir(working_dir);
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());

    if !runtime_directories.is_empty() {
        // Make shared dependency libraries resolvable even on platforms or
        // toolchains that do not apply the generated BUILD_RPATH.
        let variable = if cfg!(target_os = "macos") {
            "DYLD_LIBRARY_PATH"
        } else if cfg!(windows) {
            "PATH"
        } else {
            "LD_LIBRARY_PATH"
        };
        let mut paths = runtime_directories.to_vec();
        if let Some(existing) = std::env::var_os(variable) {
            paths.extend(std::env::split_paths(&existing));
        }
        let joined = std::env::join_paths(paths)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
        command.env(variable, OsString::from(joined));
    }

    command.status()
}

pub(crate) fn exit_code_from_status(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }

    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "crux_test_run_{prefix}_{nanos}_{}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("failed to create temporary test directory");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_validate_executable_nonexistent() {
        let temp = TestDir::new("nonexistent");
        let root = temp.path.canonicalize().unwrap();
        let exe = root.join(".crux/build/debug/missing_binary");
        let res = validate_executable(&exe, &root);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("was not found"));
    }

    #[test]
    fn test_validate_executable_directory_not_file() {
        let temp = TestDir::new("dir_not_file");
        let root = temp.path.canonicalize().unwrap();
        let build_dir = root.join(".crux").join("build");
        fs::create_dir_all(&build_dir).unwrap();
        let fake_exe = build_dir.join("not_a_file");
        fs::create_dir_all(&fake_exe).unwrap();

        let res = validate_executable(&fake_exe, &root);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("is not a regular file"));
    }

    #[test]
    fn test_validate_executable_outside_build_root() {
        let temp = TestDir::new("outside_build");
        let root = temp.path.canonicalize().unwrap();
        let outside_exe = root.join("external_bin");
        fs::write(&outside_exe, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&outside_exe, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let res = validate_executable(&outside_exe, &root);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("outside the Crux build output"));
    }

    #[test]
    fn test_validate_executable_valid() {
        let temp = TestDir::new("valid");
        let root = temp.path.canonicalize().unwrap();
        let build_dir = root.join(".crux").join("build").join("debug");
        fs::create_dir_all(&build_dir).unwrap();
        let exe = build_dir.join("app");
        fs::write(&exe, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let res = validate_executable(&exe, &root);
        assert!(res.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_executable_non_executable_permissions() {
        let temp = TestDir::new("non_exec_perms");
        let root = temp.path.canonicalize().unwrap();
        let build_dir = root.join(".crux").join("build").join("debug");
        fs::create_dir_all(&build_dir).unwrap();
        let exe = build_dir.join("app_no_exec");
        fs::write(&exe, "plain text").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o644)).unwrap();
        }

        let res = validate_executable(&exe, &root);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("is not marked executable"));
    }
}
