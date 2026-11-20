//! Package registry metadata, manifests, and definition models.

use crate::dependency::Dependency;
use crate::lock::Checksum;
use crate::package::PackageMetadata;
use crate::resolution::{GitRef, PackageSource};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Standard file name for registry manifest.
pub const REGISTRY_MANIFEST_FILENAME: &str = "registry.toml";

/// Standard file name for package definitions.
pub const PACKAGE_DEFINITION_FILENAME: &str = "package.toml";

/// Supported registry format version.
pub const CURRENT_REGISTRY_FORMAT_VERSION: u32 = 1;

/// Registry manifest located at `registry.toml` in the registry root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryManifest {
    pub format_version: u32,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Represents the `[source]` section of a `package.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSection {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default, rename = "rev")]
    pub rev: Option<String>,
}

/// Represents the `[build]` section of a `package.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSection {
    pub system: String,
    /// Deterministic CMake options applied when building this package.
    #[serde(default)]
    pub cmake_args: Vec<String>,
}

/// Raw deserialization model for `package.toml`.
///
/// The `[source]` section is required in registry definitions but optional for
/// direct sources, where the dependency declaration is the authoritative source
/// identity. Dependencies accept either SemVer strings or source inline tables.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPackageDefinition {
    pub package: RawPackageMetadataSection,
    #[serde(default)]
    pub source: Option<SourceSection>,
    #[serde(default)]
    pub build: Option<BuildSection>,
    #[serde(default)]
    pub dependencies: Option<BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPackageMetadataSection {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
}

/// A validated package definition loaded from a registry or direct source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDefinition {
    pub metadata: PackageMetadata,
    pub source: PackageSourceDefinition,
    pub build: Option<BuildSection>,
    pub dependencies: Vec<Dependency>,
}

/// Validated source definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageSourceDefinition {
    Archive {
        url: String,
        checksum: Checksum,
    },
    Git {
        url: String,
        requested: GitRef,
        resolved_commit: Option<String>,
    },
    Path {
        path: String,
    },
}

impl PackageSourceDefinition {
    /// Converts a canonical locked source identity into a source definition.
    pub fn from_package_source(source: &PackageSource) -> Result<Self, String> {
        let direct = source
            .direct()
            .ok_or_else(|| format!("source '{source}' is not a direct source"))?;
        Ok(match direct {
            crate::resolution::DirectSource::Git {
                url,
                requested,
                resolved_commit,
            } => Self::Git {
                url,
                requested,
                resolved_commit,
            },
            crate::resolution::DirectSource::Archive { url, checksum } => {
                Self::Archive { url, checksum }
            }
            crate::resolution::DirectSource::Path { path } => Self::Path { path },
        })
    }
}

/// Validates and converts a `package.toml` document into a package definition.
///
/// `source_override` is used for direct sources, where the source identity comes
/// from the dependency declaration rather than the package metadata.
pub fn parse_package_definition(
    text: &str,
    origin: &std::path::Path,
    source_override: Option<PackageSourceDefinition>,
) -> Result<PackageDefinition, crate::registry::RegistryError> {
    use crate::registry::RegistryError;

    let raw: RawPackageDefinition = toml::from_str(text)
        .map_err(|e| RegistryError::Toml(format!("failed to parse '{}': {e}", origin.display())))?;

    let name = crate::package::PackageName::new(&raw.package.name).map_err(|e| {
        RegistryError::InvalidPackageName(format!(
            "invalid package name '{}' in '{}': {e}",
            raw.package.name,
            origin.display()
        ))
    })?;
    let version = semver::Version::parse(&raw.package.version).map_err(|e| {
        RegistryError::InvalidVersion(format!(
            "invalid version '{}' in '{}': {e}",
            raw.package.version,
            origin.display()
        ))
    })?;

    let metadata = PackageMetadata {
        name: name.clone(),
        version: version.clone(),
        description: raw.package.description,
        license: raw.package.license,
        homepage: raw.package.homepage,
        repository: raw.package.repository,
    };

    let source = match source_override {
        Some(source) => source,
        None => {
            let section = raw.source.ok_or_else(|| {
                RegistryError::Validation(format!(
                    "package definition '{}' is missing a [source] section",
                    origin.display()
                ))
            })?;
            match section.source_type.as_str() {
                "archive" => {
                    let checksum_str = section.checksum.ok_or_else(|| {
                        RegistryError::Validation(format!(
                            "package '{} {version}' has source type 'archive' but lacks a required checksum",
                            name
                        ))
                    })?;
                    let checksum = Checksum::parse(&checksum_str).map_err(|e| {
                        RegistryError::Validation(format!(
                            "invalid checksum in '{} {version}': {e}",
                            name
                        ))
                    })?;
                    PackageSourceDefinition::Archive {
                        url: section.url,
                        checksum,
                    }
                }
                "git" => {
                    let requested = match section.rev {
                        Some(rev) => GitRef::Rev(rev),
                        None => GitRef::Head,
                    };
                    PackageSourceDefinition::Git {
                        url: section.url,
                        requested,
                        resolved_commit: None,
                    }
                }
                other => {
                    return Err(RegistryError::Validation(format!(
                        "unsupported source type '{other}' in '{} {version}'",
                        name
                    )));
                }
            }
        }
    };

    let mut dependencies = Vec::new();
    if let Some(deps_map) = raw.dependencies {
        for (dep_name, value) in deps_map {
            let dep = crate::manifest::parse_dependency_value(&dep_name, value).map_err(|e| {
                RegistryError::Validation(format!(
                    "invalid dependency '{dep_name}' in '{} {version}': {e}",
                    name
                ))
            })?;
            dependencies.push(dep);
        }
        dependencies.sort();
    }

    let build = match raw.build {
        Some(mut build) => {
            for arg in &build.cmake_args {
                if arg.is_empty()
                    || !arg.starts_with('-')
                    || arg.chars().any(|c| c == '\0' || c == '\n' || c == '\r')
                {
                    return Err(RegistryError::Validation(format!(
                        "invalid CMake argument '{arg}' in '{} {version}'",
                        name
                    )));
                }
            }
            build.cmake_args.sort();
            Some(build)
        }
        None => None,
    };

    Ok(PackageDefinition {
        metadata,
        source,
        build,
        dependencies,
    })
}
