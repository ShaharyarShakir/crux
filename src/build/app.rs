//! Application build engine for compiling Crux projects.
//!
//! The application build pipeline is intentionally split into distinct stages:
//!
//! 1. **Planning** ([`AppBuildModel::prepare_with_cmake`]): load `crux.toml`,
//!    validate the project configuration, detect the compiler via the M18
//!    toolchain engine, discover source files under `src/`, and resolve the
//!    deterministic build layout.
//! 2. **Generation** ([`super::generator`]): produce an internal CMake project
//!    under `.crux/build/generated/`.
//! 3. **Execution** ([`AppBuildEngine::execute`]): configure and compile
//!    out-of-source under `.crux/build/<profile>/`.
//! 4. **Reporting** ([`AppBuildResult`]): expose structured results to the CLI.
//!
//! Users never create or maintain a root-level `CMakeLists.txt`; CMake is an
//! internal backend managed entirely by Crux.

use super::BuildError;
use super::dependency::{AppDependencyPlan, plan_app_dependencies};
use super::discovery::{DiscoveredSources, DiscoveryError, discover_sources_with_roots};
use super::fingerprint::{AppFingerprintInputs, app_build_fingerprint};
use super::generator::{AppCMakeConfig, generate_app_cmakelists, sanitize_target_name};
use super::metadata::BuildProfile;
use super::sanitizer::check_sanitizer_support;
use super::toolchain::{
    CompilerFamily, ProjectToolchain, ToolchainError, detect_project_toolchain, find_in_path,
};
use crate::core::filesystem::write_atomically;
use crate::core::process::{
    DEFAULT_TAIL_LIMIT, LoggedRun, OutputTee, ProcessOutput, describe_command, run_logged,
    run_with_timeout,
};
use crate::project::config::ProjectConfig;
use crate::project::import::ImportMetadata;
use crate::project::manifest::{
    ProjectLanguage, default_standard_for_language, normalize_standard,
};
use crate::resolution::lockfile::{LOCKFILE_FILENAME, Lockfile};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// File in the profile build directory recording the build fingerprint.
const BUILD_FINGERPRINT_FILE: &str = "crux-build.fingerprint";
/// Timeout for short toolchain probes such as `cmake --version`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for CMake configure steps.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Timeout for CMake build steps.
const BUILD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Kind of application target produced by the build engine.
///
/// M19 supports a single executable target, but the kind is modelled explicitly
/// so library and multi-target builds can be added without rewriting the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppTargetKind {
    Executable,
}

/// A single application target managed by Crux.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AppTarget {
    /// Sanitized target name, derived from the project name.
    pub name: String,
    /// Target kind (currently always an executable).
    pub kind: AppTargetKind,
}

impl AppTarget {
    /// Creates an executable target, normalizing the supplied name safely.
    pub fn executable(name: impl AsRef<str>) -> Self {
        Self {
            name: sanitize_target_name(name.as_ref()),
            kind: AppTargetKind::Executable,
        }
    }

    /// Creates the default executable target for a project name.
    pub fn for_project(project_name: &str) -> Self {
        Self::executable(project_name)
    }

    /// Returns the host-appropriate executable file name.
    pub fn executable_file_name(&self) -> String {
        if cfg!(windows) {
            format!("{}.exe", self.name)
        } else {
            self.name.clone()
        }
    }
}

/// Detected CMake backend used for application builds.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CMakeInfo {
    /// CMake executable path or command name.
    pub path: PathBuf,
    /// Full first line of `cmake --version` (for example `CMake 4.1.2`).
    pub version: String,
    /// Explicitly selected generator, if any.
    pub generator: Option<String>,
}

impl CMakeInfo {
    /// Returns a human-readable generator name.
    pub fn generator_display(&self) -> &str {
        self.generator.as_deref().unwrap_or("CMake default")
    }
}

/// Deterministic project-local build layout.
///
/// All generated and compiled artifacts live under `.crux/build/`. Debug and
/// Release builds use isolated profile directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBuildLayout {
    /// `.crux/build`
    pub build_root: PathBuf,
    /// `.crux/build/generated`
    pub generated_dir: PathBuf,
    /// `.crux/build/generated/CMakeLists.txt`
    pub generated_cmake_path: PathBuf,
    /// `.crux/build/<profile>`
    pub profile_dir: PathBuf,
    /// `.crux/build/logs`
    pub logs_dir: PathBuf,
    /// `.crux/build/logs/configure-<profile>.log`
    pub configure_log_path: PathBuf,
    /// `.crux/build/logs/build-<profile>.log`
    pub build_log_path: PathBuf,
    /// Expected executable path after a successful build.
    pub executable_path: PathBuf,
}

impl AppBuildLayout {
    /// Computes the build layout for a project, profile, and target.
    pub fn new(project_root: &Path, profile: BuildProfile, target: &AppTarget) -> Self {
        let build_root = project_root.join(".crux").join("build");
        let generated_dir = build_root.join("generated");
        let generated_cmake_path = generated_dir.join("CMakeLists.txt");
        let profile_dir = build_root.join(profile.as_str());
        let logs_dir = build_root.join("logs");
        let configure_log_path = logs_dir.join(format!("configure-{}.log", profile.as_str()));
        let build_log_path = logs_dir.join(format!("build-{}.log", profile.as_str()));
        let executable_path = profile_dir.join(target.executable_file_name());

        Self {
            build_root,
            generated_dir,
            generated_cmake_path,
            profile_dir,
            logs_dir,
            configure_log_path,
            build_log_path,
            executable_path,
        }
    }
}

/// Reusable, fully-resolved model of an application build.
///
/// This is the single source of truth consumed by CMake generation, build
/// execution, and CLI reporting. Creating a model performs no compilation and
/// writes nothing to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBuildModel {
    /// Canonical project root directory.
    pub project_root: PathBuf,
    /// Validated project configuration loaded from `crux.toml`.
    pub config: ProjectConfig,
    /// Effective application language.
    pub language: ProjectLanguage,
    /// Effective, normalized language standard.
    pub standard: String,
    /// Detected and validated compiler toolchain (M18).
    pub toolchain: ProjectToolchain,
    /// Deterministically ordered source files discovered under `src/`.
    pub sources: DiscoveredSources,
    /// Additional include directories supplied by ad-hoc `crux run` invocations.
    pub extra_include_dirs: Vec<PathBuf>,
    /// Project-relative include directories recorded by `crux import`.
    pub project_include_dirs: Vec<String>,
    /// Validated dependency link plan (empty when no dependencies are declared).
    pub dependencies: AppDependencyPlan,
    /// Application target metadata.
    pub target: AppTarget,
    /// Selected Debug/Release configuration.
    pub profile: BuildProfile,
    /// Project-local build layout.
    pub layout: AppBuildLayout,
    /// Detected CMake backend.
    pub cmake: CMakeInfo,
}

impl AppBuildModel {
    /// Prepares an application build model using the default `cmake` binary.
    pub fn prepare(project_dir: &Path, profile: BuildProfile) -> Result<Self, BuildError> {
        Self::prepare_with_cmake(project_dir, profile, Path::new("cmake"))
    }

    /// Prepares an application build model using an explicit CMake binary.
    pub fn prepare_with_cmake(
        project_dir: &Path,
        profile: BuildProfile,
        cmake_bin: &Path,
    ) -> Result<Self, BuildError> {
        // 1. Resolve and validate the project root.
        let project_root = project_dir.canonicalize().map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => BuildError::InvalidConfiguration(format!(
                "project directory '{}' does not exist",
                project_dir.display()
            )),
            _ => BuildError::Io(err),
        })?;

        if !project_root.is_dir() {
            return Err(BuildError::InvalidConfiguration(format!(
                "project path '{}' is not a directory",
                project_root.display()
            )));
        }

        // 2. Load and validate the manifest / project configuration.
        let config = ProjectConfig::load_from_dir(&project_root)
            .map_err(|err| BuildError::InvalidConfiguration(err.to_string()))?;

        if !config.is_project {
            return Err(BuildError::InvalidConfiguration(format!(
                "manifest in '{}' does not define a [project] section; application builds require a Crux project manifest",
                project_root.display()
            )));
        }

        // 3. Resolve the application language.
        let language = match config.language {
            Some(ProjectLanguage::Other) => {
                return Err(BuildError::InvalidConfiguration(
                    "projects with language 'other' cannot be compiled with 'crux build'; only C and C++ are supported"
                        .to_string(),
                ));
            }
            Some(lang) => lang,
            None => ProjectLanguage::Cpp,
        };

        // 4. Resolve and normalize the language standard.
        let standard = normalize_standard(
            config
                .standard
                .as_deref()
                .or_else(|| default_standard_for_language(language))
                .unwrap_or("c++20"),
        );

        // 5. Detect and validate the compiler via the M18 toolchain engine.
        let toolchain = detect_project_toolchain(language, Some(&standard)).map_err(|err| {
            match err {
                ToolchainError::CompilerNotFound {
                    language,
                    candidate_or_env,
                } => BuildError::ToolchainDetectionFailed(format!(
                    "no compatible {} compiler found ({candidate_or_env})",
                    language.display_name()
                )),
                ToolchainError::UnsupportedStandard {
                    language,
                    standard,
                    compiler,
                    compiler_version,
                } => BuildError::InvalidConfiguration(format!(
                    "compiler {compiler} ({compiler_version}) does not support requested {} standard '{standard}'",
                    language.display_name()
                )),
                other => BuildError::ToolchainDetectionFailed(other.to_string()),
            }
        })?;

        // 5b. Requested sanitizers must be buildable with this exact compiler;
        //     they are never silently dropped.
        check_sanitizer_support(&toolchain.compiler, &config.build)?;

        // 6. Load Crux import metadata and discover application sources.
        //    Projects without import metadata retain canonical src/ behavior.
        let import_metadata = ImportMetadata::load_from_project(&project_root).map_err(|err| {
            BuildError::InvalidConfiguration(format!("invalid Crux import metadata: {err}"))
        })?;
        let project_include_dirs = match &import_metadata {
            Some(metadata) => metadata
                .resolve_include_dirs(&project_root)
                .map_err(|err| {
                    BuildError::InvalidConfiguration(format!("invalid Crux import metadata: {err}"))
                })?,
            None => Vec::new(),
        };
        let import_source_roots = import_metadata
            .as_ref()
            .map(|metadata| metadata.source_roots.as_slice());
        let sources = discover_sources_with_roots(&project_root, language, import_source_roots)
            .map_err(map_discovery_error)?;

        // 7. Resolve the dependency link plan from validated installations.
        let dependencies = if config.dependencies.is_empty() {
            AppDependencyPlan::default()
        } else {
            let lockfile_path = project_root.join(LOCKFILE_FILENAME);
            let lockfile = Lockfile::load_from_path(&lockfile_path).map_err(|err| {
                BuildError::InvalidConfiguration(format!(
                    "unable to link dependencies: lockfile '{}' could not be read ({err}); \
                     run 'crux lock' to generate it",
                    lockfile_path.display()
                ))
            })?;
            plan_app_dependencies(&project_root, &lockfile, &config.dependencies, profile)?
        };

        // 8. Detect the CMake backend and select a generator.
        let cmake = detect_cmake(cmake_bin)?;

        // 9. Derive the application target and deterministic layout.
        let target = AppTarget::for_project(config.name());
        let layout = AppBuildLayout::new(&project_root, profile, &target);

        Ok(Self {
            project_root,
            config,
            language,
            standard,
            toolchain,
            sources,
            extra_include_dirs: Vec::new(),
            project_include_dirs,
            dependencies,
            target,
            profile,
            layout,
            cmake,
        })
    }

    /// Fingerprint of everything that can change this build's compiled output.
    pub fn build_fingerprint(&self) -> String {
        let dependency_fingerprints: Vec<String> = self
            .dependencies
            .dependencies
            .iter()
            .map(|d| format!("{}@{}:{}", d.name, d.version, d.fingerprint))
            .collect();
        app_build_fingerprint(&AppFingerprintInputs {
            compiler: &self.toolchain.compiler,
            language: self.language,
            standard: &self.standard,
            profile: self.profile.as_str(),
            cmake_version: &self.cmake.version,
            generator: self.cmake.generator.as_deref(),
            settings: &self.config.build,
            dependency_fingerprints: &dependency_fingerprints,
        })
    }

    /// Returns the project name.
    pub fn project_name(&self) -> &str {
        &self.config.name
    }

    /// Returns the number of discovered application source files.
    pub fn source_count(&self) -> usize {
        self.sources.files.len()
    }

    /// Returns the number of direct and transitive linked dependencies.
    pub fn dependency_count(&self) -> usize {
        self.dependencies.len()
    }
}

/// Request to build an application project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBuildRequest {
    pub project_dir: PathBuf,
    pub profile: BuildProfile,
    pub verbose: bool,
    pub force: bool,
}

impl AppBuildRequest {
    pub fn new(project_dir: impl Into<PathBuf>) -> Self {
        Self {
            project_dir: project_dir.into(),
            profile: BuildProfile::Debug,
            verbose: false,
            force: false,
        }
    }

    pub fn with_profile(mut self, profile: BuildProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

/// Result of a completed application build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBuildResult {
    pub project_name: String,
    pub language: ProjectLanguage,
    pub standard: String,
    pub compiler_family: CompilerFamily,
    pub compiler_version: String,
    pub compiler_path: PathBuf,
    pub cmake_version: String,
    pub generator: Option<String>,
    pub profile: BuildProfile,
    pub target_name: String,
    pub source_count: usize,
    pub build_dir: PathBuf,
    pub executable_path: PathBuf,
    pub generated_cmake_path: PathBuf,
    pub configure_log_path: PathBuf,
    pub build_log_path: PathBuf,
    pub duration: Duration,
}

/// Engine for compiling Crux application projects.
pub struct AppBuildEngine {
    cmake_bin: PathBuf,
}

impl Default for AppBuildEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AppBuildEngine {
    /// Creates a new application build engine using the default `cmake` binary in PATH.
    pub fn new() -> Self {
        Self {
            cmake_bin: PathBuf::from("cmake"),
        }
    }

    /// Overrides the path to the CMake binary.
    pub fn with_cmake_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.cmake_bin = bin.into();
        self
    }

    /// Prepares a build model without compiling anything.
    pub fn prepare(&self, request: &AppBuildRequest) -> Result<AppBuildModel, BuildError> {
        AppBuildModel::prepare_with_cmake(&request.project_dir, request.profile, &self.cmake_bin)
    }

    /// Executes a prepared application build model.
    pub fn execute(
        &self,
        model: &AppBuildModel,
        force: bool,
        verbose: bool,
    ) -> Result<AppBuildResult, BuildError> {
        self.execute_with_tee(model, force, verbose, None)
    }

    /// Like [`AppBuildEngine::execute`], streaming CMake and compiler output
    /// to `tee` while it is produced (used for `-vv`).
    pub fn execute_with_tee(
        &self,
        model: &AppBuildModel,
        force: bool,
        verbose: bool,
        tee: Option<OutputTee>,
    ) -> Result<AppBuildResult, BuildError> {
        let start_time = Instant::now();

        // 1. Ensure project-local build directories exist.
        fs::create_dir_all(&model.layout.profile_dir).map_err(BuildError::Io)?;
        fs::create_dir_all(&model.layout.logs_dir).map_err(BuildError::Io)?;

        // 2. Honor --force by discarding the profile build tree.
        if force && model.layout.profile_dir.exists() {
            fs::remove_dir_all(&model.layout.profile_dir).map_err(BuildError::Io)?;
            fs::create_dir_all(&model.layout.profile_dir).map_err(BuildError::Io)?;
        }

        // 3. Discard stale CMake caches when the compiler or generator changed.
        if build_dir_is_stale(
            &model.layout.profile_dir,
            model.language,
            &model.toolchain.compiler.path,
            model.cmake.generator.as_deref(),
        ) {
            fs::remove_dir_all(&model.layout.profile_dir).map_err(BuildError::Io)?;
            fs::create_dir_all(&model.layout.profile_dir).map_err(BuildError::Io)?;
        }

        // 3b. Discard a build tree produced under a different configuration
        //     (compiler version, flags, sanitizers, profile, dependencies).
        let fingerprint = model.build_fingerprint();
        let fingerprint_path = model.layout.profile_dir.join(BUILD_FINGERPRINT_FILE);
        let previous = fs::read_to_string(&fingerprint_path).ok();
        let has_prior_tree = model.layout.profile_dir.join("CMakeCache.txt").exists();
        // A recorded fingerprint that differs, or a pre-fingerprint tree
        // (created by an older Crux) whose configuration is unknown.
        let stale = match &previous {
            Some(previous) => previous.trim() != fingerprint,
            None => has_prior_tree,
        };
        if stale {
            fs::remove_dir_all(&model.layout.profile_dir).map_err(BuildError::Io)?;
            fs::create_dir_all(&model.layout.profile_dir).map_err(BuildError::Io)?;
        }
        write_atomically(&fingerprint_path, format!("{fingerprint}\n").as_bytes())
            .map_err(BuildError::Io)?;

        // 4. Generate internal CMake configuration (deterministic, atomic).
        let mut dependency_include_dirs = model.dependencies.include_dirs();
        dependency_include_dirs.extend(model.extra_include_dirs.iter().cloned());
        let dependency_link_directories = model.dependencies.link_directories();
        let dependency_link_libraries = model.dependencies.link_libraries();
        let dependency_runtime_directories = model.dependencies.runtime_directories();

        let generated_content = generate_app_cmakelists(&AppCMakeConfig {
            project_root: &model.project_root,
            target_name: &model.target.name,
            project_version: &format!(
                "{}.{}.{}",
                model.config.version.major, model.config.version.minor, model.config.version.patch
            ),
            language: model.language,
            standard: &model.standard,
            sources: &model.sources.files,
            project_include_dirs: &model.project_include_dirs,
            dependency_include_dirs: &dependency_include_dirs,
            dependency_link_directories: &dependency_link_directories,
            dependency_link_libraries: &dependency_link_libraries,
            dependency_runtime_directories: &dependency_runtime_directories,
            build_settings: &model.config.build,
        })?;
        write_if_changed(
            &model.layout.generated_cmake_path,
            generated_content.as_bytes(),
        )?;

        // 5. Configure.
        let mut configure = Command::new(&model.cmake.path);
        configure.arg("-S").arg(&model.layout.generated_dir);
        configure.arg("-B").arg(&model.layout.profile_dir);
        configure.arg(format!(
            "-DCMAKE_BUILD_TYPE={}",
            model.profile.as_cmake_build_type()
        ));
        if let Some(generator) = &model.cmake.generator {
            configure.arg("-G").arg(generator);
        }
        match model.language {
            ProjectLanguage::C => {
                configure.arg(format!(
                    "-DCMAKE_C_COMPILER={}",
                    model.toolchain.compiler.path.display()
                ));
            }
            ProjectLanguage::Cpp => {
                configure.arg(format!(
                    "-DCMAKE_CXX_COMPILER={}",
                    model.toolchain.compiler.path.display()
                ));
            }
            ProjectLanguage::Other => unreachable!("language validated during model preparation"),
        }
        if verbose {
            configure.arg("--log-level=VERBOSE");
        }

        let configure_command = describe_command(&configure);
        let configure_output = run_logged(
            &mut configure,
            CONFIGURE_TIMEOUT,
            LoggedRun {
                log_path: &model.layout.configure_log_path,
                header: format!("$ {configure_command}\n\n"),
                tail_limit: DEFAULT_TAIL_LIMIT,
                tee: tee.clone(),
            },
        )
        .map_err(|e| {
            BuildError::MissingCMake(format!(
                "failed to launch CMake configure at '{}': {e}",
                model.cmake.path.display()
            ))
        })?;

        if !configure_output.success() {
            return Err(BuildError::ConfigureFailed {
                exit_code: configure_output.code(),
                log_path: model.layout.configure_log_path.clone(),
                summary: build_failure_summary(&configure_output, 20),
                command: configure_command,
                build_dir: model.layout.profile_dir.clone(),
            });
        }

        // 6. Compile.
        let mut build = Command::new(&model.cmake.path);
        build.arg("--build").arg(&model.layout.profile_dir);
        build
            .arg("--config")
            .arg(model.profile.as_cmake_build_type());
        if verbose {
            build.arg("--verbose");
        }

        let build_command = describe_command(&build);
        let build_output = run_logged(
            &mut build,
            BUILD_TIMEOUT,
            LoggedRun {
                log_path: &model.layout.build_log_path,
                header: format!("$ {build_command}\n\n"),
                tail_limit: DEFAULT_TAIL_LIMIT,
                tee,
            },
        )
        .map_err(|e| {
            BuildError::MissingCMake(format!(
                "failed to launch CMake build at '{}': {e}",
                model.cmake.path.display()
            ))
        })?;

        if !build_output.success() {
            return Err(BuildError::BuildFailed {
                exit_code: build_output.code(),
                log_path: model.layout.build_log_path.clone(),
                summary: build_failure_summary(&build_output, 25),
                command: build_command,
                build_dir: model.layout.profile_dir.clone(),
            });
        }

        // 7. Validate the expected executable exists before reporting success.
        let executable_path =
            locate_executable(&model.layout.profile_dir, &model.target, model.profile);

        if !executable_path.is_file() {
            return Err(BuildError::BuildFailed {
                exit_code: Some(0),
                log_path: model.layout.build_log_path.clone(),
                summary: format!(
                    "build finished successfully but expected executable '{}' was not created",
                    executable_path.display()
                ),
                command: build_command,
                build_dir: model.layout.profile_dir.clone(),
            });
        }

        Ok(AppBuildResult {
            project_name: model.config.name.clone(),
            language: model.language,
            standard: model.standard.clone(),
            compiler_family: model.toolchain.compiler.family,
            compiler_version: model.toolchain.compiler.version.clone(),
            compiler_path: model.toolchain.compiler.path.clone(),
            cmake_version: model.cmake.version.clone(),
            generator: model.cmake.generator.clone(),
            profile: model.profile,
            target_name: model.target.name.clone(),
            source_count: model.sources.files.len(),
            build_dir: model.layout.profile_dir.clone(),
            executable_path,
            generated_cmake_path: model.layout.generated_cmake_path.clone(),
            configure_log_path: model.layout.configure_log_path.clone(),
            build_log_path: model.layout.build_log_path.clone(),
            duration: start_time.elapsed(),
        })
    }

    /// Prepares and executes a complete application build.
    pub fn build(&self, request: &AppBuildRequest) -> Result<AppBuildResult, BuildError> {
        let model = self.prepare(request)?;
        self.execute(&model, request.force, request.verbose)
    }
}

fn map_discovery_error(error: DiscoveryError) -> BuildError {
    match error {
        DiscoveryError::MissingSourceDirectory(path) => BuildError::InvalidConfiguration(format!(
            "source directory not found at '{}': Crux projects must contain a 'src/' directory",
            path.display()
        )),
        DiscoveryError::NoSourceRootsFound(path) => BuildError::InvalidConfiguration(format!(
            "no compilable source roots found in '{}'; add sources under src/ or import the project with 'crux import'",
            path.display()
        )),
        DiscoveryError::InvalidSourceRoot { path, reason } => BuildError::InvalidConfiguration(
            format!("invalid source root '{}': {reason}", path.display()),
        ),
        DiscoveryError::NoSourceFilesFound(path) => BuildError::InvalidConfiguration(format!(
            "no application source files found in '{}'; add a C or C++ compilation unit",
            path.display()
        )),
        DiscoveryError::LanguageMismatch { reason, .. } => BuildError::InvalidConfiguration(reason),
        DiscoveryError::UnsupportedSourceExtension { file, extension } => {
            BuildError::InvalidConfiguration(format!(
                "unsupported application source extension '.{extension}' in '{}'; only the project language's source extensions may appear under src/",
                file.display()
            ))
        }
        DiscoveryError::SymlinkEscape { symlink, target } => {
            BuildError::SecurityViolation(format!(
                "symlink escape at '{}' pointing outside the project source tree ('{}')",
                symlink.display(),
                target.display()
            ))
        }
        DiscoveryError::UnsupportedLanguage(language) => BuildError::InvalidConfiguration(format!(
            "projects with language '{}' cannot be compiled with 'crux build'",
            language.display_name()
        )),
        DiscoveryError::Io(error) => BuildError::Io(error),
    }
}

pub(crate) fn detect_cmake(cmake_bin: &Path) -> Result<CMakeInfo, BuildError> {
    let mut command = Command::new(cmake_bin);
    command.arg("--version");

    let output = run_with_timeout(&mut command, PROBE_TIMEOUT).map_err(|error| {
        BuildError::MissingCMake(format!(
            "CMake is required to build Crux applications but '{}' could not be executed: {error}",
            cmake_bin.display()
        ))
    })?;

    if output.timed_out {
        return Err(BuildError::MissingCMake(format!(
            "'{} --version' timed out after {} seconds",
            cmake_bin.display(),
            PROBE_TIMEOUT.as_secs()
        )));
    }

    if !output.status.success() {
        return Err(BuildError::MissingCMake(format!(
            "'{} --version' failed with exit status {}",
            cmake_bin.display(),
            output.status
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw_version = stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("cmake version unknown");

    let version = raw_version
        .strip_prefix("cmake version ")
        .map(|value| format!("CMake {value}"))
        .unwrap_or_else(|| raw_version.to_string());

    Ok(CMakeInfo {
        path: cmake_bin.to_path_buf(),
        version,
        generator: select_generator(),
    })
}

/// Selects a deterministic single-configuration generator when available.
///
/// Single-configuration generators keep Debug and Release output layouts
/// predictable and avoid generator-specific multi-config subdirectories.
fn select_generator() -> Option<String> {
    if find_in_path(Path::new("ninja")).is_some() {
        return Some("Ninja".to_string());
    }

    #[cfg(unix)]
    {
        if find_in_path(Path::new("make")).is_some() {
            return Some("Unix Makefiles".to_string());
        }
    }

    None
}

fn write_if_changed(path: &Path, content: &[u8]) -> Result<(), BuildError> {
    let should_write = match fs::read(path) {
        Ok(existing) => existing != content,
        Err(_) => true,
    };

    if should_write {
        write_atomically(path, content).map_err(BuildError::Io)?;
    }

    Ok(())
}

fn build_failure_summary(output: &ProcessOutput, max_lines: usize) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = if stderr.trim().is_empty() {
        stdout.as_ref()
    } else {
        stderr.as_ref()
    };

    let tail = extract_log_tail(combined, max_lines);

    if output.timed_out {
        format!("process timed out and was terminated\n{tail}")
    } else {
        tail
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

fn locate_executable(profile_dir: &Path, target: &AppTarget, profile: BuildProfile) -> PathBuf {
    let file_name = target.executable_file_name();
    let primary = profile_dir.join(&file_name);
    if primary.is_file() {
        return primary;
    }

    // Fallback for multi-configuration generators.
    let multi_config = profile_dir
        .join(profile.as_cmake_build_type())
        .join(&file_name);
    if multi_config.is_file() {
        return multi_config;
    }

    primary
}

/// Returns true when an existing CMake cache was produced by a different
/// compiler or generator and must be discarded before reconfiguration.
fn build_dir_is_stale(
    build_dir: &Path,
    language: ProjectLanguage,
    compiler: &Path,
    generator: Option<&str>,
) -> bool {
    let cache_path = build_dir.join("CMakeCache.txt");
    let cache = match fs::read_to_string(&cache_path) {
        Ok(content) => content,
        Err(_) => return false,
    };

    let compiler_key = match language {
        ProjectLanguage::C => "CMAKE_C_COMPILER",
        ProjectLanguage::Cpp => "CMAKE_CXX_COMPILER",
        ProjectLanguage::Other => return true,
    };

    if let Some(cached_compiler) = cache_value(&cache, compiler_key) {
        if !paths_equivalent(Path::new(cached_compiler), compiler) {
            return true;
        }
    }

    if let Some(expected_generator) = generator {
        if let Some(cached_generator) = cache_value(&cache, "CMAKE_GENERATOR") {
            if cached_generator != expected_generator {
                return true;
            }
        }
    }

    false
}

fn cache_value<'a>(cache: &'a str, key: &str) -> Option<&'a str> {
    cache.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        if name.split(':').next() == Some(key) {
            Some(value)
        } else {
            None
        }
    })
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_app_target_names() {
        let target = AppTarget::for_project("my-demo");
        assert_eq!(target.name, "my-demo");
        assert_eq!(target.kind, AppTargetKind::Executable);
        assert_eq!(
            target.executable_file_name(),
            if cfg!(windows) {
                "my-demo.exe".to_string()
            } else {
                "my-demo".to_string()
            }
        );
    }

    #[test]
    fn test_layout_isolates_profiles() {
        let root = Path::new("/workspace/demo");
        let target = AppTarget::for_project("demo");

        let debug = AppBuildLayout::new(root, BuildProfile::Debug, &target);
        let release = AppBuildLayout::new(root, BuildProfile::Release, &target);

        assert_eq!(debug.generated_dir, root.join(".crux/build/generated"));
        assert_eq!(debug.profile_dir, root.join(".crux/build/debug"));
        assert_eq!(release.profile_dir, root.join(".crux/build/release"));
        assert_ne!(debug.executable_path, release.executable_path);
        assert_eq!(
            debug.configure_log_path,
            root.join(".crux/build/logs/configure-debug.log")
        );
    }

    #[test]
    fn test_cache_value_parsing() {
        let cache = "CMAKE_C_COMPILER:FILEPATH=/usr/bin/gcc\nCMAKE_GENERATOR:INTERNAL=Ninja\n";
        assert_eq!(cache_value(cache, "CMAKE_C_COMPILER"), Some("/usr/bin/gcc"));
        assert_eq!(cache_value(cache, "CMAKE_GENERATOR"), Some("Ninja"));
        assert_eq!(cache_value(cache, "MISSING"), None);
    }

    #[test]
    fn test_extract_log_tail() {
        let log = "one\ntwo\nthree\nfour";
        assert_eq!(extract_log_tail(log, 10), log);
        assert_eq!(extract_log_tail(log, 2), "[...]\nthree\nfour");
    }
}
