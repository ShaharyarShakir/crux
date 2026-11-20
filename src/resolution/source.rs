//! Validated package source identities used by manifests and lockfiles.
//!
//! Crux keeps source identity as a canonical string so the resolver, lockfile,
//! cache, and acquisition pipeline can share one representation. Direct sources
//! have a strict grammar:
//!
//! - `git+<url>#tag=<tag>`, `git+<url>#branch=<branch>`, `git+<url>#rev=<rev>`,
//!   or `git+<url>#commit=<full-commit>`
//! - `git+<url>?rev=<rev>` (legacy compatibility form)
//! - `archive+<url>#sha256=<digest>`
//! - `path+<relative-or-absolute-path>`
//!
//! Locked Git sources may carry both the originally requested ref and the
//! resolved immutable commit, for example
//! `git+https://example.invalid/lib.git#tag=v1.2.3#commit=<40-hex>`.

use super::lockfile::{Checksum, ChecksumError};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

const REGISTRY_PREFIX: &str = "registry+";
const GIT_PREFIX: &str = "git+";
const ARCHIVE_PREFIX: &str = "archive+";
const PATH_PREFIX: &str = "path+";

/// Errors encountered while parsing or validating a package source identity.
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

impl From<ChecksumError> for SourceError {
    fn from(err: ChecksumError) -> Self {
        SourceError::InvalidFormat(err.to_string())
    }
}

/// The requested Git revision for a direct Git dependency.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitRef {
    /// Track the repository's default `HEAD`.
    Head,
    /// Request a named tag.
    Tag(String),
    /// Request a branch. The resolved commit is always locked.
    Branch(String),
    /// Request an arbitrary revision expression (tag, short SHA, etc.).
    Rev(String),
    /// Request a specific full commit SHA.
    Commit(String),
}

impl GitRef {
    /// Validates a user-supplied Git ref before it is used as a Git argument.
    pub fn validate(&self) -> Result<(), SourceError> {
        match self {
            GitRef::Head => Ok(()),
            GitRef::Tag(value) | GitRef::Branch(value) | GitRef::Rev(value) => {
                validate_git_ref(value).map(|_| ())
            }
            GitRef::Commit(value) => validate_commit(value).map(|_| ()),
        }
    }

    /// Returns the canonical fragment token for this ref.
    pub fn fragment(&self) -> String {
        match self {
            GitRef::Head => "head".to_string(),
            GitRef::Tag(value) => format!("tag={value}"),
            GitRef::Branch(value) => format!("branch={value}"),
            GitRef::Rev(value) => format!("rev={value}"),
            GitRef::Commit(value) => format!("commit={value}"),
        }
    }

    /// Returns a short human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            GitRef::Head => "default HEAD",
            GitRef::Tag(_) => "tag",
            GitRef::Branch(_) => "branch",
            GitRef::Rev(_) => "revision",
            GitRef::Commit(_) => "commit",
        }
    }
}

/// A validated direct package source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectSource {
    /// A Git repository, with an optional resolved immutable commit.
    Git {
        url: String,
        requested: GitRef,
        resolved_commit: Option<String>,
    },
    /// A verified remote source archive.
    Archive { url: String, checksum: Checksum },
    /// A local filesystem package directory.
    Path { path: String },
}

impl DirectSource {
    /// Parses a canonical direct-source string.
    pub fn parse(value: &str) -> Result<Self, SourceError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(SourceError::Empty);
        }

        if let Some(rest) = value.strip_prefix(GIT_PREFIX) {
            return Self::parse_git(rest);
        }
        if let Some(rest) = value.strip_prefix(ARCHIVE_PREFIX) {
            return Self::parse_archive(rest);
        }
        if let Some(rest) = value.strip_prefix(PATH_PREFIX) {
            return Self::parse_path(rest);
        }

        Err(SourceError::InvalidFormat(format!(
            "'{value}' must begin with 'git+', 'archive+', or 'path+'"
        )))
    }

    fn parse_git(rest: &str) -> Result<Self, SourceError> {
        if rest.is_empty() {
            return Err(SourceError::InvalidFormat(
                "git source is missing a repository URL".to_string(),
            ));
        }

        // Legacy form: git+<url>?rev=<rev>
        if let Some((url, query)) = rest.split_once('?') {
            let rev = query.strip_prefix("rev=").ok_or_else(|| {
                SourceError::InvalidFormat(format!(
                    "unsupported Git source query '{query}'; expected legacy '?rev=<revision>'"
                ))
            })?;
            validate_git_url(url)?;
            let requested = GitRef::Rev(validate_git_ref(rev)?);
            return Ok(Self::Git {
                url: url.to_string(),
                requested,
                resolved_commit: None,
            });
        }

        let (url, fragments) = match rest.split_once('#') {
            Some((url, fragments)) => (url, fragments),
            None => (rest, ""),
        };
        validate_git_url(url)?;

        let mut requested: Option<GitRef> = None;
        let mut resolved_commit: Option<String> = None;

        for fragment in fragments.split('#').filter(|part| !part.is_empty()) {
            if fragment == "head" {
                set_git_ref(&mut requested, GitRef::Head)?;
                continue;
            }

            let Some((key, value)) = fragment.split_once('=') else {
                return Err(SourceError::InvalidFormat(format!(
                    "invalid Git source fragment '{fragment}'"
                )));
            };
            if value.is_empty() {
                return Err(SourceError::InvalidFormat(format!(
                    "Git source fragment '{key}' requires a value"
                )));
            }

            match key {
                "tag" => set_git_ref(&mut requested, GitRef::Tag(validate_git_ref(value)?))?,
                "branch" => set_git_ref(&mut requested, GitRef::Branch(validate_git_ref(value)?))?,
                "rev" => set_git_ref(&mut requested, GitRef::Rev(validate_git_ref(value)?))?,
                "commit" => {
                    if resolved_commit.is_some() {
                        return Err(SourceError::InvalidFormat(
                            "Git source declares more than one resolved commit".to_string(),
                        ));
                    }
                    resolved_commit = Some(validate_commit(value)?);
                }
                other => {
                    return Err(SourceError::InvalidFormat(format!(
                        "unknown Git source fragment '{other}'"
                    )));
                }
            }
        }

        Ok(Self::Git {
            url: url.to_string(),
            requested: requested.unwrap_or(GitRef::Head),
            resolved_commit,
        })
    }

    fn parse_archive(rest: &str) -> Result<Self, SourceError> {
        let (url, fragment) = rest.split_once('#').ok_or_else(|| {
            SourceError::InvalidFormat(
                "archive source requires '#sha256=<digest>' after the URL".to_string(),
            )
        })?;
        validate_archive_url(url)?;
        let digest = fragment.strip_prefix("sha256=").ok_or_else(|| {
            SourceError::InvalidFormat(format!(
                "unsupported archive source fragment '{fragment}'; expected 'sha256=<digest>'"
            ))
        })?;
        if fragment.contains('#') {
            return Err(SourceError::InvalidFormat(
                "archive source declares multiple fragments".to_string(),
            ));
        }
        let checksum = Checksum::new_sha256(digest)?;
        Ok(Self::Archive {
            url: url.to_string(),
            checksum,
        })
    }

    fn parse_path(rest: &str) -> Result<Self, SourceError> {
        let normalized = normalize_direct_path(rest);
        if normalized.as_os_str().is_empty() {
            return Err(SourceError::InvalidFormat(
                "local path source is missing a path".to_string(),
            ));
        }
        let text = path_to_source_string(&normalized)?;
        Ok(Self::Path { path: text })
    }

    /// Returns the canonical source string.
    pub fn to_string_canonical(&self) -> String {
        match self {
            DirectSource::Git {
                url,
                requested,
                resolved_commit,
            } => {
                let mut out = format!("{GIT_PREFIX}{url}#{}", requested.fragment());
                if let Some(commit) = resolved_commit {
                    out.push_str(&format!("#commit={commit}"));
                }
                out
            }
            DirectSource::Archive { url, checksum } => {
                format!("{ARCHIVE_PREFIX}{url}#sha256={}", checksum.digest())
            }
            DirectSource::Path { path } => format!("{PATH_PREFIX}{path}"),
        }
    }

    /// Returns a copy with an immutable Git commit attached.
    pub fn with_resolved_commit(&self, commit: &str) -> Result<Self, SourceError> {
        match self {
            DirectSource::Git {
                url,
                requested,
                resolved_commit: _,
            } => Ok(Self::Git {
                url: url.clone(),
                requested: requested.clone(),
                resolved_commit: Some(validate_commit(commit)?),
            }),
            other => Err(SourceError::InvalidFormat(format!(
                "source kind '{}' cannot carry a Git commit",
                other.kind()
            ))),
        }
    }

    /// Returns a stable key that ignores the resolved Git commit.
    ///
    /// Two Git sources are the same dependency if they point at the same URL
    /// and request the same ref. The commit is acquisition state, not a new
    /// dependency identity.
    pub fn identity_key(&self) -> String {
        match self {
            DirectSource::Git { url, requested, .. } => {
                format!("{GIT_PREFIX}{url}#{}", requested.fragment())
            }
            DirectSource::Archive { url, checksum } => {
                format!("{ARCHIVE_PREFIX}{url}#sha256={}", checksum.digest())
            }
            DirectSource::Path { path } => format!("{PATH_PREFIX}{path}"),
        }
    }

    /// Returns true when this source can satisfy a manifest declaration.
    pub fn matches_manifest(&self, manifest: &DirectSource) -> bool {
        match (self, manifest) {
            (
                DirectSource::Git {
                    url: locked_url,
                    requested: locked_ref,
                    resolved_commit,
                },
                DirectSource::Git {
                    url: manifest_url,
                    requested: manifest_ref,
                    resolved_commit: manifest_commit,
                },
            ) => {
                if locked_url != manifest_url {
                    return false;
                }
                // A locked commit is authoritative. A manifest declaration may
                // retain its requested ref while the lockfile pins the commit.
                if resolved_commit.is_some() {
                    let ref_matches = manifest_ref == locked_ref
                        || (matches!(locked_ref, GitRef::Commit(_))
                            && matches!(manifest_ref, GitRef::Rev(_)));
                    return ref_matches
                        && manifest_commit
                            .as_deref()
                            .is_none_or(|c| resolved_commit.as_deref() == Some(c));
                }
                locked_ref == manifest_ref && resolved_commit == manifest_commit
            }
            (
                DirectSource::Archive {
                    url: locked_url,
                    checksum: locked_checksum,
                },
                DirectSource::Archive {
                    url: manifest_url,
                    checksum: manifest_checksum,
                },
            ) => locked_url == manifest_url && locked_checksum == manifest_checksum,
            (DirectSource::Path { path: locked }, DirectSource::Path { path: manifest }) => {
                locked == manifest
            }
            _ => false,
        }
    }

    /// Returns a human-readable source kind.
    pub fn kind(&self) -> &'static str {
        match self {
            DirectSource::Git { .. } => "git",
            DirectSource::Archive { .. } => "archive",
            DirectSource::Path { .. } => "path",
        }
    }
}

fn set_git_ref(slot: &mut Option<GitRef>, value: GitRef) -> Result<(), SourceError> {
    if slot.is_some() {
        return Err(SourceError::InvalidFormat(
            "Git source declares more than one requested revision".to_string(),
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn has_control_characters(value: &str) -> bool {
    value.chars().any(|c| c.is_control())
}

/// Validates a Git repository URL.
///
/// HTTPS, SSH, `git://`, `file://`, SCP-style `user@host:path`, and local paths
/// are accepted. Leading dashes and shell metacharacters are rejected because
/// Git arguments are never interpreted by a shell but may still be parsed as
/// options by the Git CLI.
pub fn validate_git_url(url: &str) -> Result<String, SourceError> {
    let url = url.trim();
    if url.is_empty() || has_control_characters(url) {
        return Err(SourceError::InvalidFormat(
            "Git repository URL cannot be empty or contain control characters".to_string(),
        ));
    }
    if url.starts_with('-') {
        return Err(SourceError::InvalidFormat(format!(
            "Git repository URL '{url}' cannot start with '-'"
        )));
    }
    if url.contains('#') {
        return Err(SourceError::InvalidFormat(
            "Git repository URL cannot contain '#' because it separates source fragments"
                .to_string(),
        ));
    }
    if url.contains('?') {
        return Err(SourceError::InvalidFormat(
            "Git repository URL cannot contain '?' because it is reserved by the legacy rev form"
                .to_string(),
        ));
    }

    let accepted_scheme = ["https://", "http://", "ssh://", "git://", "file://"]
        .iter()
        .any(|scheme| url.starts_with(scheme));
    let scp_style = url.contains('@') && url.contains(':') && !url.starts_with('/');
    let local_path = url.starts_with('/') || url.starts_with("./") || url.starts_with("../");

    if !accepted_scheme && !scp_style && !local_path {
        return Err(SourceError::InvalidFormat(format!(
            "unsupported Git repository URL '{url}': expected an HTTPS/SSH URL, a file URL, or a local path"
        )));
    }

    Ok(url.to_string())
}

/// Validates a Git tag, branch, or revision expression.
pub fn validate_git_ref(value: &str) -> Result<String, SourceError> {
    let value = value.trim();
    if value.is_empty() || has_control_characters(value) {
        return Err(SourceError::InvalidFormat(
            "Git revision cannot be empty or contain control characters".to_string(),
        ));
    }
    if value.starts_with('-') || value.starts_with('/') || value.ends_with('/') {
        return Err(SourceError::InvalidFormat(format!(
            "unsafe Git revision '{value}'"
        )));
    }
    if value.contains("..") || value.contains('\\') || value.contains(':') {
        return Err(SourceError::InvalidFormat(format!(
            "unsafe Git revision '{value}'"
        )));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
    {
        return Err(SourceError::InvalidFormat(format!(
            "Git revision '{value}' contains unsupported characters"
        )));
    }
    Ok(value.to_string())
}

fn validate_commit(value: &str) -> Result<String, SourceError> {
    let value = value.trim();
    if value.len() != 40 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SourceError::InvalidFormat(format!(
            "locked Git commit must be a 40-character hexadecimal SHA, found '{value}'"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

/// Validates an HTTP(S) archive URL.
pub fn validate_archive_url(url: &str) -> Result<String, SourceError> {
    let url = url.trim();
    if url.is_empty() || has_control_characters(url) {
        return Err(SourceError::InvalidFormat(
            "archive URL cannot be empty or contain control characters".to_string(),
        ));
    }
    if url.contains('#') {
        return Err(SourceError::InvalidFormat(
            "archive URL cannot contain '#' because it separates the checksum fragment".to_string(),
        ));
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(SourceError::InvalidFormat(format!(
            "unsupported archive URL '{url}': only http:// and https:// are supported"
        )));
    }
    Ok(url.to_string())
}

/// Lexically normalizes a path while preserving leading `..` components.
pub fn normalize_direct_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let mut normalized = PathBuf::new();
    let mut leading_parents = 0usize;

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(Component::Prefix(prefix)),
            Component::RootDir => normalized.push(Component::RootDir),
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some() && normalized != Path::new("..") {
                    normalized.pop();
                } else if !normalized.has_root() {
                    leading_parents += 1;
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    if leading_parents > 0 {
        let mut result = PathBuf::new();
        for _ in 0..leading_parents {
            result.push("..");
        }
        result.push(normalized);
        result
    } else {
        normalized
    }
}

fn path_to_source_string(path: &Path) -> Result<String, SourceError> {
    let text = path.to_string_lossy();
    if text.is_empty() || has_control_characters(&text) {
        return Err(SourceError::InvalidFormat(
            "local path source is empty or contains control characters".to_string(),
        ));
    }
    Ok(text.into_owned())
}

/// Expresses `target` relative to `base` when both paths share a lexical root.
///
/// Returns `None` when a relative representation is not possible. The result is
/// normalized and suitable for a `path+` source string.
pub fn relative_source_path(base: &Path, target: &Path) -> Option<String> {
    if base.is_absolute() != target.is_absolute() {
        return None;
    }

    let base = normalize_path_components(base);
    let target = normalize_path_components(target);
    let base_parts: Vec<_> = base.components().collect();
    let target_parts: Vec<_> = target.components().collect();

    let mut common = 0usize;
    while common < base_parts.len()
        && common < target_parts.len()
        && base_parts[common] == target_parts[common]
    {
        common += 1;
    }

    let mut result = PathBuf::new();
    for _ in common..base_parts.len() {
        result.push("..");
    }
    for part in &target_parts[common..] {
        result.push(part.as_os_str());
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    path_to_source_string(&result).ok()
}

fn normalize_path_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.file_name().is_some() && out != Path::new("..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// Represents the origin or acquisition source of a locked package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageSource(String);

impl PackageSource {
    /// Creates and validates a package source identifier.
    pub fn new(source: impl Into<String>) -> Result<Self, SourceError> {
        let source = source.into();
        let trimmed = source.trim();
        if trimmed.is_empty() {
            return Err(SourceError::Empty);
        }
        if has_control_characters(trimmed) {
            return Err(SourceError::InvalidFormat(
                "source identity cannot contain control characters".to_string(),
            ));
        }

        if let Some(name) = trimmed.strip_prefix(REGISTRY_PREFIX) {
            if name.trim().is_empty() {
                return Err(SourceError::InvalidFormat(
                    "registry source is missing a registry name".to_string(),
                ));
            }
            return Ok(Self(trimmed.to_string()));
        }

        // Validate direct sources eagerly so malformed lockfiles are rejected at
        // parse time instead of failing much later during acquisition.
        DirectSource::parse(trimmed)?;
        Ok(Self(trimmed.to_string()))
    }

    /// Creates a registry source identifier.
    pub fn registry(name: &str) -> Self {
        Self(format!("{REGISTRY_PREFIX}{name}"))
    }

    /// Creates the default registry source identifier.
    pub fn default_registry() -> Self {
        Self::registry("default")
    }

    /// Creates a legacy Git source with an explicit revision.
    pub fn git(url: &str, rev: &str) -> Self {
        Self::try_git(url, GitRef::Rev(rev.to_string())).expect("validated Git source")
    }

    /// Creates a validated Git source for a requested ref.
    pub fn git_ref(url: &str, requested: GitRef) -> Self {
        Self::try_git(url, requested).expect("validated Git source")
    }

    /// Creates a validated Git source, preserving both the requested ref and the
    /// immutable commit recorded in the lockfile.
    pub fn git_commit(url: &str, requested: GitRef, commit: &str) -> Self {
        Self::try_git_commit(url, requested, commit).expect("validated Git source")
    }

    /// Fallible constructor for untrusted locked Git source input.
    pub fn try_git_commit(url: &str, requested: GitRef, commit: &str) -> Result<Self, SourceError> {
        requested.validate()?;
        let source = DirectSource::Git {
            url: validate_git_url(url)?,
            requested,
            resolved_commit: Some(validate_commit(commit)?),
        };
        Ok(Self(source.to_string_canonical()))
    }

    /// Fallible constructor for untrusted Git source input.
    pub fn try_git(url: &str, requested: GitRef) -> Result<Self, SourceError> {
        requested.validate()?;
        let source = DirectSource::Git {
            url: validate_git_url(url)?,
            requested,
            resolved_commit: None,
        };
        Ok(Self(source.to_string_canonical()))
    }

    /// Creates a validated remote archive source.
    pub fn archive(url: &str, checksum: &Checksum) -> Self {
        Self::try_archive(url, checksum).expect("validated archive source")
    }

    /// Fallible constructor for untrusted archive source input.
    pub fn try_archive(url: &str, checksum: &Checksum) -> Result<Self, SourceError> {
        let source = DirectSource::Archive {
            url: validate_archive_url(url)?,
            checksum: checksum.clone(),
        };
        Ok(Self(source.to_string_canonical()))
    }

    /// Creates a validated local path source.
    pub fn path(path: &str) -> Self {
        Self::try_path(path).expect("validated path source")
    }

    /// Fallible constructor for untrusted local path input.
    pub fn try_path(path: &str) -> Result<Self, SourceError> {
        let normalized = path_to_source_string(&normalize_direct_path(path))?;
        let source = DirectSource::parse(&format!("{PATH_PREFIX}{normalized}"))?;
        Ok(Self(source.to_string_canonical()))
    }

    /// Returns the source as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns true when this source is registry-backed.
    pub fn is_registry(&self) -> bool {
        self.0.starts_with(REGISTRY_PREFIX)
    }

    /// Returns true when this source is a direct Git/archive/path source.
    pub fn is_direct(&self) -> bool {
        !self.is_registry()
    }

    /// Returns the parsed direct source, if this is one.
    pub fn direct(&self) -> Option<DirectSource> {
        DirectSource::parse(&self.0).ok()
    }

    /// Returns a stable identity key, ignoring acquisition-only Git commits.
    pub fn identity_key(&self) -> String {
        if self.is_registry() {
            self.0.clone()
        } else {
            self.direct()
                .map(|source| source.identity_key())
                .unwrap_or_else(|| self.0.clone())
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_git_sources() {
        let source = DirectSource::parse("git+https://example.invalid/lib.git#tag=v1.2.3")
            .expect("valid git source");
        assert_eq!(
            source,
            DirectSource::Git {
                url: "https://example.invalid/lib.git".to_string(),
                requested: GitRef::Tag("v1.2.3".to_string()),
                resolved_commit: None,
            }
        );

        let locked = DirectSource::parse(&format!(
            "git+https://example.invalid/lib.git#tag=v1.2.3#commit={}",
            "a".repeat(40)
        ))
        .expect("valid locked git source");
        assert!(matches!(
            locked,
            DirectSource::Git {
                resolved_commit: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn parses_legacy_git_rev_source() {
        let source = DirectSource::parse("git+https://example.invalid/lib.git?rev=abc123")
            .expect("valid legacy git source");
        assert_eq!(
            source,
            DirectSource::Git {
                url: "https://example.invalid/lib.git".to_string(),
                requested: GitRef::Rev("abc123".to_string()),
                resolved_commit: None,
            }
        );
    }

    #[test]
    fn parses_archive_and_path_sources() {
        let digest = "a".repeat(64);
        let archive = DirectSource::parse(&format!(
            "archive+https://example.invalid/lib.tar.gz#sha256={digest}"
        ))
        .expect("valid archive source");
        assert!(matches!(archive, DirectSource::Archive { .. }));

        let path = DirectSource::parse("path+../local-utils").expect("valid path source");
        assert_eq!(
            path,
            DirectSource::Path {
                path: "../local-utils".to_string()
            }
        );
    }

    #[test]
    fn rejects_ambiguous_and_unsafe_sources() {
        assert!(
            DirectSource::parse("git+https://example.invalid/lib.git#tag=v1#branch=main").is_err()
        );
        assert!(
            DirectSource::parse("git+https://example.invalid/lib.git#tag=--upload-pack=x").is_err()
        );
        assert!(
            DirectSource::parse("archive+ftp://example.invalid/lib.tar.gz#sha256=abc").is_err()
        );
        assert!(DirectSource::parse("archive+https://example.invalid/lib.tar.gz#md5=abc").is_err());
        assert!(PackageSource::new("unknown+thing").is_err());
    }

    #[test]
    fn relative_source_paths_are_normalized() {
        assert_eq!(
            relative_source_path(Path::new("/a/b/project"), Path::new("/a/b/local-utils")),
            Some("../local-utils".to_string())
        );
        assert_eq!(
            relative_source_path(Path::new("/a/b"), Path::new("/a/b/libs/math")),
            Some("libs/math".to_string())
        );
        assert_eq!(
            relative_source_path(Path::new("/a"), Path::new("relative")),
            None
        );
    }
}
