//! Crux lockfile definitions, deterministic serialization, integrity models, and verification.

use crate::catalog::PackageCatalog;
use crate::manifest::{MANIFEST_FILENAME, Manifest, ManifestError};
use crate::package::{NameValidationError, PackageId, PackageName};
use crate::resolver::{ResolutionError, Resolver};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Standard file name for Crux lockfiles.
pub const LOCKFILE_FILENAME: &str = "crux.lock";

/// Current supported lockfile schema format version.
pub const CURRENT_LOCKFILE_VERSION: u32 = 1;

/// Supported checksum algorithms for package integrity verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChecksumAlgorithm {
    Sha256,
}

impl ChecksumAlgorithm {
    /// Returns the algorithm identifier string (e.g. "sha256").
    pub fn as_str(&self) -> &'static str {
        match self {
            ChecksumAlgorithm::Sha256 => "sha256",
        }
    }
}

impl fmt::Display for ChecksumAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ChecksumAlgorithm {
    type Err = ChecksumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "sha256" => Ok(ChecksumAlgorithm::Sha256),
            other => Err(ChecksumError::UnsupportedAlgorithm(other.to_string())),
        }
    }
}

/// Errors related to checksum parsing and validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksumError {
    InvalidFormat(String),
    UnsupportedAlgorithm(String),
    InvalidHexDigest(String),
}

impl fmt::Display for ChecksumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChecksumError::InvalidFormat(val) => {
                write!(
                    f,
                    "invalid checksum format '{val}': expected '<algorithm>:<hex_digest>'"
                )
            }
            ChecksumError::UnsupportedAlgorithm(algo) => {
                write!(
                    f,
                    "unsupported checksum algorithm '{algo}' (expected 'sha256')"
                )
            }
            ChecksumError::InvalidHexDigest(msg) => {
                write!(f, "invalid checksum hex digest: {msg}")
            }
        }
    }
}

impl std::error::Error for ChecksumError {}

/// Represents an explicit package source archive integrity checksum.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Checksum {
    algorithm: ChecksumAlgorithm,
    digest: String,
}

impl Checksum {
    /// Constructs a new SHA-256 checksum from a 64-character hexadecimal digest.
    pub fn new_sha256(hex_digest: impl AsRef<str>) -> Result<Self, ChecksumError> {
        let digest_str = hex_digest.as_ref();
        if digest_str.len() != 64 {
            return Err(ChecksumError::InvalidHexDigest(format!(
                "expected exactly 64 hex characters for SHA-256, found length {}",
                digest_str.len()
            )));
        }

        if !digest_str.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ChecksumError::InvalidHexDigest(format!(
                "contains non-hexadecimal characters in '{digest_str}'"
            )));
        }

        Ok(Self {
            algorithm: ChecksumAlgorithm::Sha256,
            digest: digest_str.to_ascii_lowercase(),
        })
    }

    /// Parses a checksum string formatted as `<algorithm>:<hex_digest>`.
    pub fn parse(s: &str) -> Result<Self, ChecksumError> {
        let (algo_str, hex_str) = s
            .split_once(':')
            .ok_or_else(|| ChecksumError::InvalidFormat(s.to_string()))?;

        let algo: ChecksumAlgorithm = algo_str.parse()?;
        match algo {
            ChecksumAlgorithm::Sha256 => Self::new_sha256(hex_str),
        }
    }

    /// Returns the checksum algorithm.
    pub fn algorithm(&self) -> ChecksumAlgorithm {
        self.algorithm
    }

    /// Returns the normalized lowercase hexadecimal digest.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

impl fmt::Display for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm, self.digest)
    }
}

impl FromStr for Checksum {
    type Err = ChecksumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for Checksum {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Checksum {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ChecksumVisitor;

        impl Visitor<'_> for ChecksumVisitor {
            type Value = Checksum;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a checksum formatted as '<algorithm>:<hex_digest>'")
            }

            fn visit_str<E>(self, value: &str) -> Result<Checksum, E>
            where
                E: serde::de::Error,
            {
                Checksum::parse(value).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(ChecksumVisitor)
    }
}

/// Errors related to package source definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    Empty,
    InvalidFormat(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Empty => write!(f, "package source cannot be empty"),
            SourceError::InvalidFormat(msg) => write!(f, "invalid package source: {msg}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Represents the origin or acquisition source of a locked package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageSource(String);

impl PackageSource {
    /// Creates and validates a new package source identifier.
    pub fn new(source: impl Into<String>) -> Result<Self, SourceError> {
        let s = source.into();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(SourceError::Empty);
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Convenience constructor for registry sources (e.g. "registry+default").
    pub fn registry(name: &str) -> Self {
        Self(format!("registry+{name}"))
    }

    /// Convenience constructor for the default registry source.
    pub fn default_registry() -> Self {
        Self::registry("default")
    }

    /// Convenience constructor for git repository sources with explicit revisions.
    pub fn git(url: &str, rev: &str) -> Self {
        Self(format!("git+{url}?rev={rev}"))
    }

    /// Convenience constructor for local path dependency sources.
    pub fn path(path: &str) -> Self {
        Self(format!("path+{path}"))
    }

    /// Returns the source as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for PackageSource {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for PackageSource {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PackageSource {
    type Err = SourceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for PackageSource {
    type Error = SourceError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl TryFrom<String> for PackageSource {
    type Error = SourceError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Serialize for PackageSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PackageSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SourceVisitor;

        impl Visitor<'_> for SourceVisitor {
            type Value = PackageSource;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a valid package source string")
            }

            fn visit_str<E>(self, value: &str) -> Result<PackageSource, E>
            where
                E: serde::de::Error,
            {
                PackageSource::new(value).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(SourceVisitor)
    }
}

/// Represents a single locked package entry in `crux.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPackage {
    pub name: PackageName,
    pub version: semver::Version,
    pub source: PackageSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<Checksum>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<PackageId>,
}

impl LockedPackage {
    /// Creates a new locked package entry without checksum or dependencies.
    pub fn new(name: PackageName, version: semver::Version, source: PackageSource) -> Self {
        Self {
            name,
            version,
            source,
            checksum: None,
            dependencies: Vec::new(),
        }
    }

    /// Returns the unique `PackageId` for this package.
    pub fn id(&self) -> PackageId {
        PackageId::new(self.name.clone(), self.version.clone())
    }

    /// Sets or updates the archive integrity checksum.
    pub fn with_checksum(mut self, checksum: Checksum) -> Self {
        self.checksum = Some(checksum);
        self
    }

    /// Sets or updates the list of resolved dependencies.
    pub fn with_dependencies(mut self, dependencies: Vec<PackageId>) -> Self {
        self.dependencies = dependencies;
        self
    }
}

/// Determines deterministic ordering for locked packages.
///
/// Ordering precedence:
/// 1. `name`: Lexicographical comparison of `PackageName`.
/// 2. `version`: Semantic version comparison via `semver::Version::cmp`.
/// 3. `source`: Lexicographical comparison of `PackageSource`.
impl PartialOrd for LockedPackage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LockedPackage {
    fn cmp(&self, other: &Self) -> Ordering {
        self.name
            .cmp(&other.name)
            .then_with(|| self.version.cmp(&other.version))
            .then_with(|| self.source.cmp(&other.source))
    }
}

/// Semantic validation errors encountered when validating a lockfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockfileValidationError {
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
    DuplicatePackage {
        id: PackageId,
        source: PackageSource,
    },
    InvalidPackageName(NameValidationError),
    InvalidVersion(String),
    Checksum(ChecksumError),
    Source(SourceError),
}

impl fmt::Display for LockfileValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockfileValidationError::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "unsupported lockfile version '{found}' (current supported version is {supported})"
                )
            }
            LockfileValidationError::DuplicatePackage { id, source } => {
                write!(
                    f,
                    "duplicate locked package entry '{id}' with source '{source}'"
                )
            }
            LockfileValidationError::InvalidPackageName(err) => {
                write!(f, "invalid package name in lockfile: {err}")
            }
            LockfileValidationError::InvalidVersion(err) => {
                write!(f, "invalid version in lockfile: {err}")
            }
            LockfileValidationError::Checksum(err) => {
                write!(f, "checksum error in lockfile: {err}")
            }
            LockfileValidationError::Source(err) => write!(f, "source error in lockfile: {err}"),
        }
    }
}

impl std::error::Error for LockfileValidationError {}

impl From<NameValidationError> for LockfileValidationError {
    fn from(err: NameValidationError) -> Self {
        LockfileValidationError::InvalidPackageName(err)
    }
}

impl From<ChecksumError> for LockfileValidationError {
    fn from(err: ChecksumError) -> Self {
        LockfileValidationError::Checksum(err)
    }
}

impl From<SourceError> for LockfileValidationError {
    fn from(err: SourceError) -> Self {
        LockfileValidationError::Source(err)
    }
}

/// Errors encountered when loading, reading, parsing, or validating a lockfile.
#[derive(Debug)]
pub enum LockfileError {
    Io(io::Error),
    Toml(toml::de::Error),
    Validation(LockfileValidationError),
}

impl fmt::Display for LockfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockfileError::Io(err) => write!(f, "failed to read lockfile: {err}"),
            LockfileError::Toml(err) => write!(f, "failed to parse lockfile TOML: {err}"),
            LockfileError::Validation(err) => write!(f, "lockfile validation error: {err}"),
        }
    }
}

impl std::error::Error for LockfileError {}

impl From<io::Error> for LockfileError {
    fn from(err: io::Error) -> Self {
        LockfileError::Io(err)
    }
}

impl From<toml::de::Error> for LockfileError {
    fn from(err: toml::de::Error) -> Self {
        LockfileError::Toml(err)
    }
}

impl From<LockfileValidationError> for LockfileError {
    fn from(err: LockfileValidationError) -> Self {
        LockfileError::Validation(err)
    }
}

/// Top-level model representing `crux.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    pub lockfile_version: u32,
    #[serde(default, rename = "package")]
    pub packages: Vec<LockedPackage>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self::new()
    }
}

impl Lockfile {
    /// Creates an empty lockfile initialized with the current supported schema version.
    pub fn new() -> Self {
        Self {
            lockfile_version: CURRENT_LOCKFILE_VERSION,
            packages: Vec::new(),
        }
    }

    /// Creates a lockfile from a list of packages and validates constraints.
    pub fn with_packages(
        mut packages: Vec<LockedPackage>,
    ) -> Result<Self, LockfileValidationError> {
        let mut lockfile = Self {
            lockfile_version: CURRENT_LOCKFILE_VERSION,
            packages: Vec::new(),
        };

        packages.sort();
        for pkg in packages {
            lockfile.add_package(pkg)?;
        }

        Ok(lockfile)
    }

    /// Adds a package entry, ensuring no duplicate identities exist.
    pub fn add_package(&mut self, pkg: LockedPackage) -> Result<(), LockfileValidationError> {
        let key = (pkg.id(), pkg.source.clone());
        if self
            .packages
            .iter()
            .any(|p| p.id() == key.0 && p.source == key.1)
        {
            return Err(LockfileValidationError::DuplicatePackage {
                id: key.0,
                source: key.1,
            });
        }

        self.packages.push(pkg);
        self.sort_packages();
        Ok(())
    }

    /// Sorts packages and their dependencies deterministically.
    pub fn sort_packages(&mut self) {
        self.packages.sort();
        for pkg in &mut self.packages {
            pkg.dependencies.sort();
        }
    }

    /// Validates the semantic constraints of the lockfile.
    pub fn validate(&self) -> Result<(), LockfileValidationError> {
        if self.lockfile_version != CURRENT_LOCKFILE_VERSION {
            return Err(LockfileValidationError::UnsupportedVersion {
                found: self.lockfile_version,
                supported: CURRENT_LOCKFILE_VERSION,
            });
        }

        let mut seen = HashSet::new();
        for pkg in &self.packages {
            let key = (pkg.id(), pkg.source.clone());
            if !seen.insert(key.clone()) {
                return Err(LockfileValidationError::DuplicatePackage {
                    id: key.0,
                    source: key.1,
                });
            }
        }

        Ok(())
    }

    /// Returns the number of locked packages.
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Parses a lockfile from a TOML string and verifies version and semantic constraints.
    pub fn from_toml_str(content: &str) -> Result<Self, LockfileError> {
        let lockfile: Lockfile = toml::from_str(content).map_err(LockfileError::Toml)?;
        lockfile.validate().map_err(LockfileError::Validation)?;
        Ok(lockfile)
    }

    /// Loads and parses a lockfile from disk.
    pub fn load_from_path(path: &Path) -> Result<Self, LockfileError> {
        let content = fs::read_to_string(path).map_err(LockfileError::Io)?;
        Self::from_toml_str(&content)
    }

    /// Deterministically serializes the lockfile into canonical TOML format.
    ///
    /// Stable package ordering is enforced: packages are sorted primarily by name,
    /// secondarily by semantic version, and tertiarily by package source.
    /// Repeated serialization of identical logical content produces byte-identical output.
    pub fn to_toml_string(&self) -> Result<String, LockfileError> {
        self.validate().map_err(LockfileError::Validation)?;

        let mut normalized = self.clone();
        normalized.sort_packages();

        let mut out = format!("lockfile_version = {}\n", normalized.lockfile_version);

        for pkg in &normalized.packages {
            out.push_str("\n[[package]]\n");
            out.push_str(&format!("name = \"{}\"\n", pkg.name));
            out.push_str(&format!("version = \"{}\"\n", pkg.version));
            out.push_str(&format!("source = \"{}\"\n", pkg.source));

            if let Some(ref cs) = pkg.checksum {
                out.push_str(&format!("checksum = \"{cs}\"\n"));
            }

            if !pkg.dependencies.is_empty() {
                out.push_str("dependencies = [\n");
                for dep in &pkg.dependencies {
                    out.push_str(&format!("    \"{dep}\",\n"));
                }
                out.push_str("]\n");
            }
        }

        Ok(out)
    }

    /// Atomically writes the lockfile to disk in the specified directory using a temporary file.
    ///
    /// Ensures that:
    /// 1. The lockfile is fully validated and serialized in memory before any filesystem operations.
    /// 2. Bytes are flushed and synced to physical storage.
    /// 3. The destination file is replaced via an atomic rename.
    /// 4. Any failure cleans up temporary files without corrupting an existing `crux.lock`.
    pub fn write_atomic(&self, dir: &Path) -> Result<PathBuf, LockfileError> {
        write_lockfile_atomically(dir, self)
    }
}

/// Atomically writes a lockfile to disk in the specified directory.
pub fn write_lockfile_atomically(
    dir: &Path,
    lockfile: &Lockfile,
) -> Result<PathBuf, LockfileError> {
    if !dir.is_dir() {
        return Err(LockfileError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("directory '{}' does not exist", dir.display()),
        )));
    }

    // Validate and serialize in-memory before touching disk
    lockfile.validate().map_err(LockfileError::Validation)?;
    let serialized = lockfile.to_toml_string()?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let temp_name = format!(".{LOCKFILE_FILENAME}.tmp.{pid}.{nanos}");
    let temp_path = dir.join(temp_name);

    struct TempFileGuard<'a> {
        path: &'a Path,
        active: bool,
    }

    impl<'a> Drop for TempFileGuard<'a> {
        fn drop(&mut self) {
            if self.active {
                let _ = fs::remove_file(self.path);
            }
        }
    }

    let mut guard = TempFileGuard {
        path: &temp_path,
        active: true,
    };

    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(LockfileError::Io)?;

        file.write_all(serialized.as_bytes())
            .map_err(LockfileError::Io)?;
        file.flush().map_err(LockfileError::Io)?;
        file.sync_data().map_err(LockfileError::Io)?;
    }

    let dest_path = dir.join(LOCKFILE_FILENAME);
    fs::rename(&temp_path, &dest_path).map_err(LockfileError::Io)?;

    guard.active = false;
    Ok(dest_path)
}

/// Detailed reasons why a lockfile is out of synchronization with `crux.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockSyncError {
    MissingPackage {
        package: PackageName,
        requirement: semver::VersionReq,
    },
    IncompatibleVersion {
        package: PackageName,
        locked: semver::Version,
        requirement: semver::VersionReq,
    },
    ObsoletePackage {
        package: PackageId,
    },
    InvalidDependencyRelationship {
        dependent: PackageId,
        missing: PackageId,
    },
    ResolutionFailed(String),
}

impl fmt::Display for LockSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockSyncError::MissingPackage {
                package,
                requirement,
            } => {
                write!(
                    f,
                    "missing package '{package}' in lockfile (required by manifest with '{requirement}')"
                )
            }
            LockSyncError::IncompatibleVersion {
                package,
                locked,
                requirement,
            } => {
                write!(
                    f,
                    "locked version of package '{package}' is {locked}, which does not satisfy manifest requirement '{requirement}'"
                )
            }
            LockSyncError::ObsoletePackage { package } => {
                write!(
                    f,
                    "obsolete package '{package}' in lockfile is no longer reachable from manifest dependencies"
                )
            }
            LockSyncError::InvalidDependencyRelationship { dependent, missing } => {
                write!(
                    f,
                    "package '{dependent}' in lockfile depends on '{missing}', which is not present in the lockfile"
                )
            }
            LockSyncError::ResolutionFailed(msg) => {
                write!(f, "resolution failed during synchronization check: {msg}")
            }
        }
    }
}

impl std::error::Error for LockSyncError {}

/// Errors that can occur during `crux lock --check` or lockfile synchronization.
#[derive(Debug)]
pub enum LockError {
    LockfileNotFound(PathBuf),
    ManifestNotFound(PathBuf),
    Manifest(ManifestError),
    Lockfile(LockfileError),
    Sync(Box<LockSyncError>),
    Resolution(Box<ResolutionError>),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::LockfileNotFound(path) => {
                write!(f, "error: lockfile not found at '{}'", path.display())
            }
            LockError::ManifestNotFound(path) => {
                write!(f, "error: manifest not found at '{}'", path.display())
            }
            LockError::Manifest(err) => write!(f, "manifest error: {err}"),
            LockError::Lockfile(err) => write!(f, "{err}"),
            LockError::Sync(err) => write!(f, "lockfile out of sync: {err}"),
            LockError::Resolution(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for LockError {}

impl From<LockfileError> for LockError {
    fn from(err: LockfileError) -> Self {
        LockError::Lockfile(err)
    }
}

impl From<ManifestError> for LockError {
    fn from(err: ManifestError) -> Self {
        LockError::Manifest(err)
    }
}

impl From<LockSyncError> for LockError {
    fn from(err: LockSyncError) -> Self {
        LockError::Sync(Box::new(err))
    }
}

impl From<ResolutionError> for LockError {
    fn from(err: ResolutionError) -> Self {
        LockError::Resolution(Box::new(err))
    }
}

/// Summary report returned upon successful lockfile validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockCheckReport {
    pub lockfile_path: PathBuf,
    pub lockfile_version: u32,
    pub package_count: usize,
    pub is_synchronized: bool,
}

/// Verifies synchronization between `crux.toml` and `crux.lock` in the specified directory.
///
/// Does not modify or write to any files during verification.
pub fn check_lockfile_sync(
    dir: &Path,
    catalog: &dyn PackageCatalog,
) -> Result<LockCheckReport, LockError> {
    let lockfile_path = dir.join(LOCKFILE_FILENAME);
    if !lockfile_path.is_file() {
        return Err(LockError::LockfileNotFound(lockfile_path));
    }

    let lockfile = Lockfile::load_from_path(&lockfile_path)?;

    let manifest_path = dir.join(MANIFEST_FILENAME);
    if !manifest_path.is_file() {
        // If no manifest is present in this directory, we only validate the lockfile schema
        return Ok(LockCheckReport {
            lockfile_path,
            lockfile_version: lockfile.lockfile_version,
            package_count: lockfile.package_count(),
            is_synchronized: false,
        });
    }

    let manifest = Manifest::load_from_path(&manifest_path)?;

    // 1. Verify that each direct dependency in manifest exists in lockfile with compatible version
    for dep in &manifest.dependencies {
        match lockfile.packages.iter().find(|p| p.name == *dep.name()) {
            None => {
                return Err(LockError::Sync(Box::new(LockSyncError::MissingPackage {
                    package: dep.name().clone(),
                    requirement: dep.version_req().clone(),
                })));
            }
            Some(locked_pkg) => {
                if !dep.version_req().matches(&locked_pkg.version) {
                    return Err(LockError::Sync(Box::new(
                        LockSyncError::IncompatibleVersion {
                            package: dep.name().clone(),
                            locked: locked_pkg.version.clone(),
                            requirement: dep.version_req().clone(),
                        },
                    )));
                }
            }
        }
    }

    // 2. Verify internal dependency relationships within lockfile
    for pkg in &lockfile.packages {
        for dep_id in &pkg.dependencies {
            if !lockfile.packages.iter().any(|p| p.id() == *dep_id) {
                return Err(LockError::Sync(Box::new(
                    LockSyncError::InvalidDependencyRelationship {
                        dependent: pkg.id(),
                        missing: dep_id.clone(),
                    },
                )));
            }
        }
    }

    // 3. Resolve with preferences from lockfile to verify full graph equivalence
    let mut preferences = BTreeMap::new();
    for pkg in &lockfile.packages {
        preferences.insert(pkg.name.clone(), pkg.version.clone());
    }

    let resolver = Resolver::new(catalog);
    let graph = resolver
        .resolve_with_preferences(
            Some(manifest.package.id()),
            &manifest.dependencies,
            &preferences,
        )
        .map_err(|e| LockError::Sync(Box::new(LockSyncError::ResolutionFailed(e.to_string()))))?;

    let expected_lockfile = graph
        .to_lockfile()
        .map_err(|e| LockError::Lockfile(LockfileError::Validation(e)))?;

    // Check for obsolete packages (present in lockfile, but not reachable in expected graph)
    for pkg in &lockfile.packages {
        if !expected_lockfile
            .packages
            .iter()
            .any(|p| p.name == pkg.name)
        {
            return Err(LockError::Sync(Box::new(LockSyncError::ObsoletePackage {
                package: pkg.id(),
            })));
        }
    }

    // Check for missing packages (in expected graph, but not in lockfile)
    for exp_pkg in &expected_lockfile.packages {
        match lockfile.packages.iter().find(|p| p.name == exp_pkg.name) {
            None => {
                return Err(LockError::Sync(Box::new(LockSyncError::MissingPackage {
                    package: exp_pkg.name.clone(),
                    requirement: semver::VersionReq::STAR,
                })));
            }
            Some(locked) => {
                if locked.version != exp_pkg.version {
                    return Err(LockError::Sync(Box::new(
                        LockSyncError::IncompatibleVersion {
                            package: exp_pkg.name.clone(),
                            locked: locked.version.clone(),
                            requirement: semver::VersionReq::parse(&format!(
                                "={}",
                                exp_pkg.version
                            ))
                            .unwrap(),
                        },
                    )));
                }
            }
        }
    }

    Ok(LockCheckReport {
        lockfile_path,
        lockfile_version: lockfile.lockfile_version,
        package_count: lockfile.package_count(),
        is_synchronized: true,
    })
}

/// Verifies and checks an existing `crux.lock` file in the specified directory.
///
/// If a `crux.toml` manifest is present, checks full synchronization against
/// declared dependencies using the supplied catalog. Does not modify or write
/// to the lockfile during validation. Callers must provide the same registry
/// catalog used during resolution; no implicit test fixture is consulted.
pub fn check_lockfile(
    dir: &Path,
    catalog: &dyn PackageCatalog,
) -> Result<LockCheckReport, LockError> {
    check_lockfile_sync(dir, catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::default_dev_catalog;

    #[test]
    fn test_empty_lockfile() {
        let lockfile = Lockfile::new();
        assert_eq!(lockfile.lockfile_version, 1);
        assert_eq!(lockfile.package_count(), 0);

        let toml_str = lockfile.to_toml_string().unwrap();
        assert_eq!(toml_str.trim(), "lockfile_version = 1");

        let parsed = Lockfile::from_toml_str(&toml_str).unwrap();
        assert_eq!(parsed, lockfile);
    }

    #[test]
    fn test_valid_lockfile_with_checksum() {
        let toml_str = r#"
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
checksum = "sha256:d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620"

[[package]]
name = "zlib"
version = "1.3.1"
source = "registry+default"
checksum = "sha256:9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"
"#;
        let lockfile = Lockfile::from_toml_str(toml_str).unwrap();
        assert_eq!(lockfile.lockfile_version, 1);
        assert_eq!(lockfile.package_count(), 2);

        let fmt_pkg = &lockfile.packages[0];
        assert_eq!(fmt_pkg.name.as_str(), "fmt");
        assert_eq!(fmt_pkg.version.to_string(), "11.1.1");
        assert_eq!(fmt_pkg.source.as_str(), "registry+default");
        assert!(fmt_pkg.checksum.is_some());
        assert_eq!(
            fmt_pkg.checksum.as_ref().unwrap().to_string(),
            "sha256:d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620"
        );
    }

    #[test]
    fn test_multiple_versions_of_same_package() {
        let toml_str = r#"
lockfile_version = 1

[[package]]
name = "zlib"
version = "1.2.11"
source = "registry+default"

[[package]]
name = "zlib"
version = "1.3.1"
source = "registry+default"
"#;
        let lockfile = Lockfile::from_toml_str(toml_str).unwrap();
        assert_eq!(lockfile.package_count(), 2);
        assert_eq!(lockfile.packages[0].version.to_string(), "1.2.11");
        assert_eq!(lockfile.packages[1].version.to_string(), "1.3.1");
    }

    #[test]
    fn test_duplicate_package_identities_rejected() {
        let toml_str = r#"
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
        let res = Lockfile::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(LockfileError::Validation(
                LockfileValidationError::DuplicatePackage { .. }
            ))
        ));
    }

    #[test]
    fn test_deterministic_serialization_package_ordering() {
        let pkg_zlib = LockedPackage::new(
            PackageName::new("zlib").unwrap(),
            semver::Version::parse("1.3.1").unwrap(),
            PackageSource::default_registry(),
        );

        let pkg_fmt = LockedPackage::new(
            PackageName::new("fmt").unwrap(),
            semver::Version::parse("11.1.1").unwrap(),
            PackageSource::default_registry(),
        );

        let pkg_openssl = LockedPackage::new(
            PackageName::new("openssl").unwrap(),
            semver::Version::parse("3.4.0").unwrap(),
            PackageSource::default_registry(),
        );

        // Construct two lockfiles in reverse order
        let mut lockfile_a = Lockfile::new();
        lockfile_a.packages = vec![pkg_zlib.clone(), pkg_fmt.clone(), pkg_openssl.clone()];

        let mut lockfile_b = Lockfile::new();
        lockfile_b.packages = vec![pkg_openssl, pkg_fmt, pkg_zlib];

        let out_a = lockfile_a.to_toml_string().unwrap();
        let out_b = lockfile_b.to_toml_string().unwrap();

        assert_eq!(out_a, out_b);

        // Verify alphabetical order by name
        let parsed = Lockfile::from_toml_str(&out_a).unwrap();
        assert_eq!(parsed.packages[0].name.as_str(), "fmt");
        assert_eq!(parsed.packages[1].name.as_str(), "openssl");
        assert_eq!(parsed.packages[2].name.as_str(), "zlib");
    }

    #[test]
    fn test_unsupported_lockfile_version() {
        let toml_str = r#"
lockfile_version = 2

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
        let res = Lockfile::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(LockfileError::Validation(
                LockfileValidationError::UnsupportedVersion {
                    found: 2,
                    supported: 1
                }
            ))
        ));
    }

    #[test]
    fn test_missing_lockfile_version() {
        let toml_str = r#"
[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
        let res = Lockfile::from_toml_str(toml_str);
        assert!(matches!(res, Err(LockfileError::Toml(_))));
    }

    #[test]
    fn test_missing_required_fields_in_package() {
        let toml_str = r#"
lockfile_version = 1

[[package]]
name = "fmt"
"#;
        let res = Lockfile::from_toml_str(toml_str);
        assert!(matches!(res, Err(LockfileError::Toml(_))));
    }

    #[test]
    fn test_checksum_validation() {
        // Valid checksum
        let cs = Checksum::parse(
            "sha256:d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620",
        )
        .unwrap();
        assert_eq!(cs.algorithm(), ChecksumAlgorithm::Sha256);
        assert_eq!(
            cs.digest(),
            "d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620"
        );

        // Uppercase normalized to lowercase
        let cs_upper = Checksum::parse(
            "sha256:D8C6B48BCBE6C507C57DBD8091176B66E33F3893DA7E4C29C8E1CB0FCBF1A620",
        )
        .unwrap();
        assert_eq!(cs, cs_upper);

        // Invalid algorithm
        assert!(matches!(
            Checksum::parse("md5:d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620"),
            Err(ChecksumError::UnsupportedAlgorithm(_))
        ));

        // Invalid length
        assert!(matches!(
            Checksum::parse("sha256:1234"),
            Err(ChecksumError::InvalidHexDigest(_))
        ));

        // Non-hex characters
        assert!(matches!(
            Checksum::parse(
                "sha256:zzc6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620"
            ),
            Err(ChecksumError::InvalidHexDigest(_))
        ));
    }

    #[test]
    fn test_atomic_write_creates_lockfile() {
        let temp_dir = std::env::temp_dir().join(format!(
            "crux_test_atomic_write_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let mut lockfile = Lockfile::new();
        lockfile
            .add_package(LockedPackage::new(
                PackageName::new("fmt").unwrap(),
                semver::Version::parse("11.1.1").unwrap(),
                PackageSource::default_registry(),
            ))
            .unwrap();

        let written_path = lockfile.write_atomic(&temp_dir).unwrap();
        assert!(written_path.is_file());
        assert_eq!(written_path, temp_dir.join(LOCKFILE_FILENAME));

        let loaded = Lockfile::load_from_path(&written_path).unwrap();
        assert_eq!(loaded, lockfile);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_atomic_write_to_nonexistent_directory_fails() {
        let nonexistent = PathBuf::from("/nonexistent/path/for/crux/test");
        let lockfile = Lockfile::new();
        let res = lockfile.write_atomic(&nonexistent);
        assert!(res.is_err());
    }

    #[test]
    fn test_check_lockfile_sync_with_matching_manifest() {
        let temp_dir = std::env::temp_dir().join(format!(
            "crux_test_sync_ok_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let manifest_content = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = "^11.1"
"#;
        fs::write(temp_dir.join("crux.toml"), manifest_content).unwrap();

        let lock_content = r#"lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
        fs::write(temp_dir.join("crux.lock"), lock_content).unwrap();

        let report = check_lockfile(&temp_dir, &default_dev_catalog()).unwrap();
        assert!(report.is_synchronized);
        assert_eq!(report.package_count, 1);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_check_lockfile_sync_detects_incompatible_version() {
        let temp_dir = std::env::temp_dir().join(format!(
            "crux_test_sync_incompat_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let manifest_content = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = "^11.1"
"#;
        fs::write(temp_dir.join("crux.toml"), manifest_content).unwrap();

        // Locked fmt is 10.0.0, which does not satisfy ^11.1
        let lock_content = r#"lockfile_version = 1

[[package]]
name = "fmt"
version = "10.0.0"
source = "registry+default"
"#;
        fs::write(temp_dir.join("crux.lock"), lock_content).unwrap();

        let err = check_lockfile(&temp_dir, &default_dev_catalog()).unwrap_err();
        match err {
            LockError::Sync(details) => {
                assert!(matches!(
                    *details,
                    LockSyncError::IncompatibleVersion { .. }
                ));
            }
            other => panic!("expected Sync error with IncompatibleVersion, got: {other:?}"),
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
