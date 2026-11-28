//! The CMake adapter for package builds: tool discovery, configure, build and
//! install commands, and what a CMake build tree says about itself.
//!
//! Application projects are built by [`crate::build::app`], not by this adapter.

use super::{BuildStep, StepExitCodes};
use crate::build::metadata::{BuildConfiguration, SelectedCompiler, SelectedToolchain};
use crate::build::selection::{
    AmbientEnv, CompilerDeclarations, ToolchainSelection, same_compiler,
};
use crate::build::toolchain::ToolchainInfo;
use crate::build::{BuildError, BuildRequest, BuildResult};
use crate::install::{InstallError, promote_unwrapped_prefix};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const C_COMPILER: &str = "CMAKE_C_COMPILER";
const CXX_COMPILER: &str = "CMAKE_CXX_COMPILER";
const TOOLCHAIN_FILE: &str = "CMAKE_TOOLCHAIN_FILE";

/// The CMake executable and the compiler overrides given to an engine.
#[derive(Debug, Clone)]
pub(crate) struct CMake {
    pub bin: PathBuf,
    pub c_compiler: Option<PathBuf>,
    pub cxx_compiler: Option<PathBuf>,
}

impl Default for CMake {
    fn default() -> Self {
        Self {
            bin: PathBuf::from("cmake"),
            c_compiler: None,
            cxx_compiler: None,
        }
    }
}

impl CMake {
    pub(super) fn check_source(&self, source_dir: &Path) -> Result<(), BuildError> {
        if source_dir.join("CMakeLists.txt").is_file() {
            Ok(())
        } else {
            Err(BuildError::MissingCMakeLists(source_dir.to_path_buf()))
        }
    }

    pub(crate) fn detect(
        &self,
        config: &BuildConfiguration,
        ambient: Option<&AmbientEnv>,
    ) -> Result<(ToolchainInfo, ToolchainSelection), BuildError> {
        // 1. Detect CMake version
        let cmake_output = Command::new(&self.bin)
            .arg("--version")
            .output()
            .map_err(|e| {
                BuildError::MissingCMake(format!(
                    "failed to execute '{} --version': {e}",
                    self.bin.display()
                ))
            })?;

        if !cmake_output.status.success() {
            return Err(BuildError::MissingCMake(format!(
                "cmake --version failed with exit status {}",
                cmake_output.status
            )));
        }

        let cmake_stdout = String::from_utf8_lossy(&cmake_output.stdout);
        let cmake_version = cmake_stdout
            .lines()
            .next()
            .unwrap_or("cmake version unknown")
            .trim()
            .to_string();

        // 2. Detect generator
        let generator = if let Some(generator_name) = &config.generator {
            generator_name.clone()
        } else {
            // Default to "Unix Makefiles" or "Ninja" if available
            if has_command("ninja") {
                "Ninja".to_string()
            } else {
                "Unix Makefiles".to_string()
            }
        };

        // 3. Resolve the C and C++ compilers and ambient flags
        let definition =
            |name: &'static str| definition(&config.cmake_args, name).map(|value| (name, value));
        let selection = ToolchainSelection::resolve(
            config,
            self.c_compiler.as_deref(),
            self.cxx_compiler.as_deref(),
            ambient,
            &CompilerDeclarations {
                c: definition(C_COMPILER),
                cxx: definition(CXX_COMPILER),
                toolchain_file_variable: TOOLCHAIN_FILE,
                declares_toolchain_file: definition(TOOLCHAIN_FILE).is_some(),
            },
        )?;

        let toolchain = ToolchainInfo {
            cmake_version,
            c_compiler_version: selection.c.version.clone(),
            cxx_compiler_version: selection.cxx.version.clone(),
            generator,
        };
        Ok((toolchain, selection))
    }

    pub(super) fn is_configured(&self, build_dir: &Path) -> bool {
        build_dir.join("CMakeCache.txt").is_file()
    }

    /// Builds that recorded their selection are compared with it. Older ones
    /// recorded only what Crux assumed, so their CMake tree is inspected.
    pub(super) fn conflicts_with(
        &self,
        selection: &ToolchainSelection,
        recorded: Option<&SelectedToolchain>,
        build_dir: &Path,
        config: &BuildConfiguration,
    ) -> bool {
        if let Some(recorded) = recorded {
            return selection.differs_from(recorded);
        }
        let current = selection.record();
        let tree = CMakeTree::read(build_dir);
        let compiler_differs = [
            (C_COMPILER, current.c_compiler),
            (CXX_COMPILER, current.cxx_compiler),
        ]
        .into_iter()
        .any(|(variable, new)| match (tree.compiler(variable), new) {
            (Some(old), Some(new)) => !same_compiler(&old, &new),
            _ => false,
        });
        compiler_differs
            || (!selection.uses_toolchain_file() && tree_has_stray_flags(selection, &tree, config))
    }

    /// Whether CMake would have to replace the compiler cached in `build_dir`,
    /// which it cannot do reliably in place.
    pub(super) fn tree_needs_reset(
        &self,
        selection: &ToolchainSelection,
        build_dir: &Path,
    ) -> bool {
        let tree = CMakeTree::read(build_dir);
        [(C_COMPILER, &selection.c), (CXX_COMPILER, &selection.cxx)]
            .into_iter()
            .any(|(variable, choice)| {
                let new = choice.compiler.as_ref().filter(|_| choice.pass);
                match (tree.compiler(variable), new) {
                    (Some(old), Some(new)) => old != *new,
                    _ => false,
                }
            })
    }

    pub(super) fn configure_and_build(
        &self,
        step: &BuildStep<'_>,
    ) -> Result<StepExitCodes, BuildError> {
        let &BuildStep {
            request,
            toolchain,
            selection,
            dependencies,
            build_dir,
            configure_log_path,
            build_log_path,
        } = step;

        // Configure
        let mut configure_cmd = Command::new(&self.bin);
        configure_cmd.arg("-S").arg(&request.source_dir);
        configure_cmd.arg("-B").arg(build_dir);
        configure_cmd.arg(format!(
            "-DCMAKE_BUILD_TYPE={}",
            request.configuration.profile.as_cmake_build_type()
        ));
        configure_cmd.arg("-G").arg(&toolchain.generator);

        if let Some(shared) = request.configuration.build_shared_libs {
            configure_cmd.arg(format!(
                "-DBUILD_SHARED_LIBS={}",
                if shared { "ON" } else { "OFF" }
            ));
        }

        configure_cmd.args(compiler_arguments(selection));

        // Add custom CMake options
        for arg in &request.configuration.cmake_args {
            configure_cmd.arg(arg);
        }
        if let Some(arg) = module_path_argument(
            dependencies.module_dirs(),
            &request.configuration.cmake_args,
        )? {
            configure_cmd.arg(arg);
        }

        // The fingerprinted flags, then custom environment variables
        selection.apply_environment(&mut configure_cmd);
        for (k, v) in &request.configuration.extra_env {
            configure_cmd.env(k, v);
        }
        dependencies.apply_environment(&mut configure_cmd, selection)?;

        let configure_command = crate::core::process::describe_command(&configure_cmd);
        let configure_output = run_step(
            &mut configure_cmd,
            configure_log_path,
            &configure_command,
            request,
        )
        .map_err(|e| {
            BuildError::MissingCMake(format!(
                "failed to launch CMake configure at '{}': {e}",
                self.bin.display()
            ))
        })?;

        if !configure_output.status.success() {
            return Err(BuildError::ConfigureFailed {
                exit_code: configure_output.status.code(),
                log_path: configure_log_path.to_path_buf(),
                summary: extract_log_tail(&failure_text(&configure_output), 20),
                command: configure_command,
                build_dir: build_dir.to_path_buf(),
            });
        }

        // Build
        let mut build_cmd = Command::new(&self.bin);
        // CMake may re-run configuration from the build step.
        selection.apply_environment(&mut build_cmd);
        dependencies.apply_environment(&mut build_cmd, selection)?;
        build_cmd.arg("--build").arg(build_dir);
        build_cmd
            .arg("--config")
            .arg(request.configuration.profile.as_cmake_build_type());

        if let Some(jobs) = request.configuration.jobs {
            build_cmd.arg("--parallel").arg(jobs.to_string());
        }

        let build_command = crate::core::process::describe_command(&build_cmd);
        let build_output = run_step(&mut build_cmd, build_log_path, &build_command, request)
            .map_err(|e| {
                BuildError::MissingCMake(format!(
                    "failed to launch CMake build at '{}': {e}",
                    self.bin.display()
                ))
            })?;

        if !build_output.status.success() {
            return Err(BuildError::BuildFailed {
                exit_code: build_output.status.code(),
                log_path: build_log_path.to_path_buf(),
                summary: extract_log_tail(&failure_text(&build_output), 20),
                command: build_command,
                build_dir: build_dir.to_path_buf(),
            });
        }

        Ok(StepExitCodes {
            configure: configure_output.status.code().unwrap_or(0),
            build: build_output.status.code().unwrap_or(0),
        })
    }

    /// CMake internals, generator files and intermediate objects.
    pub(super) fn is_build_internal(&self, file_name: &str) -> bool {
        file_name == "CMakeFiles"
            || file_name == "CMakeCache.txt"
            || file_name == "cmake_install.cmake"
            || file_name == "Makefile"
            || file_name.ends_with(".ninja")
            || file_name == ".ninja_deps"
            || file_name == ".ninja_log"
            || file_name.ends_with(".o")
            || file_name.ends_with(".obj")
            || file_name.ends_with(".d")
    }

    pub(super) fn check_installable(&self, build_dir: &Path) -> Result<(), InstallError> {
        if self.is_configured(build_dir) || build_dir.join("cmake_install.cmake").is_file() {
            Ok(())
        } else {
            Err(InstallError::MissingCMakeConfig(build_dir.to_path_buf()))
        }
    }

    /// Runs `cmake --install`, logging to the build's `install.log`.
    pub(super) fn install_into(
        &self,
        build: &BuildResult,
        staging_dir: &Path,
    ) -> Result<(), InstallError> {
        // Check if cmake_install.cmake contains absolute destination targets (like zlib's /usr/local/lib)
        let cmake_install_script = build.build_dir.join("cmake_install.cmake");
        let has_absolute_dest = if let Ok(content) = fs::read_to_string(&cmake_install_script) {
            content.contains("CMAKE_ABSOLUTE_DESTINATION_FILES")
        } else {
            false
        };

        // Run `cmake --install <build_dir> ...`
        let mut cmd = Command::new(&self.bin);
        cmd.arg("--install").arg(&build.build_dir);
        if has_absolute_dest {
            cmd.arg("--prefix").arg("/");
            cmd.env("DESTDIR", staging_dir);
        } else {
            cmd.arg("--prefix").arg(staging_dir);
        }
        cmd.arg("--config").arg(build.profile.as_cmake_build_type());

        let output = cmd.output().map_err(InstallError::Io)?;

        // Write install log to the build's logs directory
        let install_log_path = build.logs_dir.join("install.log");
        if let Ok(mut log_file) = fs::File::create(&install_log_path) {
            let prefix_display = if has_absolute_dest {
                "/".to_string()
            } else {
                staging_dir.display().to_string()
            };
            let _ = writeln!(log_file, "=== CMake Install Command ===");
            let _ = writeln!(
                log_file,
                "cmake --install {} --prefix {} --config {} (absolute_dest: {has_absolute_dest})",
                build.build_dir.display(),
                prefix_display,
                build.profile.as_cmake_build_type()
            );
            let _ = writeln!(log_file, "\n=== STDOUT ===");
            let _ = log_file.write_all(&output.stdout);
            let _ = writeln!(log_file, "\n=== STDERR ===");
            let _ = log_file.write_all(&output.stderr);
        }

        if !output.status.success() {
            let summary = String::from_utf8_lossy(&output.stderr);
            let summary_tail = summary.lines().rev().take(10).collect::<Vec<_>>();
            let mut reversed = summary_tail;
            reversed.reverse();
            let summary_str = if reversed.is_empty() {
                let stdout_summary = String::from_utf8_lossy(&output.stdout);
                let stdout_tail = stdout_summary.lines().rev().take(10).collect::<Vec<_>>();
                let mut rev_stdout = stdout_tail;
                rev_stdout.reverse();
                rev_stdout.join("\n")
            } else {
                reversed.join("\n")
            };

            return Err(InstallError::CMakeExecutionFailed {
                exit_code: output.status.code(),
                log_path: install_log_path,
                summary: summary_str,
            });
        }

        // If absolute destinations were used, promote usr/local or usr contents up to staging_dir
        if has_absolute_dest {
            promote_unwrapped_prefix(staging_dir)?;
        }
        Ok(())
    }
}

fn has_command(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Runs one CMake step, streaming its complete output into `log_path`
/// (and to the request's live observer) while keeping only a bounded
/// tail in memory. Package builds have no time limit, as before.
fn run_step(
    command: &mut Command,
    log_path: &Path,
    display: &str,
    request: &BuildRequest,
) -> Result<crate::core::process::ProcessOutput, io::Error> {
    crate::core::process::run_logged(
        command,
        Duration::ZERO,
        crate::core::process::LoggedRun {
            log_path,
            header: format!("$ {display}\n\n"),
            tail_limit: crate::core::process::DEFAULT_TAIL_LIMIT,
            tee: request.output_observer.as_ref().map(|o| o.0.clone()),
        },
    )
}

/// The text summarized on failure: stderr when it has content, else stdout.
fn failure_text(output: &crate::core::process::ProcessOutput) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        stderr.into_owned()
    }
}

fn extract_log_tail(log: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = log.lines().collect();
    if lines.len() <= max_lines {
        log.to_string()
    } else {
        let tail = &lines[lines.len() - max_lines..];
        format!("[...]\n{}", tail.join("\n"))
    }
}

/// The `-D` arguments that make CMake use the fingerprinted compilers.
fn compiler_arguments(selection: &ToolchainSelection) -> Vec<OsString> {
    [(C_COMPILER, &selection.c), (CXX_COMPILER, &selection.cxx)]
        .into_iter()
        .filter(|(_, choice)| choice.pass)
        .filter_map(|(variable, choice)| {
            let compiler = choice.compiler.as_ref()?;
            let mut arg = OsString::from(format!("-D{variable}="));
            arg.push(&compiler.path);
            // CMake reads a list as the compiler plus its fixed arguments.
            for fixed in &compiler.args {
                arg.push(";");
                arg.push(fixed);
            }
            Some(arg)
        })
        .collect()
}

/// `-DCMAKE_MODULE_PATH=...`, when a dependency installs CMake modules.
/// CMake has no environment variable for it. A value the package's CMake
/// arguments define is kept first, so this must follow those arguments.
fn module_path_argument<'a>(
    module_dirs: impl IntoIterator<Item = &'a Path>,
    cmake_args: &'a [String],
) -> Result<Option<String>, BuildError> {
    let mut dirs = Vec::new();
    for dir in module_dirs {
        match dir.to_str() {
            Some(text) if !text.contains(';') => dirs.push(text),
            _ => {
                return Err(BuildError::InvalidConfiguration(format!(
                    "cannot add '{}' to CMAKE_MODULE_PATH: not representable in a CMake list",
                    dir.display()
                )));
            }
        }
    }
    if dirs.is_empty() {
        return Ok(None);
    }
    if let Some(existing) = definition(cmake_args, "CMAKE_MODULE_PATH").filter(|v| !v.is_empty()) {
        dirs.insert(0, existing);
    }
    Ok(Some(format!("-DCMAKE_MODULE_PATH={}", dirs.join(";"))))
}

/// The value of the last `-D<name>[:TYPE]=value` among the CMake arguments.
fn definition<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().rev().find_map(|arg| {
        let rest = arg.strip_prefix("-D")?.trim_start().strip_prefix(name)?;
        let (kind, value) = rest.split_once('=')?;
        (kind.is_empty() || kind.starts_with(':')).then_some(value)
    })
}

/// Flags in an unrecorded tree that nothing in this request explains.
fn tree_has_stray_flags(
    selection: &ToolchainSelection,
    tree: &CMakeTree,
    config: &BuildConfiguration,
) -> bool {
    [
        ("CFLAGS", "CMAKE_C_FLAGS"),
        ("CXXFLAGS", "CMAKE_CXX_FLAGS"),
        ("LDFLAGS", "CMAKE_EXE_LINKER_FLAGS"),
        ("LDFLAGS", "CMAKE_SHARED_LINKER_FLAGS"),
        ("LDFLAGS", "CMAKE_MODULE_LINKER_FLAGS"),
    ]
    .into_iter()
    .any(|(env_var, cmake_var)| {
        selection.flag_is_unset(env_var)
            && definition(&config.cmake_args, cmake_var).is_none()
            && tree.cache_value(cmake_var).is_some_and(|v| !v.is_empty())
    })
}

/// The parts of a configured CMake build tree that show what it used.
struct CMakeTree {
    cache: String,
    compiler_files: String,
}

impl CMakeTree {
    fn read(build_dir: &Path) -> Self {
        let cache = fs::read_to_string(build_dir.join("CMakeCache.txt")).unwrap_or_default();
        let mut compiler_files = String::new();
        if let Ok(entries) = fs::read_dir(build_dir.join("CMakeFiles")) {
            for entry in entries.flatten() {
                for name in ["CMakeCCompiler.cmake", "CMakeCXXCompiler.cmake"] {
                    if let Ok(text) = fs::read_to_string(entry.path().join(name)) {
                        compiler_files.push_str(&text);
                    }
                }
            }
        }
        Self {
            cache,
            compiler_files,
        }
    }

    fn cache_value(&self, name: &str) -> Option<&str> {
        self.cache.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            rest.split_once('=').map(|(_, value)| value)
        })
    }

    fn compiler(&self, cmake_var: &str) -> Option<SelectedCompiler> {
        let path = self.cache_value(cmake_var).filter(|v| !v.is_empty())?;
        let arg_var = format!("{cmake_var}_ARG1");
        let marker = format!("set({arg_var} \"");
        let args = self
            .compiler_files
            .lines()
            .find_map(|line| line.strip_prefix(&marker)?.strip_suffix("\")"))
            .or_else(|| self.cache_value(&arg_var))
            .unwrap_or("");
        Some(SelectedCompiler {
            path: PathBuf::from(path),
            args: args.split_whitespace().map(str::to_string).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_module_directories_follow_the_declared_module_path() {
        let dirs = [
            Path::new("/p/lib/cmake/x"),
            Path::new("/p/share/cmake/Modules"),
        ];
        let args = ["-DCMAKE_MODULE_PATH=/mine".to_string()];
        assert_eq!(
            module_path_argument(dirs, &args).unwrap().as_deref(),
            Some("-DCMAKE_MODULE_PATH=/mine;/p/lib/cmake/x;/p/share/cmake/Modules")
        );
        assert_eq!(module_path_argument([], &[]).unwrap(), None);
        assert!(matches!(
            module_path_argument([Path::new("/a;b")], &[]),
            Err(BuildError::InvalidConfiguration(_))
        ));
    }
}
