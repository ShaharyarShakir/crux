//! Package Installation Engine for Crux.
//!
//! Installs built C/C++ packages into isolated, project-local directories
//! under `.crux/installed/<package>/<version>/<fingerprint>/` using CMake's
//! native `--install` mechanism. Provides safe staging, path validation,
//! atomic publication, persistent file inventory metadata (`installed.toml`),
//! and deterministic cache reuse.

pub mod engine;
pub mod inventory;
pub mod metadata;

pub use engine::{CMakeInstallEngine, InstallEngine, InstallRequest, InstallResult};
pub use inventory::{merge_dir_contents, promote_unwrapped_prefix, scan_staging_dir};
pub use metadata::{
    CURRENT_INSTALL_METADATA_VERSION, INSTALL_METADATA_FILENAME, InstallMetadata, InstalledFile,
    InstalledFileType,
};

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Structured error type for the package installation engine.
#[derive(Debug)]
pub enum InstallError {
    InvalidBuildResult(String),
    MissingBuildDirectory(PathBuf),
    MissingCMakeConfig(PathBuf),
    CMakeExecutionFailed {
        exit_code: Option<i32>,
        log_path: PathBuf,
        summary: String,
    },
    EmptyInstallation(PathBuf),
    SecurityViolation(String),
    MetadataError(String),
    Io(io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBuildResult(msg) => write!(f, "invalid build result: {msg}"),
            Self::MissingBuildDirectory(path) => {
                write!(f, "build directory does not exist: '{}'", path.display())
            }
            Self::MissingCMakeConfig(path) => {
                write!(
                    f,
                    "build directory missing CMake configuration (CMakeCache.txt or cmake_install.cmake): '{}'",
                    path.display()
                )
            }
            Self::CMakeExecutionFailed {
                exit_code,
                log_path,
                summary,
            } => {
                let code_str = exit_code
                    .map(|c| format!("exit code {c}"))
                    .unwrap_or_else(|| "terminated by signal".to_string());
                write!(
                    f,
                    "CMake installation failed ({code_str}). See log at '{}':\n{summary}",
                    log_path.display()
                )
            }
            Self::EmptyInstallation(path) => {
                write!(
                    f,
                    "staged installation at '{}' contains no files (missing or incomplete CMake install rules)",
                    path.display()
                )
            }
            Self::SecurityViolation(msg) => write!(f, "installation security violation: {msg}"),
            Self::MetadataError(msg) => write!(f, "installation metadata error: {msg}"),
            Self::Io(err) => write!(f, "I/O error during installation: {err}"),
        }
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
