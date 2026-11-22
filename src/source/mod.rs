//! Source acquisition, verification, caching, and extraction subsystem for Crux.

pub mod cache;
pub mod direct;
pub mod download;
pub mod events;
pub mod extraction;
pub mod tree;

pub use cache::{
    SourceCache, TempDirGuard, TempFileGuard, compute_file_sha256, verify_file_checksum,
};
pub use direct::{ActiveCatalog, DirectCatalog, LockedChecksumStatus};
pub use download::{
    DEFAULT_DOWNLOAD_TIMEOUT, DEFAULT_MAX_DOWNLOAD_BYTES, HttpTransport, MockHttpTransport,
    MockResponse, UreqHttpTransport,
};
pub use events::{AcquisitionEvent, AcquisitionObserver, RecordingObserver};
pub use extraction::{
    ArchiveFormat, COMPLETION_SENTINEL, DEFAULT_MAX_FILES, DEFAULT_MAX_UNCOMPRESSED_BYTES,
    ExtractionLimits, extract_archive, normalize_relative_path, sanitize_entry_path,
    unwrap_single_top_level_dir,
};
pub use tree::{compute_legacy_tree_sha256_v1, compute_tree_sha256, copy_validated_tree};

use crate::lock::{Checksum, ChecksumAlgorithm};
use crate::package::PackageId;
use crate::registry::{GitOps, PackageDefinition, PackageSourceDefinition, RegistryError};
use crate::resolution::{GitRef, PackageSource};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Errors occurring during source acquisition, verification, or extraction.
#[derive(Debug)]
pub enum AcquisitionError {
    Io(io::Error),
    Registry(Box<RegistryError>),
    PackageNotFound(String),
    InvalidSource(String),
    UnsupportedScheme(String),
    UnsupportedArchiveFormat(String),
    DownloadFailed {
        url: String,
        status: Option<u16>,
        message: String,
    },
    DownloadSizeExceeded {
        max_bytes: u64,
    },
    ChecksumMismatch {
        expected: String,
        actual: String,
    },
    SecurityViolation(String),
    DecompressionBomb(String),
    ExtractionFailed(String),
    GitOperation(String),
    PackageMetadata(String),
    SourceConflict(String),
    SourceCycle(String),
    /// Acquired content does not match the checksum recorded in `crux.lock`.
    LockedChecksumMismatch {
        package: String,
        source: String,
        locked: String,
        actual: String,
    },
}

impl fmt::Display for AcquisitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcquisitionError::Io(err) => write!(f, "I/O error during source acquisition: {err}"),
            AcquisitionError::Registry(err) => write!(f, "registry error: {err}"),
            AcquisitionError::PackageNotFound(msg) => write!(f, "package not found: {msg}"),
            AcquisitionError::InvalidSource(msg) => write!(f, "invalid package source: {msg}"),
            AcquisitionError::UnsupportedScheme(url) => {
                write!(
                    f,
                    "unsupported URL scheme in '{url}': only https:// is supported"
                )
            }
            AcquisitionError::UnsupportedArchiveFormat(url) => {
                write!(
                    f,
                    "unsupported archive format for '{url}': expected .zip, .tar.gz, .tgz, or .tar"
                )
            }
            AcquisitionError::DownloadFailed {
                url,
                status,
                message,
            } => {
                if let Some(code) = status {
                    write!(f, "failed to download '{url}': HTTP {code} ({message})")
                } else {
                    write!(f, "failed to download '{url}': {message}")
                }
            }
            AcquisitionError::DownloadSizeExceeded { max_bytes } => {
                write!(
                    f,
                    "download exceeded maximum allowed size of {max_bytes} bytes"
                )
            }
            AcquisitionError::ChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "checksum verification failed: expected '{expected}', computed '{actual}'"
                )
            }
            AcquisitionError::SecurityViolation(msg) => {
                write!(f, "archive security violation: {msg}")
            }
            AcquisitionError::DecompressionBomb(msg) => {
                write!(f, "decompression limit exceeded (potential bomb): {msg}")
            }
            AcquisitionError::ExtractionFailed(msg) => {
                write!(f, "archive extraction failed: {msg}")
            }
            AcquisitionError::GitOperation(msg) => write!(f, "git operation failed: {msg}"),
            AcquisitionError::PackageMetadata(msg) => write!(f, "invalid package metadata: {msg}"),
            AcquisitionError::SourceConflict(msg) => write!(f, "source conflict: {msg}"),
            AcquisitionError::SourceCycle(msg) => write!(f, "source cycle detected: {msg}"),
            AcquisitionError::LockedChecksumMismatch {
                package,
                source,
                locked,
                actual,
            } => write!(
                f,
                "checksum for package '{package}' from '{source}' does not match crux.lock: \
                 locked '{locked}', found '{actual}'. The content of an already-locked \
                 version changed; if this change is expected, re-resolve with \
                 `crux lock --update`"
            ),
        }
    }
}

impl std::error::Error for AcquisitionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AcquisitionError::Io(err) => Some(err),
            AcquisitionError::Registry(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for AcquisitionError {
    fn from(err: io::Error) -> Self {
        AcquisitionError::Io(err)
    }
}

impl From<RegistryError> for AcquisitionError {
    fn from(err: RegistryError) -> Self {
        AcquisitionError::Registry(Box::new(err))
    }
}

/// Represents an acquired and verified package source directory and archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquiredSource {
    pub package_id: PackageId,
    pub source_dir: PathBuf,
    /// Present only for archive-backed sources. Git and local-path sources have
    /// no archive artifact; their `checksum` is a deterministic source-tree digest.
    pub archive_path: Option<PathBuf>,
    /// Immutable commit resolved during Git acquisition, when applicable.
    pub resolved_git_commit: Option<String>,
    pub checksum: Checksum,
}

impl AcquiredSource {
    /// Returns the archive path when this source was acquired from an archive.
    pub fn archive(&self) -> Option<&Path> {
        self.archive_path.as_deref()
    }
}

/// Orchestrates package source acquisition, validation, verification, and extraction.
#[derive(Clone)]
pub struct SourceAcquirer {
    cache: SourceCache,
    transport: Arc<dyn HttpTransport>,
    limits: ExtractionLimits,
    download_timeout: Duration,
    max_download_bytes: u64,
    observer: Option<Arc<dyn AcquisitionObserver>>,
}

impl SourceAcquirer {
    /// Creates a new `SourceAcquirer` with default production HTTP transport and default limits.
    pub fn new(cache: SourceCache) -> Self {
        Self {
            cache,
            transport: Arc::new(UreqHttpTransport),
            limits: ExtractionLimits::default(),
            download_timeout: DEFAULT_DOWNLOAD_TIMEOUT,
            max_download_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
            observer: None,
        }
    }

    /// Creates a new `SourceAcquirer` with a custom HTTP transport (useful for offline tests).
    pub fn with_transport(cache: SourceCache, transport: Arc<dyn HttpTransport>) -> Self {
        Self {
            cache,
            transport,
            limits: ExtractionLimits::default(),
            download_timeout: DEFAULT_DOWNLOAD_TIMEOUT,
            max_download_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
            observer: None,
        }
    }

    /// Sets the extraction security limits.
    pub fn with_limits(mut self, limits: ExtractionLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets the maximum download size in bytes.
    pub fn with_max_download_bytes(mut self, max_bytes: u64) -> Self {
        self.max_download_bytes = max_bytes;
        self
    }

    /// Sets the network download timeout.
    /// Reports acquisition progress and cache status to `observer`.
    pub fn with_observer(mut self, observer: Arc<dyn AcquisitionObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    fn emit(&self, id: &PackageId, event: AcquisitionEvent) {
        if let Some(observer) = &self.observer {
            observer.on_event(id, &event);
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.download_timeout = timeout;
        self
    }

    /// Returns a reference to the underlying source cache.
    pub fn cache(&self) -> &SourceCache {
        &self.cache
    }

    /// Acquires, verifies, and extracts a package given its `PackageDefinition`.
    pub fn acquire_package_definition(
        &self,
        id: &PackageId,
        definition: &PackageDefinition,
        force: bool,
    ) -> Result<AcquiredSource, AcquisitionError> {
        match &definition.source {
            PackageSourceDefinition::Archive { url, checksum } => {
                self.acquire_archive(id, url, checksum, force)
            }
            PackageSourceDefinition::Git { .. } | PackageSourceDefinition::Path { .. } => {
                Err(AcquisitionError::InvalidSource(format!(
                    "package '{id}' declares a direct source that requires project-relative resolution; use acquire_direct instead"
                )))
            }
        }
    }

    /// Acquires an archive from `url`, verifies it against `expected_checksum`, and extracts it.
    pub fn acquire_archive(
        &self,
        id: &PackageId,
        url: &str,
        expected_checksum: &Checksum,
        force: bool,
    ) -> Result<AcquiredSource, AcquisitionError> {
        if !url.starts_with("https://") {
            return Err(AcquisitionError::UnsupportedScheme(url.to_string()));
        }

        if expected_checksum.algorithm() != ChecksumAlgorithm::Sha256 {
            return Err(AcquisitionError::InvalidSource(format!(
                "unsupported checksum algorithm '{}': only sha256 is supported",
                expected_checksum.algorithm()
            )));
        }

        let digest = expected_checksum.digest();

        // 1. Cache hit: source directory already completely extracted and valid
        if !force && self.cached_archive_source_is_valid(digest) {
            self.emit(id, AcquisitionEvent::SourceCacheHit);
            let source_dir = self.cache.source_dir(digest);
            let archive_path = self.cache.archive_path(digest);
            return Ok(AcquiredSource {
                package_id: id.clone(),
                source_dir,
                archive_path: Some(archive_path),
                resolved_git_commit: None,
                checksum: expected_checksum.clone(),
            });
        }

        // 2. Archive cached and valid: skip download, extract directly
        let archive_path = if !force && self.cache.has_valid_archive(digest) {
            self.emit(id, AcquisitionEvent::ArchiveCacheHit);
            self.cache.archive_path(digest)
        } else {
            // 3. Download archive to temporary file and verify checksum
            let temp_file_guard = self.cache.create_temp_file("archive")?;
            let temp_path = temp_file_guard.path().to_path_buf();

            let mut out_file = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&temp_path)?;

            let shown_url = crate::resolution::redact_url_credentials(url);
            let mut started = false;
            let (bytes_downloaded, computed_digest) = self.transport.download_to_with_progress(
                url,
                &mut out_file,
                self.max_download_bytes,
                self.download_timeout,
                &mut |downloaded, total| {
                    if !started {
                        started = true;
                        self.emit(
                            id,
                            AcquisitionEvent::DownloadStarted {
                                url: shown_url.clone(),
                                total,
                            },
                        );
                    }
                    self.emit(id, AcquisitionEvent::DownloadProgress { downloaded, total });
                },
            )?;
            drop(out_file);
            self.emit(
                id,
                AcquisitionEvent::DownloadFinished {
                    bytes: bytes_downloaded,
                },
            );

            if !computed_digest.eq_ignore_ascii_case(digest) {
                // Drop guard cleans up temp file
                return Err(AcquisitionError::ChecksumMismatch {
                    expected: expected_checksum.to_string(),
                    actual: format!("sha256:{computed_digest}"),
                });
            }

            self.emit(
                id,
                AcquisitionEvent::ChecksumVerified {
                    checksum: expected_checksum.to_string(),
                },
            );

            // Atomically store verified archive into cache
            let saved_path = self.cache.store_archive(&temp_path, digest)?;
            let _ = temp_file_guard.disarm();
            saved_path
        };

        // 4. Safe extraction into temporary staging directory
        let temp_dir_guard = self.cache.create_temp_dir("source")?;
        let staging_path = temp_dir_guard.path().to_path_buf();

        let format = ArchiveFormat::detect(&archive_path, url)?;
        extract_archive(&archive_path, &staging_path, format, &self.limits)?;

        // 5. Unwrap single common top-level directory if present
        unwrap_single_top_level_dir(&staging_path)?;

        // 6. Record the extracted tree digest and write the completion sentinel marker
        let tree_digest = compute_tree_sha256(&staging_path)?;
        let sentinel_path = staging_path.join(COMPLETION_SENTINEL);
        fs::write(
            &sentinel_path,
            format!("package={id}\nchecksum={expected_checksum}\ntree={tree_digest}\n"),
        )?;

        // 7. Atomically move staged sources into content-addressed source directory
        let final_source_dir = self.cache.store_source(&staging_path, digest)?;
        let _ = temp_dir_guard.disarm();
        self.emit(id, AcquisitionEvent::Extracted);

        Ok(AcquiredSource {
            package_id: id.clone(),
            source_dir: final_source_dir,
            archive_path: Some(archive_path),
            resolved_git_commit: None,
            checksum: expected_checksum.clone(),
        })
    }

    /// Acquires a direct Git, archive, or local-path package source.
    ///
    /// `base_dir` is the project directory used to resolve relative path sources.
    pub fn acquire_direct(
        &self,
        id: &PackageId,
        source: &PackageSourceDefinition,
        base_dir: &Path,
        force: bool,
    ) -> Result<AcquiredSource, AcquisitionError> {
        match source {
            PackageSourceDefinition::Archive { url, checksum } => {
                self.acquire_archive(id, url, checksum, force)
            }
            PackageSourceDefinition::Git {
                url,
                requested,
                resolved_commit,
            } => self.acquire_git(id, url, requested, resolved_commit.as_deref(), force),
            PackageSourceDefinition::Path { path } => self.acquire_path(id, path, base_dir, force),
        }
    }

    fn acquire_git(
        &self,
        id: &PackageId,
        url: &str,
        requested: &GitRef,
        resolved_commit: Option<&str>,
        force: bool,
    ) -> Result<AcquiredSource, AcquisitionError> {
        let validated_url = crate::resolution::validate_git_url(url)
            .map_err(|err| AcquisitionError::InvalidSource(err.to_string()))?;
        // Only an immutable commit may identify a cached Git snapshot. Mutable
        // refs (branches, tags, HEAD, rev expressions) are always re-resolved
        // against the remote first, then looked up by the commit they name.
        let known_commit =
            resolved_commit
                .map(str::to_ascii_lowercase)
                .or_else(|| match requested {
                    GitRef::Commit(commit) if is_full_commit_hash(commit) => {
                        Some(commit.to_ascii_lowercase())
                    }
                    _ => None,
                });

        if !force && let Some(commit) = &known_commit {
            let pinned = PackageSource::git_commit(&validated_url, requested.clone(), commit);
            let pinned_key = sha256_text(pinned.as_str());
            if let Some(checksum) = self.cached_tree_checksum(&pinned_key, &pinned) {
                self.emit(
                    id,
                    AcquisitionEvent::GitCommit {
                        commit: commit.clone(),
                        cached: true,
                    },
                );
                return Ok(AcquiredSource {
                    package_id: id.clone(),
                    source_dir: self.cache.source_dir(&pinned_key),
                    archive_path: None,
                    resolved_git_commit: Some(commit.clone()),
                    checksum,
                });
            }
        }

        self.emit(
            id,
            AcquisitionEvent::GitCloning {
                url: crate::resolution::redact_url_credentials(&validated_url),
            },
        );
        let clone_guard = self.cache.create_temp_dir("git")?;
        let clone_dir = clone_guard.path().join("repo");
        let clone_ref = match &known_commit {
            Some(commit) => GitRef::Commit(commit.clone()),
            None => requested.clone(),
        };
        let commit = GitOps::clone_and_pin_commit(
            &validated_url,
            &clone_dir,
            &clone_ref,
            Duration::from_secs(60),
        )
        .map_err(|err| AcquisitionError::GitOperation(err.to_string()))?;
        if let Some(expected) = &known_commit
            && commit != *expected
        {
            return Err(AcquisitionError::GitOperation(format!(
                "Git commit '{expected}' resolved to unexpected commit '{commit}'"
            )));
        }

        self.emit(
            id,
            AcquisitionEvent::GitCommit {
                commit: commit.clone(),
                cached: false,
            },
        );
        let locked_source = PackageSource::git_commit(&validated_url, requested.clone(), &commit);
        let final_key = sha256_text(locked_source.as_str());
        let final_dir = self.cache.source_dir(&final_key);

        if !force && let Some(checksum) = self.cached_tree_checksum(&final_key, &locked_source) {
            return Ok(AcquiredSource {
                package_id: id.clone(),
                source_dir: final_dir,
                archive_path: None,
                resolved_git_commit: Some(commit.clone()),
                checksum,
            });
        }

        let staging_guard = self.cache.create_temp_dir("git-source")?;
        let staging_dir = staging_guard.path();
        copy_validated_tree(&clone_dir, staging_dir)?;
        let tree_digest = compute_tree_sha256(staging_dir)?;
        let locked_provenance = format!("source={locked_source}\ntree={tree_digest}\n");
        fs::write(staging_dir.join(COMPLETION_SENTINEL), &locked_provenance)?;

        let checksum = Checksum::new_sha256(&tree_digest)
            .map_err(|err| AcquisitionError::InvalidSource(err.to_string()))?;
        let source_dir = self.cache.store_source(staging_dir, &final_key)?;
        let _ = staging_guard.disarm();

        Ok(AcquiredSource {
            package_id: id.clone(),
            source_dir,
            archive_path: None,
            resolved_git_commit: Some(commit),
            checksum,
        })
    }

    fn acquire_path(
        &self,
        id: &PackageId,
        path: &str,
        base_dir: &Path,
        force: bool,
    ) -> Result<AcquiredSource, AcquisitionError> {
        let normalized = crate::resolution::normalize_direct_path(path);
        let candidate = if normalized.is_absolute() {
            normalized
        } else {
            base_dir.join(normalized)
        };
        let canonical = fs::canonicalize(&candidate).map_err(|err| {
            AcquisitionError::InvalidSource(format!(
                "local package path '{}' cannot be resolved: {err}",
                candidate.display()
            ))
        })?;
        if !canonical.is_dir() {
            return Err(AcquisitionError::InvalidSource(format!(
                "local package path '{}' is not a directory",
                canonical.display()
            )));
        }

        // Local paths are mutable working copies. Builds never read them in
        // place: the tree is snapshotted into the content-addressed cache, the
        // snapshot itself is hashed, and the snapshot is what gets built.
        let live_digest = compute_tree_sha256(&canonical)?;
        let provenance_source = PackageSource::path(path);
        let provenance = format!("source={provenance_source}\ntree={live_digest}\n");
        let cache_key = sha256_text(&provenance);
        let checksum = Checksum::new_sha256(&live_digest)
            .map_err(|err| AcquisitionError::InvalidSource(err.to_string()))?;

        let cached_dir = self.cache.source_dir(&cache_key);
        let cached_is_valid = !force
            && self.cache.has_valid_source_with(&cache_key, &provenance)
            && compute_tree_sha256(&cached_dir)
                .is_ok_and(|digest| digest.eq_ignore_ascii_case(&live_digest));

        self.emit(
            id,
            AcquisitionEvent::PathSnapshot {
                reused: cached_is_valid,
            },
        );
        if !cached_is_valid {
            let staging_guard = self.cache.create_temp_dir("path-source")?;
            copy_validated_tree(&canonical, staging_guard.path())?;
            let snapshot_digest = compute_tree_sha256(staging_guard.path())?;
            if !snapshot_digest.eq_ignore_ascii_case(&live_digest) {
                return Err(AcquisitionError::InvalidSource(format!(
                    "local package path '{}' changed while it was being snapshotted; retry once it is no longer being modified",
                    canonical.display()
                )));
            }
            fs::write(staging_guard.path().join(COMPLETION_SENTINEL), &provenance)?;
            self.cache.store_source(staging_guard.path(), &cache_key)?;
            let _ = staging_guard.disarm();
        }

        Ok(AcquiredSource {
            package_id: id.clone(),
            source_dir: cached_dir,
            archive_path: None,
            resolved_git_commit: None,
            checksum,
        })
    }

    /// Verifies that a cached archive extraction still matches its recorded tree digest.
    fn cached_archive_source_is_valid(&self, digest: &str) -> bool {
        if !self.cache.has_valid_source(digest) {
            return false;
        }
        let Some(provenance) = self.cache.source_provenance(digest) else {
            return false;
        };
        let Some(recorded_tree) = provenance
            .lines()
            .find_map(|line| line.strip_prefix("tree="))
        else {
            return false;
        };
        let Ok(actual_tree) = compute_tree_sha256(&self.cache.source_dir(digest)) else {
            return false;
        };
        actual_tree.eq_ignore_ascii_case(recorded_tree)
    }

    fn cached_tree_checksum(
        &self,
        cache_key: &str,
        expected_source: &PackageSource,
    ) -> Option<Checksum> {
        let provenance = self.cache.source_provenance(cache_key)?;
        let source_matches = provenance
            .lines()
            .any(|line| line == format!("source={expected_source}"));
        if !source_matches {
            return None;
        }
        let tree = provenance
            .lines()
            .find_map(|line| line.strip_prefix("tree="))?;
        let actual_tree = compute_tree_sha256(&self.cache.source_dir(cache_key)).ok()?;
        if !actual_tree.eq_ignore_ascii_case(tree) {
            return None;
        }
        Checksum::new_sha256(&actual_tree).ok()
    }
}

fn is_full_commit_hash(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn sha256_text(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archive_format_from_url_and_magic() {
        assert_eq!(
            ArchiveFormat::from_url_or_filename("https://example.com/fmt-11.1.1.zip"),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            ArchiveFormat::from_url_or_filename("https://example.com/zlib-1.3.1.tar.gz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_url_or_filename("https://example.com/catch2-3.8.0.tgz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_url_or_filename("https://example.com/lib.tar"),
            Some(ArchiveFormat::Tar)
        );
        assert_eq!(
            ArchiveFormat::from_url_or_filename("https://example.com/lib.unknown"),
            None
        );

        assert_eq!(
            ArchiveFormat::from_magic_bytes(b"PK\x03\x04rest"),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            ArchiveFormat::from_magic_bytes(&[0x1f, 0x8b, 0x08, 0x00]),
            Some(ArchiveFormat::TarGz)
        );
    }

    #[test]
    fn test_sanitize_entry_path_rejects_traversal() {
        assert!(sanitize_entry_path(std::path::Path::new("../evil.txt")).is_err());
        assert!(sanitize_entry_path(std::path::Path::new("foo/../../evil.txt")).is_err());
        assert!(sanitize_entry_path(std::path::Path::new("/etc/passwd")).is_err());
        assert!(sanitize_entry_path(std::path::Path::new("\\windows\\system32")).is_err());
        assert!(sanitize_entry_path(std::path::Path::new("C:\\boot.ini")).is_err());

        let ok = sanitize_entry_path(std::path::Path::new("include/fmt/format.h")).unwrap();
        assert_eq!(ok, PathBuf::from("include/fmt/format.h"));
    }

    #[test]
    fn test_mock_transport_success_and_error() {
        let transport = MockHttpTransport::new();
        let url = "https://example.com/test.zip";
        transport.add_response(url, b"fake zip contents".to_vec());

        let mut output = Vec::new();
        let (bytes, digest) = transport
            .download_to(url, &mut output, 1024, Duration::from_secs(5))
            .unwrap();

        assert_eq!(bytes, 17);
        assert_eq!(output, b"fake zip contents");
        assert_eq!(
            digest,
            "58b184a82c063327f97c38ed97f21acbfb8d4bc50d52b4070b9aed8c06b4bc73"
        );

        let err = transport.download_to(
            "https://example.com/missing.zip",
            &mut Vec::new(),
            1024,
            Duration::from_secs(5),
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_unwrap_single_top_level_dir() {
        let temp = std::env::temp_dir().join("test_crux_unwrap");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        let sub = temp.join("fmt-11.1.1");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("CMakeLists.txt"), b"cmake").unwrap();
        fs::write(sub.join("README.md"), b"readme").unwrap();

        unwrap_single_top_level_dir(&temp).unwrap();

        assert!(temp.join("CMakeLists.txt").is_file());
        assert!(temp.join("README.md").is_file());
        assert!(!sub.exists());

        let _ = fs::remove_dir_all(&temp);
    }
}
