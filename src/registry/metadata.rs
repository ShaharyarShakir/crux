//! Package registry metadata, manifests, and definition models.

use crate::dependency::Dependency;
use crate::lock::Checksum;
use crate::package::PackageMetadata;
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
    pub url: String,
    #[serde(default)]
    pub checksum: Option<String>,
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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPackageDefinition {
    pub package: RawPackageMetadataSection,
    pub source: SourceSection,
    #[serde(default)]
    pub build: Option<BuildSection>,
    #[serde(default)]
    pub dependencies: Option<BTreeMap<String, String>>,
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

/// A validated package definition loaded from a registry's `package.toml`.
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
    Archive { url: String, checksum: Checksum },
    Git { url: String, rev: Option<String> },
}
