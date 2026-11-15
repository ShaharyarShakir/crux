//! Git-backed package registry, package definition models, and local registry management.

pub mod config;
pub mod git;
pub mod metadata;

pub use config::{
    CruxConfig, RegistryConfig, RegistryStatus, add_registry, config_file_path,
    init_test_git_registry, list_registries, load_config, registries_dir_path, save_config,
    seed_registry_files, update_registry,
};
pub use git::{GitOps, GitRegistry};
pub use metadata::{
    BuildSection, CURRENT_REGISTRY_FORMAT_VERSION, PACKAGE_DEFINITION_FILENAME, PackageDefinition,
    PackageSourceDefinition, REGISTRY_MANIFEST_FILENAME, RegistryManifest, SourceSection,
};

use crate::package::PackageId;
use std::fmt;
use std::io;
use std::path::PathBuf;

/// Errors related to Git registry operations, loading, or configuration.
#[derive(Debug)]
pub enum RegistryError {
    Io(io::Error),
    Git(String),
    Toml(String),
    NotAGitRepository(PathBuf),
    MissingRegistryManifest(PathBuf),
    UnsupportedFormatVersion(u32),
    InvalidPackageName(String),
    InvalidVersion(String),
    MissingPackageManifest(PathBuf),
    IdentityMismatch {
        expected_name: String,
        found_name: String,
        expected_version: String,
        found_version: String,
    },
    SecurityViolation(String),
    DuplicatePackage(PackageId),
    NotFound(String),
    AlreadyExists(String),
    Config(String),
    Validation(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Io(err) => write!(f, "I/O error: {err}"),
            RegistryError::Git(msg) => write!(f, "Git error: {msg}"),
            RegistryError::Toml(msg) => write!(f, "TOML error: {msg}"),
            RegistryError::NotAGitRepository(path) => {
                write!(
                    f,
                    "directory '{}' is not a valid Git repository",
                    path.display()
                )
            }
            RegistryError::MissingRegistryManifest(path) => {
                write!(
                    f,
                    "missing registry manifest '{}': directory is not a Crux registry",
                    path.display()
                )
            }
            RegistryError::UnsupportedFormatVersion(ver) => {
                write!(
                    f,
                    "unsupported registry format version '{ver}': this version of Crux supports version {CURRENT_REGISTRY_FORMAT_VERSION}"
                )
            }
            RegistryError::InvalidPackageName(msg) => write!(f, "invalid package name: {msg}"),
            RegistryError::InvalidVersion(msg) => write!(f, "invalid version: {msg}"),
            RegistryError::MissingPackageManifest(path) => {
                write!(f, "missing package definition '{}'", path.display())
            }
            RegistryError::IdentityMismatch {
                expected_name,
                found_name,
                expected_version,
                found_version,
            } => {
                write!(
                    f,
                    "package definition identity mismatch: directory path specifies '{expected_name} {expected_version}' but package.toml declares '{found_name} {found_version}'"
                )
            }
            RegistryError::SecurityViolation(msg) => write!(f, "security violation: {msg}"),
            RegistryError::DuplicatePackage(id) => {
                write!(f, "duplicate package identity '{id}' in registry")
            }
            RegistryError::NotFound(name) => write!(f, "registry '{name}' not found"),
            RegistryError::AlreadyExists(name) => {
                write!(f, "registry '{name}' is already configured")
            }
            RegistryError::Config(msg) => write!(f, "configuration error: {msg}"),
            RegistryError::Validation(msg) => write!(f, "validation error: {msg}"),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<io::Error> for RegistryError {
    fn from(err: io::Error) -> Self {
        RegistryError::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::PackageName;
    use crate::resolution::catalog::PackageCatalog;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn test_open_seed_registry() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("registry");
        let reg = GitRegistry::open(&root).expect("failed to open root seed registry");

        assert_eq!(reg.name(), "default");
        assert_eq!(reg.package_count(), 3);
        assert_eq!(reg.total_version_count(), 6);

        let names = reg.package_names();
        let name_strs: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
        assert_eq!(name_strs, vec!["catch2", "fmt", "zlib"]);

        let fmt_versions = reg
            .get_package_versions(&PackageName::new("fmt").unwrap())
            .unwrap();
        assert_eq!(fmt_versions.len(), 2);
        assert_eq!(fmt_versions[0].version.to_string(), "11.0.2");
        assert_eq!(fmt_versions[1].version.to_string(), "11.1.1");
    }

    #[test]
    fn test_not_a_git_repo_fails() {
        let temp = std::env::temp_dir().join(format!("crux_non_git_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        fs::write(
            temp.join("registry.toml"),
            "format_version = 1\nname = \"test\"\n",
        )
        .unwrap();

        let res = GitRegistry::open(&temp);
        assert!(matches!(res, Err(RegistryError::NotAGitRepository(_))));

        let _ = fs::remove_dir_all(&temp);
    }
}
