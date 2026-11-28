//! CMake Build Engine for Crux.
//!
//! Provides isolated out-of-source configuration and compilation of C/C++ packages
//! acquired by Crux. Generates deterministic build fingerprints, executes CMake
//! via explicit subprocess argument vectors (never shell interpolation), captures
//! complete diagnostic logs, and records persistent build metadata.

pub mod adhoc;
pub mod app;
pub mod dependency;
pub mod discovery;
pub mod engine;
pub mod fingerprint;
pub mod generator;
pub mod metadata;
pub(crate) mod package_dependencies;
mod pipeline;
pub mod sanitizer;
mod selection;
pub(crate) mod systems;
pub mod toolchain;

pub use adhoc::{AdhocRunPlan, prepare_adhoc_build};
pub use app::{
    AppBuildEngine, AppBuildLayout, AppBuildModel, AppBuildRequest, AppBuildResult, AppTarget,
    AppTargetKind, CMakeInfo,
};
pub use dependency::{AppDependency, AppDependencyPlan, plan_app_dependencies};
pub use discovery::{
    DiscoveredSources, DiscoveryError, discover_sources, discover_sources_with_roots,
};
pub use engine::{BuildEngine, BuildRequest, BuildResult, CMakeBuildEngine};
pub use fingerprint::{AppFingerprintInputs, BuildFingerprint, app_build_fingerprint};
pub use generator::{
    AppCMakeConfig, escape_cmake_path, generate_app_cmakelists, minimum_cmake_version,
    sanitize_target_name, standard_to_cmake_standard,
};
pub use metadata::{
    ArtifactType, BuildArtifact, BuildConfiguration, BuildMetadata, BuildProfile,
    CURRENT_BUILD_METADATA_VERSION, SelectedCompiler, SelectedToolchain,
};
pub use sanitizer::{
    check_sanitizer_support, effective_compile_flags, effective_link_flags, sanitizer_flags,
};
pub use toolchain::{
    CompilerFamily, CompilerInfo, PlatformInfo, ProjectToolchain, ToolchainError, ToolchainInfo,
    detect_compiler, detect_compiler_with_custom, detect_project_toolchain,
    detect_project_toolchain_with_custom, probe_standard_support,
};

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Structured error type for the CMake build engine.
#[derive(Debug)]
pub enum BuildError {
    MissingSource(PathBuf),
    MissingCMakeLists(PathBuf),
    MissingCMake(String),
    ToolchainDetectionFailed(String),
    ConfigureFailed {
        exit_code: Option<i32>,
        log_path: PathBuf,
        summary: String,
        /// The CMake command that failed, formatted for display.
        command: String,
        /// The build directory, kept for investigation.
        build_dir: PathBuf,
    },
    BuildFailed {
        exit_code: Option<i32>,
        log_path: PathBuf,
        summary: String,
        /// The CMake command that failed, formatted for display.
        command: String,
        /// The build directory, kept for investigation.
        build_dir: PathBuf,
    },
    MetadataError(String),
    InvalidConfiguration(String),
    SecurityViolation(String),
    /// A declared or transitive dependency is missing or not installed for the
    /// selected profile.
    MissingDependency {
        name: String,
        version: String,
        profile: String,
        reason: String,
    },
    /// A requested sanitizer cannot be used with the detected toolchain.
    SanitizerUnsupported {
        compiler: String,
        sanitizers: String,
        reason: String,
        /// The probe command, when one was run.
        command: String,
        /// The compiler's own output, unmodified.
        output: String,
    },
    Io(io::Error),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::MissingSource(path) => {
                write!(f, "source directory does not exist: '{}'", path.display())
            }
            BuildError::MissingCMakeLists(path) => {
                write!(
                    f,
                    "missing CMakeLists.txt in source directory: '{}'",
                    path.display()
                )
            }
            BuildError::MissingCMake(msg) => write!(f, "CMake is not available: {msg}"),
            BuildError::ToolchainDetectionFailed(msg) => {
                write!(f, "failed to detect build toolchain: {msg}")
            }
            BuildError::ConfigureFailed {
                exit_code,
                log_path,
                summary,
                ..
            } => {
                let code_str = exit_code
                    .map(|c| format!("exit code {c}"))
                    .unwrap_or_else(|| "terminated by signal".to_string());
                write!(
                    f,
                    "CMake configuration failed ({code_str}). See log at '{}':\n{summary}",
                    log_path.display()
                )
            }
            BuildError::BuildFailed {
                exit_code,
                log_path,
                summary,
                ..
            } => {
                let code_str = exit_code
                    .map(|c| format!("exit code {c}"))
                    .unwrap_or_else(|| "terminated by signal".to_string());
                write!(
                    f,
                    "CMake compilation failed ({code_str}). See log at '{}':\n{summary}",
                    log_path.display()
                )
            }
            BuildError::MetadataError(msg) => write!(f, "build metadata error: {msg}"),
            BuildError::InvalidConfiguration(msg) => {
                write!(f, "invalid build configuration: {msg}")
            }
            BuildError::SecurityViolation(msg) => write!(f, "build security violation: {msg}"),
            BuildError::MissingDependency {
                name,
                version,
                profile,
                reason,
            } => write!(
                f,
                "dependency '{name}' ({version}) is not installed for the {profile} profile: {reason}"
            ),
            BuildError::SanitizerUnsupported {
                compiler,
                sanitizers,
                reason,
                output,
                ..
            } => {
                write!(f, "{sanitizers} cannot be used with {compiler}: {reason}")?;
                if !output.is_empty() {
                    write!(f, "\n{output}")?;
                }
                Ok(())
            }
            BuildError::Io(err) => write!(f, "I/O error during build: {err}"),
        }
    }
}

impl std::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BuildError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for BuildError {
    fn from(err: io::Error) -> Self {
        BuildError::Io(err)
    }
}
