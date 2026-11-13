//! Package manifest definitions, validation, parsing, and serialization.

pub use crate::dependency::{Dependency, DependencyError};
pub use crate::package::{
    NameValidationError, PackageId, PackageIdError, PackageMetadata, PackageName,
    validate_package_name,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// The canonical manifest file name used by Crux projects.
pub const MANIFEST_FILENAME: &str = "crux.toml";

/// Errors that can occur when validating manifest semantic constraints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestValidationError {
    InvalidPackageName(NameValidationError),
    InvalidVersion(String),
    InvalidDependencyName {
        name: String,
        error: NameValidationError,
    },
    InvalidDependencyVersionReq {
        name: String,
        error: String,
    },
    EmptyDependencyVersionReq {
        name: String,
    },
}

impl fmt::Display for ManifestValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestValidationError::InvalidPackageName(err) => {
                write!(f, "invalid package name: {err}")
            }
            ManifestValidationError::InvalidVersion(err) => {
                write!(f, "{err}")
            }
            ManifestValidationError::InvalidDependencyName { name, error } => {
                write!(f, "invalid dependency name '{name}': {error}")
            }
            ManifestValidationError::InvalidDependencyVersionReq { name, error } => {
                write!(
                    f,
                    "invalid version requirement for dependency '{name}': {error}"
                )
            }
            ManifestValidationError::EmptyDependencyVersionReq { name } => {
                write!(
                    f,
                    "version requirement for dependency '{name}' cannot be empty"
                )
            }
        }
    }
}

impl std::error::Error for ManifestValidationError {}

impl From<NameValidationError> for ManifestValidationError {
    fn from(err: NameValidationError) -> Self {
        ManifestValidationError::InvalidPackageName(err)
    }
}

/// Errors encountered when loading, parsing, or validating a manifest.
#[derive(Debug)]
pub enum ManifestError {
    Io(io::Error),
    Toml(toml::de::Error),
    Validation(ManifestValidationError),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Io(err) => write!(f, "failed to read manifest file: {err}"),
            ManifestError::Toml(err) => write!(f, "failed to parse manifest TOML: {err}"),
            ManifestError::Validation(err) => write!(f, "manifest validation error: {err}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<io::Error> for ManifestError {
    fn from(err: io::Error) -> Self {
        ManifestError::Io(err)
    }
}

impl From<toml::de::Error> for ManifestError {
    fn from(err: toml::de::Error) -> Self {
        ManifestError::Toml(err)
    }
}

impl From<ManifestValidationError> for ManifestError {
    fn from(err: ManifestValidationError) -> Self {
        ManifestError::Validation(err)
    }
}

/// Raw representation used during initial TOML deserialization.
///
/// Permits explicit semantic validation error reporting for package names, SemVer strings,
/// and dependency requirements.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackageMetadata {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    package: RawPackageMetadata,
    #[serde(default)]
    dependencies: Option<BTreeMap<String, toml::Value>>,
}

/// Represents the top-level Crux manifest (`crux.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub package: PackageMetadata,
    pub dependencies: Vec<Dependency>,
}

impl Manifest {
    /// Creates a new manifest with initial default values for a given valid package name.
    pub fn new(name: impl Into<String>) -> Result<Self, NameValidationError> {
        let name = PackageName::new(name)?;
        let version = semver::Version::new(0, 1, 0);

        Ok(Self {
            package: PackageMetadata::new(name, version).with_description(""),
            dependencies: Vec::new(),
        })
    }

    /// Returns the declared dependencies.
    pub fn dependencies(&self) -> &[Dependency] {
        &self.dependencies
    }

    /// Adds a dependency to the manifest, maintaining alphabetical sorting by name.
    pub fn add_dependency(&mut self, dep: Dependency) {
        self.dependencies.push(dep);
        self.dependencies.sort();
    }

    /// Looks up a dependency by package name.
    pub fn get_dependency(&self, name: &str) -> Option<&Dependency> {
        self.dependencies.iter().find(|d| d.name().as_str() == name)
    }

    /// Validates the semantic constraints of the manifest.
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        validate_package_name(&self.package.name)
            .map_err(ManifestValidationError::InvalidPackageName)?;

        for dep in &self.dependencies {
            validate_package_name(dep.name()).map_err(|err| {
                ManifestValidationError::InvalidDependencyName {
                    name: dep.name().to_string(),
                    error: err,
                }
            })?;
        }

        Ok(())
    }

    /// Parses a manifest from a TOML string and verifies its semantic validity.
    pub fn from_toml_str(content: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest = toml::from_str(content).map_err(ManifestError::Toml)?;

        let name = PackageName::new(&raw.package.name)
            .map_err(ManifestValidationError::InvalidPackageName)?;

        let version = semver::Version::parse(&raw.package.version).map_err(|err| {
            ManifestValidationError::InvalidVersion(format!(
                "invalid semantic version '{}': {err}",
                raw.package.version
            ))
        })?;

        let mut package = PackageMetadata::new(name, version);
        if let Some(desc) = raw.package.description {
            package = package.with_description(desc);
        }
        if let Some(lic) = raw.package.license {
            package = package.with_license(lic);
        }
        if let Some(hp) = raw.package.homepage {
            package = package.with_homepage(hp);
        }
        if let Some(repo) = raw.package.repository {
            package = package.with_repository(repo);
        }

        let mut dependencies = Vec::new();
        if let Some(raw_deps) = raw.dependencies {
            for (dep_name, dep_val) in raw_deps {
                let pkg_name = PackageName::new(&dep_name).map_err(|err| {
                    ManifestValidationError::InvalidDependencyName {
                        name: dep_name.clone(),
                        error: err,
                    }
                })?;

                let req_str = match dep_val {
                    toml::Value::String(s) => s,
                    other => {
                        return Err(ManifestError::Validation(
                            ManifestValidationError::InvalidDependencyVersionReq {
                                name: dep_name,
                                error: format!(
                                    "expected string version requirement, found {other}"
                                ),
                            },
                        ));
                    }
                };

                let trimmed = req_str.trim();
                if trimmed.is_empty() {
                    return Err(ManifestError::Validation(
                        ManifestValidationError::EmptyDependencyVersionReq { name: dep_name },
                    ));
                }

                let version_req = semver::VersionReq::parse(trimmed).map_err(|err| {
                    ManifestError::Validation(
                        ManifestValidationError::InvalidDependencyVersionReq {
                            name: dep_name,
                            error: format!(
                                "invalid semantic version requirement '{trimmed}': {err}"
                            ),
                        },
                    )
                })?;

                dependencies.push(Dependency::new(pkg_name, version_req));
            }
        }

        dependencies.sort();

        Ok(Manifest {
            package,
            dependencies,
        })
    }

    /// Reads and parses a manifest file from disk.
    pub fn load_from_path(path: &Path) -> Result<Self, ManifestError> {
        let content = fs::read_to_string(path).map_err(ManifestError::Io)?;
        Self::from_toml_str(&content)
    }

    /// Serializes the manifest into valid TOML format.
    pub fn to_toml_string(&self) -> String {
        let mut out = format!(
            "[package]\nname = \"{}\"\nversion = \"{}\"\n",
            self.package.name, self.package.version
        );
        if let Some(desc) = &self.package.description {
            out.push_str(&format!("description = \"{desc}\"\n"));
        }
        if let Some(lic) = &self.package.license {
            out.push_str(&format!("license = \"{lic}\"\n"));
        }
        if let Some(hp) = &self.package.homepage {
            out.push_str(&format!("homepage = \"{hp}\"\n"));
        }
        if let Some(repo) = &self.package.repository {
            out.push_str(&format!("repository = \"{repo}\"\n"));
        }

        if !self.dependencies.is_empty() {
            out.push_str("\n[dependencies]\n");
            let mut sorted_deps = self.dependencies.clone();
            sorted_deps.sort();
            for dep in sorted_deps {
                out.push_str(&format!("{} = \"{}\"\n", dep.name(), dep.version_req()));
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_manifest_parsing() {
        let toml_str = r#"
[package]
name = "my-project"
version = "1.2.3"
description = "A test project"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.package.name.as_str(), "my-project");
        assert_eq!(manifest.package.version.to_string(), "1.2.3");
        assert_eq!(
            manifest.package.description.as_deref(),
            Some("A test project")
        );
        assert_eq!(manifest.package.id().to_string(), "my-project 1.2.3");
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_valid_manifest_optional_description() {
        let toml_str = r#"
[package]
name = "no-desc"
version = "0.1.0"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.package.description, None);
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_valid_manifest_all_optional_fields() {
        let toml_str = r#"
[package]
name = "full-metadata"
version = "2.0.0-rc.1"
description = "Full metadata example"
license = "MIT OR Apache-2.0"
homepage = "https://crux-pm.org"
repository = "https://github.com/crux/full-metadata"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.package.name.as_str(), "full-metadata");
        assert_eq!(manifest.package.version.to_string(), "2.0.0-rc.1");
        assert_eq!(
            manifest.package.description.as_deref(),
            Some("Full metadata example")
        );
        assert_eq!(
            manifest.package.license.as_deref(),
            Some("MIT OR Apache-2.0")
        );
        assert_eq!(
            manifest.package.homepage.as_deref(),
            Some("https://crux-pm.org")
        );
        assert_eq!(
            manifest.package.repository.as_deref(),
            Some("https://github.com/crux/full-metadata")
        );
        assert_eq!(
            manifest.package.id().to_string(),
            "full-metadata 2.0.0-rc.1"
        );
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_manifest_with_dependencies() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"
description = "My C++ application"

[dependencies]
fmt = "^11.1"
zlib = ">=1.2, <2.0"
openssl = "~3.4"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.dependencies.len(), 3);

        // Dependencies are sorted alphabetically by package name
        assert_eq!(manifest.dependencies[0].name().as_str(), "fmt");
        assert_eq!(manifest.dependencies[0].version_req().to_string(), "^11.1");

        assert_eq!(manifest.dependencies[1].name().as_str(), "openssl");
        assert_eq!(manifest.dependencies[1].version_req().to_string(), "~3.4");

        assert_eq!(manifest.dependencies[2].name().as_str(), "zlib");
        assert_eq!(
            manifest.dependencies[2].version_req().to_string(),
            ">=1.2, <2.0"
        );

        // Serialization roundtrip
        let serialized = manifest.to_toml_string();
        let parsed_again = Manifest::from_toml_str(&serialized).unwrap();
        assert_eq!(parsed_again, manifest);
    }

    #[test]
    fn test_manifest_with_empty_dependencies_section() {
        let toml_str = r#"
[package]
name = "empty-deps"
version = "0.1.0"

[dependencies]
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_manifest_rejects_invalid_dependency_name() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
"-invalid-dep" = "^1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidDependencyName { .. }
            ))
        ));
    }

    #[test]
    fn test_manifest_rejects_empty_dependency_version_req() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = ""
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::EmptyDependencyVersionReq { .. }
            ))
        ));
    }

    #[test]
    fn test_manifest_rejects_invalid_dependency_version_req() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = "1.0.0.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidDependencyVersionReq { .. }
            ))
        ));
    }

    #[test]
    fn test_manifest_rejects_malformed_dependency_type() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = 123
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidDependencyVersionReq { .. }
            ))
        ));
    }

    #[test]
    fn test_invalid_toml_syntax() {
        let toml_str = "not valid toml ::::";
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_missing_package_section() {
        let toml_str = r#"
[other]
name = "no-package-section"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_missing_required_field_name() {
        let toml_str = r#"
[package]
version = "0.1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_invalid_package_name_in_manifest() {
        let toml_str = r#"
[package]
name = "invalid name with spaces"
version = "0.1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidPackageName(_)
            ))
        ));
    }

    #[test]
    fn test_invalid_semantic_version() {
        let toml_str = r#"
[package]
name = "bad-ver"
version = "1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidVersion(_)
            ))
        ));
    }

    #[test]
    fn test_rejects_unknown_fields_in_package() {
        let toml_str = r#"
[package]
name = "test-pkg"
version = "0.1.0"
unknown_field = "unexpected"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_rejects_unknown_top_level_section() {
        let toml_str = r#"
[package]
name = "test-pkg"
version = "0.1.0"

[unexpected_section]
foo = "bar"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }
}
