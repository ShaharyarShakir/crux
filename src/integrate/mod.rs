//! CMake Consumer Integration for Crux.
//!
//! Generates deterministic CMake integration files (`.crux/cmake/CruxDependencies.cmake`)
//! that make Crux-installed packages discoverable through standard CMake mechanisms:
//! `CMAKE_PREFIX_PATH`, `CMAKE_MODULE_PATH`, upstream CMake package configurations
//! (`find_package(<pkg> CONFIG)`), and built-in Find modules (`find_package(<pkg>)`).

pub mod cmake;
pub mod validation;

pub use cmake::{
    CMakeIntegrationGenerator, DEFAULT_CMAKE_INTEGRATION_SUBPATH, IntegrationResult,
    escape_cmake_path, generate_cmake_dependencies, sanitize_cmake_identifier,
    write_cmake_dependencies_atomically,
};
pub use validation::{
    ValidatedPackageInstallation, collect_transitive_dependencies, validate_package_installation,
};

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Errors encountered during CMake consumer integration.
#[derive(Debug)]
pub enum IntegrationError {
    LockfileMissing(PathBuf),
    LockfileOutOfSync(String),
    PackageNotFoundInLockfile(String),
    PackageNotInstalled { package: String, version: String },
    CorruptInstallation { package: String, reason: String },
    SecurityViolation { package: String, reason: String },
    InvalidPath(String),
    ManifestError(String),
    Io(io::Error),
}

impl fmt::Display for IntegrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LockfileMissing(path) => {
                write!(
                    f,
                    "lockfile does not exist at '{}'. Run 'crux lock' or 'crux install' first.",
                    path.display()
                )
            }
            Self::LockfileOutOfSync(msg) => {
                write!(
                    f,
                    "lockfile is out of sync with manifest: {msg}. Run 'crux lock' to synchronize."
                )
            }
            Self::PackageNotFoundInLockfile(name) => {
                write!(f, "package '{name}' is not declared in lockfile")
            }
            Self::PackageNotInstalled { package, version } => {
                write!(
                    f,
                    "package '{package}' ({version}) is locked in crux.lock but not installed on disk. Run 'crux install' to install dependencies."
                )
            }
            Self::CorruptInstallation { package, reason } => {
                write!(
                    f,
                    "package '{package}' installation validation failed: {reason}. Run 'crux install --force' to reinstall."
                )
            }
            Self::SecurityViolation { package, reason } => {
                write!(
                    f,
                    "installation security violation in package '{package}': {reason}"
                )
            }
            Self::InvalidPath(msg) => write!(f, "invalid path for CMake integration: {msg}"),
            Self::ManifestError(msg) => write!(f, "manifest error: {msg}"),
            Self::Io(err) => write!(f, "I/O error during CMake integration: {err}"),
        }
    }
}

impl std::error::Error for IntegrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for IntegrationError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
