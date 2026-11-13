//! Dependency requirements and specification models.

use crate::lock::PackageSource;
use crate::package::{NameValidationError, PackageName};
use std::fmt;

/// Errors that can occur when defining or validating a dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyError {
    InvalidName(NameValidationError),
    EmptyVersionReq,
    InvalidVersionReq(String),
}

impl fmt::Display for DependencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyError::InvalidName(err) => {
                write!(f, "invalid dependency package name: {err}")
            }
            DependencyError::EmptyVersionReq => {
                write!(f, "dependency version requirement cannot be empty")
            }
            DependencyError::InvalidVersionReq(err) => {
                write!(f, "invalid semantic version requirement: {err}")
            }
        }
    }
}

impl std::error::Error for DependencyError {}

impl From<NameValidationError> for DependencyError {
    fn from(err: NameValidationError) -> Self {
        DependencyError::InvalidName(err)
    }
}

/// Represents a declared package dependency requirement.
///
/// Encapsulates the target package name and the SemVer version requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    name: PackageName,
    version_req: semver::VersionReq,
    source: Option<PackageSource>,
}

impl Dependency {
    /// Creates a new dependency with default registry source.
    pub fn new(name: PackageName, version_req: semver::VersionReq) -> Self {
        Self {
            name,
            version_req,
            source: None,
        }
    }

    /// Creates and validates a new dependency from name and version requirement strings.
    pub fn try_new(name: impl AsRef<str>, req: impl AsRef<str>) -> Result<Self, DependencyError> {
        let name_str = name.as_ref();
        let req_str = req.as_ref();

        let pkg_name = PackageName::new(name_str).map_err(DependencyError::InvalidName)?;

        let trimmed_req = req_str.trim();
        if trimmed_req.is_empty() {
            return Err(DependencyError::EmptyVersionReq);
        }

        let version_req = semver::VersionReq::parse(trimmed_req)
            .map_err(|err| DependencyError::InvalidVersionReq(format!("'{trimmed_req}': {err}")))?;

        Ok(Self::new(pkg_name, version_req))
    }

    /// Sets or updates the dependency source.
    pub fn with_source(mut self, source: PackageSource) -> Self {
        self.source = Some(source);
        self
    }

    /// Returns the dependency package name.
    pub fn name(&self) -> &PackageName {
        &self.name
    }

    /// Returns the semantic version requirement.
    pub fn version_req(&self) -> &semver::VersionReq {
        &self.version_req
    }

    /// Returns the dependency source, if explicitly specified.
    pub fn source(&self) -> Option<&PackageSource> {
        self.source.as_ref()
    }

    /// Checks if a concrete semantic version satisfies this dependency requirement.
    pub fn matches(&self, version: &semver::Version) -> bool {
        self.version_req.matches(version)
    }
}

impl fmt::Display for Dependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} = \"{}\"", self.name, self.version_req)
    }
}

impl PartialOrd for Dependency {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Dependency {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name).then_with(|| {
            self.version_req
                .to_string()
                .cmp(&other.version_req.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_dependency_creation() {
        let dep = Dependency::try_new("fmt", "^11.1").unwrap();
        assert_eq!(dep.name().as_str(), "fmt");
        assert_eq!(dep.version_req().to_string(), "^11.1");

        let v_match = semver::Version::parse("11.1.1").unwrap();
        let v_nomatch = semver::Version::parse("12.0.0").unwrap();
        assert!(dep.matches(&v_match));
        assert!(!dep.matches(&v_nomatch));
    }

    #[test]
    fn test_comparator_and_tilde_requirements() {
        let dep_comp = Dependency::try_new("zlib", ">=1.2, <2.0").unwrap();
        assert!(dep_comp.matches(&semver::Version::parse("1.3.1").unwrap()));
        assert!(!dep_comp.matches(&semver::Version::parse("2.0.0").unwrap()));

        let dep_tilde = Dependency::try_new("openssl", "~3.4").unwrap();
        assert!(dep_tilde.matches(&semver::Version::parse("3.4.1").unwrap()));
        assert!(!dep_tilde.matches(&semver::Version::parse("3.5.0").unwrap()));
    }

    #[test]
    fn test_invalid_dependency_name() {
        assert!(matches!(
            Dependency::try_new("-invalid", "^1.0"),
            Err(DependencyError::InvalidName(_))
        ));
        assert!(matches!(
            Dependency::try_new("", "^1.0"),
            Err(DependencyError::InvalidName(_))
        ));
    }

    #[test]
    fn test_empty_and_invalid_version_requirements() {
        assert!(matches!(
            Dependency::try_new("fmt", ""),
            Err(DependencyError::EmptyVersionReq)
        ));
        assert!(matches!(
            Dependency::try_new("fmt", "   "),
            Err(DependencyError::EmptyVersionReq)
        ));
        assert!(matches!(
            Dependency::try_new("fmt", "1.0.0.0"),
            Err(DependencyError::InvalidVersionReq(_))
        ));
        assert!(matches!(
            Dependency::try_new("fmt", "not-a-version"),
            Err(DependencyError::InvalidVersionReq(_))
        ));
    }

    #[test]
    fn test_dependency_ordering() {
        let dep_a = Dependency::try_new("fmt", "^11.0").unwrap();
        let dep_b = Dependency::try_new("zlib", "^1.0").unwrap();
        assert!(dep_a < dep_b);
    }
}
