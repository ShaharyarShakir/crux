//! CMake Build Engine for Crux.
//!
//! Provides isolated out-of-source configuration and compilation of C/C++ packages
//! acquired by Crux. Generates deterministic build fingerprints, executes CMake
//! via explicit subprocess argument vectors (never shell interpolation), captures
//! complete diagnostic logs, and records persistent build metadata.

pub mod engine;
pub mod fingerprint;
pub mod metadata;
pub mod toolchain;

pub use engine::{BuildEngine, BuildRequest, BuildResult, CMakeBuildEngine};
pub use fingerprint::BuildFingerprint;
pub use metadata::{
    ArtifactType, BuildArtifact, BuildConfiguration, BuildMetadata, BuildProfile,
    CURRENT_BUILD_METADATA_VERSION,
};
pub use toolchain::ToolchainInfo;

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
    },
    BuildFailed {
        exit_code: Option<i32>,
        log_path: PathBuf,
        summary: String,
    },
    MetadataError(String),
    InvalidConfiguration(String),
    SecurityViolation(String),
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
