//! CMake build engine, subprocess orchestration, log capture, and artifact collection.

use super::BuildError;
use super::fingerprint::BuildFingerprint;
use super::metadata::{
    ArtifactType, BuildArtifact, BuildConfiguration, BuildMetadata, BuildProfile,
    CURRENT_BUILD_METADATA_VERSION,
};
use super::toolchain::ToolchainInfo;
use crate::lock::Checksum;
use crate::package::PackageId;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Request to build a package.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub package_id: PackageId,
    pub source_dir: PathBuf,
    pub source_checksum: Checksum,
    pub configuration: BuildConfiguration,
    pub force: bool,
    /// Optional live observer of CMake/compiler output (used for `-vv`).
    pub output_observer: Option<crate::core::process::OutputObserver>,
}

impl BuildRequest {
    /// Creates a new build request for a package with default configuration.
    pub fn new(package_id: PackageId, source_dir: PathBuf, source_checksum: Checksum) -> Self {
        Self {
            package_id,
            source_dir,
            source_checksum,
            configuration: BuildConfiguration::default(),
            force: false,
            output_observer: None,
        }
    }

    /// Streams CMake and compiler output to `observer` while it is produced.
    pub fn with_output_observer(mut self, observer: crate::core::process::OutputObserver) -> Self {
        self.output_observer = Some(observer);
        self
    }

    /// Sets the build configuration.
    pub fn with_configuration(mut self, config: BuildConfiguration) -> Self {
        self.configuration = config;
        self
    }

    /// Enables forced rebuild, bypassing any existing build cache.
    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

/// Result of a completed or cache-hit package build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResult {
    pub package_id: PackageId,
    pub fingerprint: BuildFingerprint,
    pub source_dir: PathBuf,
    pub build_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub configure_log_path: PathBuf,
    pub build_log_path: PathBuf,
    pub metadata_path: PathBuf,
    pub artifacts: Vec<BuildArtifact>,
    pub profile: BuildProfile,
    pub duration: Duration,
    pub reused_cache: bool,
}

/// Abstract interface for package build engines.
pub trait BuildEngine {
    fn build(&self, request: &BuildRequest) -> Result<BuildResult, BuildError>;
}

/// CMake-backed build engine implementing out-of-source isolated compilation.
pub struct CMakeBuildEngine {
    builds_root: PathBuf,
    cmake_bin: PathBuf,
    c_compiler_bin: Option<PathBuf>,
    cxx_compiler_bin: Option<PathBuf>,
}

impl CMakeBuildEngine {
    /// Creates a new CMake build engine managing builds under `builds_root`.
    pub fn new(builds_root: impl Into<PathBuf>) -> Self {
        Self {
            builds_root: builds_root.into(),
            cmake_bin: PathBuf::from("cmake"),
            c_compiler_bin: None,
            cxx_compiler_bin: None,
        }
    }

    /// Overrides the path to the CMake binary.
    pub fn with_cmake_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.cmake_bin = bin.into();
        self
    }

    /// Overrides the default C compiler binary.
    pub fn with_c_compiler(mut self, bin: impl Into<PathBuf>) -> Self {
        self.c_compiler_bin = Some(bin.into());
        self
    }

    /// Overrides the default C++ compiler binary.
    pub fn with_cxx_compiler(mut self, bin: impl Into<PathBuf>) -> Self {
        self.cxx_compiler_bin = Some(bin.into());
        self
    }

    /// Returns the root directory where all builds are managed.
    pub fn builds_root(&self) -> &Path {
        &self.builds_root
    }

    /// Detects toolchain information by executing detection commands.
    pub fn detect_toolchain(
        &self,
        config: &BuildConfiguration,
    ) -> Result<ToolchainInfo, BuildError> {
        // 1. Detect CMake version
        let cmake_output = Command::new(&self.cmake_bin)
            .arg("--version")
            .output()
            .map_err(|e| {
                BuildError::MissingCMake(format!(
                    "failed to execute '{} --version': {e}",
                    self.cmake_bin.display()
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
            if Self::has_command("ninja") {
                "Ninja".to_string()
            } else {
                "Unix Makefiles".to_string()
            }
        };

        // 3. Detect C compiler version
        let c_compiler = config
            .c_compiler
            .as_ref()
            .or(self.c_compiler_bin.as_ref())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("gcc"));
        let c_compiler_version = Self::query_compiler_version(&c_compiler);

        // 4. Detect C++ compiler version
        let cxx_compiler = config
            .cxx_compiler
            .as_ref()
            .or(self.cxx_compiler_bin.as_ref())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("g++"));
        let cxx_compiler_version = Self::query_compiler_version(&cxx_compiler);

        Ok(ToolchainInfo {
            cmake_version,
            c_compiler_version,
            cxx_compiler_version,
            generator,
        })
    }

    fn has_command(cmd: &str) -> bool {
        Command::new(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn query_compiler_version(compiler_bin: &Path) -> Option<String> {
        let output = Command::new(compiler_bin).arg("--version").output().ok()?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.lines().next().map(|l| l.trim().to_string())
        } else {
            None
        }
    }

    /// Extracts the tail (last N lines) of a log string to display in diagnostic messages.
    /// Returns the cached result for `request` when a previous successful
    /// build with the same fingerprint is still complete on disk.
    fn cached_result(
        request: &BuildRequest,
        fingerprint: &BuildFingerprint,
        pkg_build_root: &Path,
    ) -> Option<BuildResult> {
        let fingerprint = fingerprint.clone();
        let build_dir = pkg_build_root.join("build");
        let logs_dir = pkg_build_root.join("logs");
        let configure_log_path = logs_dir.join("configure.log");
        let build_log_path = logs_dir.join("build.log");
        let metadata_path = pkg_build_root.join("metadata.toml");
        if !request.force
            && metadata_path.is_file()
            && build_dir.is_dir()
            && let Ok(meta_str) = fs::read_to_string(&metadata_path)
            && let Ok(metadata) = toml::from_str::<BuildMetadata>(&meta_str)
            && metadata.success
            && metadata.fingerprint == fingerprint
        {
            // Verify all recorded artifacts still exist on disk and match recorded file sizes
            let all_artifacts_valid = !metadata.artifacts.is_empty()
                && metadata.artifacts.iter().all(|a| {
                    if let Ok(meta) = fs::metadata(&a.path) {
                        meta.is_file() && meta.len() == a.size_bytes
                    } else {
                        false
                    }
                });

            if all_artifacts_valid {
                return Some(BuildResult {
                    package_id: request.package_id.clone(),
                    fingerprint,
                    source_dir: request.source_dir.clone(),
                    build_dir,
                    logs_dir,
                    configure_log_path,
                    build_log_path,
                    metadata_path,
                    artifacts: metadata.artifacts,
                    profile: request.configuration.profile,
                    duration: Duration::from_millis(metadata.duration_millis),
                    reused_cache: true,
                });
            }
        }
        None
    }

    /// Returns `true` when building `request` would reuse a cached result,
    /// so callers can report `Fresh` instead of `Compiling` up front. Runs
    /// the same toolchain detection and fingerprinting as [`BuildEngine::build`].
    pub fn is_fresh(&self, request: &BuildRequest) -> bool {
        if request.force
            || !request.source_dir.is_dir()
            || !request.source_dir.join("CMakeLists.txt").is_file()
        {
            return false;
        }
        let Ok(toolchain) = self.detect_toolchain(&request.configuration) else {
            return false;
        };
        let fingerprint = BuildFingerprint::compute(
            &request.package_id,
            &request.source_checksum,
            &toolchain,
            &request.configuration,
        );
        let pkg_build_root = self
            .builds_root
            .join(request.package_id.name().as_str())
            .join(request.package_id.version().to_string())
            .join(fingerprint.hex_digest());
        Self::cached_result(request, &fingerprint, &pkg_build_root).is_some()
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

    /// Writes content to a file atomically by writing to a temporary file and renaming.
    fn write_atomic(dest_path: &Path, content: &[u8]) -> Result<(), io::Error> {
        crate::core::filesystem::write_atomically(dest_path, content)
    }

    /// Scans a build directory and collects all produced artifacts.
    fn scan_artifacts(build_dir: &Path) -> Result<Vec<BuildArtifact>, BuildError> {
        let mut artifacts = Vec::new();
        if !build_dir.is_dir() {
            return Ok(artifacts);
        }

        Self::scan_dir_recursive(build_dir, build_dir, &mut artifacts)?;
        artifacts.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(artifacts)
    }

    fn scan_dir_recursive(
        current_dir: &Path,
        build_root: &Path,
        artifacts: &mut Vec<BuildArtifact>,
    ) -> Result<(), BuildError> {
        let entries = match fs::read_dir(current_dir) {
            Ok(e) => e,
            Err(err) => return Err(BuildError::Io(err)),
        };

        for entry in entries {
            let entry = entry.map_err(BuildError::Io)?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Ignore CMake internals
            if file_name == "CMakeFiles"
                || file_name == "CMakeCache.txt"
                || file_name == "cmake_install.cmake"
                || file_name == "Makefile"
                || file_name.ends_with(".ninja")
                || file_name == ".ninja_deps"
                || file_name == ".ninja_log"
                || file_name.ends_with(".o")
                || file_name.ends_with(".obj")
                || file_name.ends_with(".d")
            {
                continue;
            }

            let file_type = entry.file_type().map_err(BuildError::Io)?;
            if file_type.is_dir() {
                Self::scan_dir_recursive(&path, build_root, artifacts)?;
            } else if file_type.is_file() {
                let artifact_type = ArtifactType::from_path(&path);
                // Include static/shared libraries, executables, headers, or any other relevant files
                if artifact_type != ArtifactType::Other || file_name.ends_with(".pc") {
                    let size_bytes = entry.metadata().map_err(BuildError::Io)?.len();
                    let relative_path =
                        path.strip_prefix(build_root).unwrap_or(&path).to_path_buf();

                    artifacts.push(BuildArtifact {
                        name: file_name,
                        relative_path,
                        path,
                        artifact_type,
                        size_bytes,
                    });
                }
            }
        }

        Ok(())
    }
}

impl BuildEngine for CMakeBuildEngine {
    fn build(&self, request: &BuildRequest) -> Result<BuildResult, BuildError> {
        let start_time = Instant::now();

        // 1. Validate source directory
        if !request.source_dir.is_dir() {
            return Err(BuildError::MissingSource(request.source_dir.clone()));
        }

        let cmakelists = request.source_dir.join("CMakeLists.txt");
        if !cmakelists.is_file() {
            return Err(BuildError::MissingCMakeLists(request.source_dir.clone()));
        }

        // 2. Detect Toolchain and compute BuildFingerprint
        let toolchain = self.detect_toolchain(&request.configuration)?;
        let fingerprint = BuildFingerprint::compute(
            &request.package_id,
            &request.source_checksum,
            &toolchain,
            &request.configuration,
        );

        // 3. Setup isolated directory structure:
        //    <builds_root>/<pkg_name>/<pkg_version>/<fingerprint_hex>/
        let pkg_build_root = self
            .builds_root
            .join(request.package_id.name().as_str())
            .join(request.package_id.version().to_string())
            .join(fingerprint.hex_digest());

        let build_dir = pkg_build_root.join("build");
        let logs_dir = pkg_build_root.join("logs");
        let configure_log_path = logs_dir.join("configure.log");
        let build_log_path = logs_dir.join("build.log");
        let metadata_path = pkg_build_root.join("metadata.toml");

        // 4. Check for reusable cached build result
        if let Some(result) = Self::cached_result(request, &fingerprint, &pkg_build_root) {
            return Ok(result);
        }

        // 5. Prepare build directory: ensure build/ and logs/ exist and are clean
        if build_dir.exists() && request.force {
            fs::remove_dir_all(&build_dir).map_err(BuildError::Io)?;
        }
        fs::create_dir_all(&build_dir).map_err(BuildError::Io)?;
        fs::create_dir_all(&logs_dir).map_err(BuildError::Io)?;

        // 6. Execute CMake Configure step
        let mut configure_cmd = Command::new(&self.cmake_bin);
        configure_cmd.arg("-S").arg(&request.source_dir);
        configure_cmd.arg("-B").arg(&build_dir);
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

        if let Some(c_compiler) = &request.configuration.c_compiler {
            configure_cmd.arg(format!("-DCMAKE_C_COMPILER={}", c_compiler.display()));
        } else if let Some(c_compiler) = &self.c_compiler_bin {
            configure_cmd.arg(format!("-DCMAKE_C_COMPILER={}", c_compiler.display()));
        }

        if let Some(cxx_compiler) = &request.configuration.cxx_compiler {
            configure_cmd.arg(format!("-DCMAKE_CXX_COMPILER={}", cxx_compiler.display()));
        } else if let Some(cxx_compiler) = &self.cxx_compiler_bin {
            configure_cmd.arg(format!("-DCMAKE_CXX_COMPILER={}", cxx_compiler.display()));
        }

        // Add custom CMake options
        for arg in &request.configuration.cmake_args {
            configure_cmd.arg(arg);
        }

        // Add custom environment variables
        for (k, v) in &request.configuration.extra_env {
            configure_cmd.env(k, v);
        }

        let configure_command = crate::core::process::describe_command(&configure_cmd);
        let configure_output = Self::run_step(
            &mut configure_cmd,
            &configure_log_path,
            &configure_command,
            request,
        )
        .map_err(|e| {
            BuildError::MissingCMake(format!(
                "failed to launch CMake configure at '{}': {e}",
                self.cmake_bin.display()
            ))
        })?;

        if !configure_output.status.success() {
            return Err(BuildError::ConfigureFailed {
                exit_code: configure_output.status.code(),
                log_path: configure_log_path,
                summary: Self::extract_log_tail(&Self::failure_text(&configure_output), 20),
                command: configure_command,
                build_dir,
            });
        }

        // 7. Execute CMake Build step
        let mut build_cmd = Command::new(&self.cmake_bin);
        build_cmd.arg("--build").arg(&build_dir);
        build_cmd
            .arg("--config")
            .arg(request.configuration.profile.as_cmake_build_type());

        if let Some(jobs) = request.configuration.jobs {
            build_cmd.arg("--parallel").arg(jobs.to_string());
        }

        let build_command = crate::core::process::describe_command(&build_cmd);
        let build_output = Self::run_step(&mut build_cmd, &build_log_path, &build_command, request)
            .map_err(|e| {
                BuildError::MissingCMake(format!(
                    "failed to launch CMake build at '{}': {e}",
                    self.cmake_bin.display()
                ))
            })?;

        if !build_output.status.success() {
            return Err(BuildError::BuildFailed {
                exit_code: build_output.status.code(),
                log_path: build_log_path,
                summary: Self::extract_log_tail(&Self::failure_text(&build_output), 20),
                command: build_command,
                build_dir,
            });
        }

        // 8. Scan for produced artifacts
        let artifacts = Self::scan_artifacts(&build_dir)?;
        let duration = start_time.elapsed();

        // 9. Serialize and save BuildMetadata atomically
        let metadata = BuildMetadata {
            format_version: CURRENT_BUILD_METADATA_VERSION,
            package_name: request.package_id.name().to_string(),
            package_version: request.package_id.version().to_string(),
            source_checksum: request.source_checksum.to_string(),
            fingerprint: fingerprint.clone(),
            profile: request.configuration.profile,
            toolchain,
            configuration: request.configuration.clone(),
            source_dir: request.source_dir.clone(),
            build_dir: build_dir.clone(),
            configure_exit_code: configure_output.status.code().unwrap_or(0),
            build_exit_code: build_output.status.code().unwrap_or(0),
            duration_millis: duration.as_millis() as u64,
            artifacts: artifacts.clone(),
            success: true,
        };

        let metadata_toml = toml::to_string_pretty(&metadata).map_err(|e| {
            BuildError::MetadataError(format!("failed to serialize build metadata: {e}"))
        })?;
        Self::write_atomic(&metadata_path, metadata_toml.as_bytes())?;

        Ok(BuildResult {
            package_id: request.package_id.clone(),
            fingerprint,
            source_dir: request.source_dir.clone(),
            build_dir,
            logs_dir,
            configure_log_path,
            build_log_path,
            metadata_path,
            artifacts,
            profile: request.configuration.profile,
            duration,
            reused_cache: false,
        })
    }
}
