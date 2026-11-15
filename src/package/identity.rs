//! Package identity and semantic versioning definitions.

use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// Errors that can occur when validating a package name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameValidationError {
    Empty,
    TooLong(usize),
    StartsWithHyphen,
    InvalidCharacter(char),
    ReservedName(String),
}

impl fmt::Display for NameValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NameValidationError::Empty => write!(f, "package name cannot be empty"),
            NameValidationError::TooLong(len) => {
                write!(
                    f,
                    "package name is too long ({len} characters; max allowed is 128)"
                )
            }
            NameValidationError::StartsWithHyphen => {
                write!(f, "package name cannot start with a hyphen ('-')")
            }
            NameValidationError::InvalidCharacter(c) => {
                write!(
                    f,
                    "package name contains invalid character '{c}'; only ASCII alphanumeric, hyphens, and underscores are allowed"
                )
            }
            NameValidationError::ReservedName(name) => {
                write!(f, "package name '{name}' is a reserved system identifier")
            }
        }
    }
}

impl std::error::Error for NameValidationError {}

/// List of reserved names on common platforms to prevent system conflicts.
pub const RESERVED_NAMES: &[&str] = &[
    ".", "..", "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Validates that a string conforms to Crux package naming conventions.
pub fn validate_package_name(name: &str) -> Result<(), NameValidationError> {
    if name.is_empty() {
        return Err(NameValidationError::Empty);
    }

    if name.len() > 128 {
        return Err(NameValidationError::TooLong(name.len()));
    }

    let lower = name.to_ascii_lowercase();
    if RESERVED_NAMES.contains(&lower.as_str()) {
        return Err(NameValidationError::ReservedName(name.to_string()));
    }

    let mut chars = name.chars();
    let first = chars.next().unwrap();

    if first == '-' {
        return Err(NameValidationError::StartsWithHyphen);
    }

    if !first.is_ascii_alphanumeric() && first != '_' {
        return Err(NameValidationError::InvalidCharacter(first));
    }

    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '-' && c != '_' {
            return Err(NameValidationError::InvalidCharacter(c));
        }
    }

    Ok(())
}

/// A validated Crux package name.
///
/// Encapsulates naming constraints ensuring valid characters, length, and non-reserved identifiers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageName(String);

impl PackageName {
    /// Constructs and validates a new `PackageName`.
    pub fn new(name: impl Into<String>) -> Result<Self, NameValidationError> {
        let name = name.into();
        validate_package_name(&name)?;
        Ok(Self(name))
    }

    /// Returns a string slice representing the package name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the underlying `String`.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::ops::Deref for PackageName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for PackageName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for PackageName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PackageName {
    type Err = NameValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for PackageName {
    type Error = NameValidationError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl TryFrom<String> for PackageName {
    type Error = NameValidationError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl PartialEq<str> for PackageName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for PackageName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for PackageName {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

impl Serialize for PackageName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PackageName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PackageNameVisitor;

        impl Visitor<'_> for PackageNameVisitor {
            type Value = PackageName;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a valid Crux package name string")
            }

            fn visit_str<E>(self, value: &str) -> Result<PackageName, E>
            where
                E: serde::de::Error,
            {
                PackageName::new(value).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(PackageNameVisitor)
    }
}

/// Errors that can occur when parsing a `PackageId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageIdError {
    InvalidFormat(String),
    InvalidName(NameValidationError),
    InvalidVersion(String),
}

impl fmt::Display for PackageIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageIdError::InvalidFormat(msg) => {
                write!(f, "invalid package identity format: {msg}")
            }
            PackageIdError::InvalidName(err) => {
                write!(f, "invalid package name in identity: {err}")
            }
            PackageIdError::InvalidVersion(err) => write!(f, "invalid version in identity: {err}"),
        }
    }
}

impl std::error::Error for PackageIdError {}

impl From<NameValidationError> for PackageIdError {
    fn from(err: NameValidationError) -> Self {
        PackageIdError::InvalidName(err)
    }
}

/// Unique package identity consisting of a package name and semantic version.
///
/// A package identity uniquely identifies a package within a registry and dependency graph.
/// It is decoupled from project-specific filesystem paths or build outputs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageId {
    name: PackageName,
    version: semver::Version,
}

impl PackageId {
    /// Creates a new `PackageId` from a validated `PackageName` and `semver::Version`.
    pub fn new(name: PackageName, version: semver::Version) -> Self {
        Self { name, version }
    }

    /// Convenience constructor that parses and validates string inputs.
    pub fn try_new(
        name: impl AsRef<str>,
        version: impl AsRef<str>,
    ) -> Result<Self, PackageIdError> {
        let pkg_name = PackageName::new(name.as_ref())?;
        let ver = semver::Version::parse(version.as_ref()).map_err(|err| {
            PackageIdError::InvalidVersion(format!(
                "invalid semantic version '{}': {err}",
                version.as_ref()
            ))
        })?;
        Ok(Self::new(pkg_name, ver))
    }

    /// Returns the package name.
    pub fn name(&self) -> &PackageName {
        &self.name
    }

    /// Returns the package semantic version.
    pub fn version(&self) -> &semver::Version {
        &self.version
    }

    /// Compares version precedence with another `PackageId` according to SemVer 2.0.0.
    ///
    /// If package names differ, names are compared lexicographically.
    /// If package names are equal, versions are compared strictly by SemVer precedence,
    /// ignoring build metadata (Clause 10 of SemVer 2.0.0).
    pub fn cmp_precedence(&self, other: &Self) -> Ordering {
        match self.name.cmp(&other.name) {
            Ordering::Equal => self.version.cmp_precedence(&other.version),
            ord => ord,
        }
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.name, self.version)
    }
}

impl FromStr for PackageId {
    type Err = PackageIdError;

    /// Parses a package identity from `<name> <version>` format (e.g. `zlib 1.3.1`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 2 {
            return Err(PackageIdError::InvalidFormat(format!(
                "expected '<name> <version>', found '{s}'"
            )));
        }
        Self::try_new(parts[0], parts[1])
    }
}

impl PartialOrd for PackageId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PackageId {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.name.cmp(&other.name) {
            Ordering::Equal => self.version.cmp(&other.version),
            ord => ord,
        }
    }
}

impl Serialize for PackageId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PackageId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PackageIdVisitor;

        impl Visitor<'_> for PackageIdVisitor {
            type Value = PackageId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a package identity formatted as '<name> <version>'")
            }

            fn visit_str<E>(self, value: &str) -> Result<PackageId, E>
            where
                E: serde::de::Error,
            {
                PackageId::from_str(value).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(PackageIdVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_package_name() {
        assert!(PackageName::new("zlib").is_ok());
        assert!(PackageName::new("fmt").is_ok());
        assert!(PackageName::new("openssl").is_ok());
        assert!(PackageName::new("OpenSSL").is_ok());
        assert!(PackageName::new("boost_math").is_ok());
        assert!(PackageName::new("lib-png-123").is_ok());
        assert!(PackageName::new("_private").is_ok());
    }

    #[test]
    fn test_invalid_package_names() {
        assert_eq!(PackageName::new(""), Err(NameValidationError::Empty));
        assert_eq!(
            PackageName::new("-starts-with-hyphen"),
            Err(NameValidationError::StartsWithHyphen)
        );
        assert_eq!(
            PackageName::new("has space"),
            Err(NameValidationError::InvalidCharacter(' '))
        );
        assert_eq!(
            PackageName::new("has/slash"),
            Err(NameValidationError::InvalidCharacter('/'))
        );
        assert_eq!(
            PackageName::new("has@at"),
            Err(NameValidationError::InvalidCharacter('@'))
        );
        assert_eq!(
            PackageName::new("con"),
            Err(NameValidationError::ReservedName("con".to_string()))
        );
        assert_eq!(
            PackageName::new("NUL"),
            Err(NameValidationError::ReservedName("NUL".to_string()))
        );
        assert_eq!(
            PackageName::new(".."),
            Err(NameValidationError::ReservedName("..".to_string()))
        );

        let long_name = "a".repeat(129);
        assert_eq!(
            PackageName::new(long_name),
            Err(NameValidationError::TooLong(129))
        );
    }

    #[test]
    fn test_package_id_display_and_parse() {
        let pkg_id = PackageId::try_new("zlib", "1.3.1").unwrap();
        assert_eq!(pkg_id.to_string(), "zlib 1.3.1");

        let parsed: PackageId = "zlib 1.3.1".parse().unwrap();
        assert_eq!(parsed, pkg_id);

        let parsed_fmt: PackageId = "fmt 11.1.1".parse().unwrap();
        assert_eq!(parsed_fmt.name().as_str(), "fmt");
        assert_eq!(parsed_fmt.version().to_string(), "11.1.1");
    }

    #[test]
    fn test_package_id_invalid_formats() {
        assert!(matches!(
            PackageId::from_str("zlib"),
            Err(PackageIdError::InvalidFormat(_))
        ));
        assert!(matches!(
            PackageId::from_str("zlib 1.3.1 extra"),
            Err(PackageIdError::InvalidFormat(_))
        ));
        assert!(matches!(
            PackageId::from_str("invalid/name 1.0.0"),
            Err(PackageIdError::InvalidName(_))
        ));
        assert!(matches!(
            PackageId::from_str("zlib invalid_version"),
            Err(PackageIdError::InvalidVersion(_))
        ));
    }

    #[test]
    fn test_package_id_equality_and_different_names() {
        let id1 = PackageId::try_new("fmt", "11.1.1").unwrap();
        let id2 = PackageId::try_new("fmt", "11.1.1").unwrap();
        let id3 = PackageId::try_new("zlib", "11.1.1").unwrap();

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_package_id_different_versions() {
        let id1 = PackageId::try_new("zlib", "1.2.11").unwrap();
        let id2 = PackageId::try_new("zlib", "1.3.1").unwrap();

        assert_ne!(id1, id2);
        assert_eq!(id1.name(), id2.name());
        assert_eq!(id1.cmp_precedence(&id2), Ordering::Less);
        assert_eq!(id2.cmp_precedence(&id1), Ordering::Greater);
    }

    #[test]
    fn test_semantic_version_ordering() {
        let v1 = semver::Version::parse("1.0.0").unwrap();
        let v2 = semver::Version::parse("1.2.3").unwrap();
        let v3 = semver::Version::parse("2.0.0").unwrap();

        assert!(v1 < v2);
        assert!(v2 < v3);
    }

    #[test]
    fn test_semver_prerelease_ordering() {
        let v_alpha1 = semver::Version::parse("2.0.0-alpha.1").unwrap();
        let v_beta2 = semver::Version::parse("2.0.0-beta.2").unwrap();
        let v_release = semver::Version::parse("2.0.0").unwrap();

        assert_eq!(v_alpha1.cmp_precedence(&v_beta2), Ordering::Less);
        assert_eq!(v_beta2.cmp_precedence(&v_release), Ordering::Less);
        assert_eq!(v_alpha1.cmp_precedence(&v_release), Ordering::Less);
    }

    #[test]
    fn test_build_metadata_behavior() {
        let v_plain = semver::Version::parse("1.0.0").unwrap();
        let v_build42 = semver::Version::parse("1.0.0+build.42").unwrap();
        let v_build99 = semver::Version::parse("1.0.0+build.99").unwrap();

        // SemVer 2.0.0 Clause 10: Build metadata MUST be ignored when determining version precedence
        assert_eq!(v_build42.cmp_precedence(&v_plain), Ordering::Equal);
        assert_eq!(v_build42.cmp_precedence(&v_build99), Ordering::Equal);

        // However, Version equality checks all fields including build metadata
        assert_ne!(v_build42, v_plain);
        assert_ne!(v_build42, v_build99);

        // PackageId precedence also ignores build metadata
        let pkg1 = PackageId::try_new("pkg", "1.0.0+build.42").unwrap();
        let pkg2 = PackageId::try_new("pkg", "1.0.0+build.99").unwrap();
        assert_eq!(pkg1.cmp_precedence(&pkg2), Ordering::Equal);
    }
}
