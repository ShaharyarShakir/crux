//! Package metadata model and serialization.

use super::identity::{PackageId, PackageName};
use serde::{Deserialize, Serialize};

/// Reusable Rust model representing package metadata.
///
/// Contains identity information (name, version) and descriptive metadata (description,
/// license, homepage, repository).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadata {
    pub name: PackageName,
    pub version: semver::Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
}

impl PackageMetadata {
    /// Creates a minimal `PackageMetadata` with required fields.
    pub fn new(name: PackageName, version: semver::Version) -> Self {
        Self {
            name,
            version,
            description: None,
            license: None,
            homepage: None,
            repository: None,
        }
    }

    /// Returns the unique `PackageId` for this metadata.
    pub fn id(&self) -> PackageId {
        PackageId::new(self.name.clone(), self.version.clone())
    }

    /// Returns the package name.
    pub fn name(&self) -> &PackageName {
        &self.name
    }

    /// Returns the semantic version.
    pub fn version(&self) -> &semver::Version {
        &self.version
    }

    /// Returns the package description, if set.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the package license, if set.
    pub fn license(&self) -> Option<&str> {
        self.license.as_deref()
    }

    /// Returns the package homepage URL, if set.
    pub fn homepage(&self) -> Option<&str> {
        self.homepage.as_deref()
    }

    /// Returns the package source repository URL, if set.
    pub fn repository(&self) -> Option<&str> {
        self.repository.as_deref()
    }

    /// Sets or updates the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Sets or updates the license identifier.
    pub fn with_license(mut self, license: impl Into<String>) -> Self {
        self.license = Some(license.into());
        self
    }

    /// Sets or updates the homepage URL.
    pub fn with_homepage(mut self, homepage: impl Into<String>) -> Self {
        self.homepage = Some(homepage.into());
        self
    }

    /// Sets or updates the repository URL.
    pub fn with_repository(mut self, repository: impl Into<String>) -> Self {
        self.repository = Some(repository.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_metadata_model_with_optional_fields() {
        let name = PackageName::new("openssl").unwrap();
        let version = semver::Version::parse("3.4.0").unwrap();

        let meta = PackageMetadata::new(name, version)
            .with_description("TLS/SSL and crypto library")
            .with_license("Apache-2.0")
            .with_homepage("https://www.openssl.org")
            .with_repository("https://github.com/openssl/openssl");

        assert_eq!(meta.name().as_str(), "openssl");
        assert_eq!(meta.version().to_string(), "3.4.0");
        assert_eq!(meta.description(), Some("TLS/SSL and crypto library"));
        assert_eq!(meta.license(), Some("Apache-2.0"));
        assert_eq!(meta.homepage(), Some("https://www.openssl.org"));
        assert_eq!(
            meta.repository(),
            Some("https://github.com/openssl/openssl")
        );

        let id = meta.id();
        assert_eq!(id.to_string(), "openssl 3.4.0");
    }

    #[test]
    fn test_package_metadata_serialization_roundtrip_toml() {
        let name = PackageName::new("fmt").unwrap();
        let version = semver::Version::parse("11.1.1").unwrap();
        let meta = PackageMetadata::new(name, version)
            .with_description("A modern formatting library")
            .with_license("MIT")
            .with_repository("https://github.com/fmtlib/fmt");

        let toml_str = toml::to_string(&meta).unwrap();
        assert!(toml_str.contains("name = \"fmt\""));
        assert!(toml_str.contains("version = \"11.1.1\""));
        assert!(toml_str.contains("description = \"A modern formatting library\""));
        assert!(toml_str.contains("license = \"MIT\""));
        assert!(toml_str.contains("repository = \"https://github.com/fmtlib/fmt\""));
        assert!(!toml_str.contains("homepage"));

        let deserialized: PackageMetadata = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized, meta);
    }

    #[test]
    fn test_package_metadata_serialization_roundtrip_minimal() {
        let name = PackageName::new("zlib").unwrap();
        let version = semver::Version::parse("1.3.1").unwrap();
        let meta = PackageMetadata::new(name, version);

        let toml_str = toml::to_string(&meta).unwrap();
        assert!(toml_str.contains("name = \"zlib\""));
        assert!(toml_str.contains("version = \"1.3.1\""));
        assert!(!toml_str.contains("description"));
        assert!(!toml_str.contains("license"));
        assert!(!toml_str.contains("homepage"));
        assert!(!toml_str.contains("repository"));

        let deserialized: PackageMetadata = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized, meta);
        assert_eq!(deserialized.id().to_string(), "zlib 1.3.1");
    }

    #[test]
    fn test_package_metadata_deserialization_rejects_unknown_fields() {
        let toml_str = r#"
name = "pkg"
version = "1.0.0"
unknown_custom_field = "invalid"
"#;
        let res: Result<PackageMetadata, _> = toml::from_str(toml_str);
        assert!(res.is_err());
    }

    #[test]
    fn test_package_metadata_deserialization_rejects_invalid_name() {
        let toml_str = r#"
name = "-invalid-name"
version = "1.0.0"
"#;
        let res: Result<PackageMetadata, _> = toml::from_str(toml_str);
        assert!(res.is_err());
    }

    #[test]
    fn test_package_metadata_deserialization_rejects_invalid_version() {
        let toml_str = r#"
name = "pkg"
version = "not-a-semver"
"#;
        let res: Result<PackageMetadata, _> = toml::from_str(toml_str);
        assert!(res.is_err());
    }
}
