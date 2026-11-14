//! Source acquisition, verification, caching, and extraction subsystem for Crux.

use crate::lock::{Checksum, ChecksumAlgorithm};
use crate::package::PackageId;
use crate::registry::{CruxConfig, PackageDefinition, PackageSourceDefinition, RegistryError};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Default timeout for HTTP downloads (30 seconds).
pub const DEFAULT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum archive download size (500 MB).
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024;

/// Default maximum total uncompressed size limit for extraction (1 GB).
pub const DEFAULT_MAX_UNCOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;

/// Default maximum number of files/entries in an archive (50,000).
pub const DEFAULT_MAX_FILES: usize = 50_000;

/// Completion sentinel file name placed in extracted source directories.
pub const COMPLETION_SENTINEL: &str = ".crux_complete";

/// Supported archive formats for source packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    TarGz,
}

impl ArchiveFormat {
    /// Detects archive format from a URL or filename extension.
    pub fn from_url_or_filename(s: &str) -> Option<Self> {
        let clean = s
            .split('?')
            .next()
            .unwrap_or(s)
            .split('#')
            .next()
            .unwrap_or(s);
        let lower = clean.to_ascii_lowercase();
        if lower.ends_with(".zip") {
            Some(ArchiveFormat::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(ArchiveFormat::TarGz)
        } else if lower.ends_with(".tar") {
            Some(ArchiveFormat::Tar)
        } else {
            None
        }
    }

    /// Detects archive format from magic bytes at the beginning of an archive file.
    pub fn from_magic_bytes(header: &[u8]) -> Option<Self> {
        if header.starts_with(b"PK\x03\x04") {
            Some(ArchiveFormat::Zip)
        } else if header.starts_with(&[0x1f, 0x8b]) {
            Some(ArchiveFormat::TarGz)
        } else if header.len() >= 262 && &header[257..262] == b"ustar" {
            Some(ArchiveFormat::Tar)
        } else {
            None
        }
    }

    /// Detects archive format using URL first, filename second, and file magic bytes third.
    pub fn detect(path: &Path, url: &str) -> Result<Self, AcquisitionError> {
        if let Some(fmt) = Self::from_url_or_filename(url) {
            return Ok(fmt);
        }
        if let Some(fmt) = Self::from_url_or_filename(&path.to_string_lossy()) {
            return Ok(fmt);
        }
        if let Ok(mut f) = fs::File::open(path) {
            let mut header = [0u8; 512];
            if let Ok(n) = f.read(&mut header)
                && let Some(fmt) = Self::from_magic_bytes(&header[..n])
            {
                return Ok(fmt);
            }
        }
        Err(AcquisitionError::UnsupportedArchiveFormat(url.to_string()))
    }
}

/// Safety limits for archive decompression and extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionLimits {
    pub max_uncompressed_bytes: u64,
    pub max_files: usize,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_uncompressed_bytes: DEFAULT_MAX_UNCOMPRESSED_BYTES,
            max_files: DEFAULT_MAX_FILES,
        }
    }
}

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
                    "unsupported URL scheme in '{url}': only http:// and https:// are supported"
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
    pub archive_path: PathBuf,
    pub checksum: Checksum,
}

/// Content-addressed local source and archive cache.
#[derive(Debug, Clone)]
pub struct SourceCache {
    root: PathBuf,
    archives_dir: PathBuf,
    sources_dir: PathBuf,
    tmp_dir: PathBuf,
}

impl SourceCache {
    /// Initializes a new source cache rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, io::Error> {
        let root = root.into();
        let archives_dir = root.join("archives").join("sha256");
        let sources_dir = root.join("sources").join("sha256");
        let tmp_dir = root.join("tmp");

        fs::create_dir_all(&archives_dir)?;
        fs::create_dir_all(&sources_dir)?;
        fs::create_dir_all(&tmp_dir)?;

        Ok(Self {
            root,
            archives_dir,
            sources_dir,
            tmp_dir,
        })
    }

    /// Resolves the default cache directory from environment or user home.
    pub fn default_cache_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("CRUX_CACHE_DIR") {
            let p = PathBuf::from(dir.trim());
            if !p.as_os_str().is_empty() {
                return p;
            }
        }
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            let p = PathBuf::from(xdg.trim());
            if !p.as_os_str().is_empty() {
                return p.join("crux");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home.trim());
            if !p.as_os_str().is_empty() {
                return p.join(".cache").join("crux");
            }
        }
        std::env::temp_dir().join("crux-cache")
    }

    /// Initializes a source cache at the default system location.
    pub fn default_cache() -> Result<Self, io::Error> {
        Self::new(Self::default_cache_dir())
    }

    /// Initializes a source cache using the directory configured in `CruxConfig`,
    /// or falling back to the standard default cache directory.
    pub fn from_config(config: &CruxConfig) -> Result<Self, io::Error> {
        if let Some(ref dir) = config.cache_dir {
            Self::new(dir)
        } else {
            Self::default_cache()
        }
    }

    /// Returns the root path of the cache.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the archives cache directory.
    pub fn archives_dir(&self) -> &Path {
        &self.archives_dir
    }

    /// Returns the sources cache directory.
    pub fn sources_dir(&self) -> &Path {
        &self.sources_dir
    }

    /// Returns the temporary directory for atomic staging.
    pub fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    /// Returns the expected path for a cached archive with given SHA-256 digest.
    pub fn archive_path(&self, digest: &str) -> PathBuf {
        self.archives_dir.join(digest.to_ascii_lowercase())
    }

    /// Returns the expected directory for extracted sources with given SHA-256 digest.
    pub fn source_dir(&self, digest: &str) -> PathBuf {
        self.sources_dir.join(digest.to_ascii_lowercase())
    }

    /// Checks whether an extracted source directory exists and is fully verified.
    pub fn has_valid_source(&self, digest: &str) -> bool {
        let dir = self.source_dir(digest);
        dir.is_dir() && dir.join(COMPLETION_SENTINEL).is_file()
    }

    /// Checks whether a cached archive exists and verifies its SHA-256 hash.
    /// If an existing archive file is corrupted, it is automatically removed.
    pub fn has_valid_archive(&self, digest: &str) -> bool {
        let path = self.archive_path(digest);
        if !path.is_file() {
            return false;
        }

        match compute_file_sha256(&path) {
            Ok(computed) if computed.eq_ignore_ascii_case(digest) => true,
            _ => {
                let _ = fs::remove_file(&path);
                false
            }
        }
    }

    /// Atomically stores an archive from a temporary file into the cache.
    pub fn store_archive(&self, temp_file: &Path, digest: &str) -> Result<PathBuf, io::Error> {
        let dest = self.archive_path(digest);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_replace_file(temp_file, &dest)?;
        Ok(dest)
    }

    /// Atomically stores an extracted directory from a temporary staging directory into the cache.
    pub fn store_source(&self, temp_dir: &Path, digest: &str) -> Result<PathBuf, io::Error> {
        let dest = self.source_dir(digest);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_replace_dir(temp_dir, &dest)?;
        Ok(dest)
    }

    /// Creates a unique temporary file path within the cache temporary directory.
    pub fn create_temp_file(&self, prefix: &str) -> Result<TempFileGuard, io::Error> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let name = format!("{prefix}_{pid}_{nanos}.tmp");
        let path = self.tmp_dir.join(name);
        fs::File::create(&path)?;
        Ok(TempFileGuard::new(path))
    }

    /// Creates a unique temporary staging directory within the cache temporary directory.
    pub fn create_temp_dir(&self, prefix: &str) -> Result<TempDirGuard, io::Error> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let name = format!("{prefix}_{pid}_{nanos}.staged");
        let path = self.tmp_dir.join(name);
        fs::create_dir_all(&path)?;
        Ok(TempDirGuard::new(path))
    }
}

/// Computes the SHA-256 hexadecimal digest of a file.
pub fn compute_file_sha256(path: &Path) -> Result<String, io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 16384];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Verifies that a file's SHA-256 matches an expected checksum.
pub fn verify_file_checksum(path: &Path, expected: &Checksum) -> Result<(), AcquisitionError> {
    if expected.algorithm() != ChecksumAlgorithm::Sha256 {
        return Err(AcquisitionError::InvalidSource(format!(
            "unsupported checksum algorithm: {}",
            expected.algorithm()
        )));
    }
    let actual = compute_file_sha256(path)?;
    if !actual.eq_ignore_ascii_case(expected.digest()) {
        return Err(AcquisitionError::ChecksumMismatch {
            expected: expected.to_string(),
            actual: format!("sha256:{actual}"),
        });
    }
    Ok(())
}

/// Abstract HTTP transport for streaming downloads.
pub trait HttpTransport: Send + Sync {
    /// Streams the body from `url` into `writer`, computing the SHA-256 digest on the fly.
    /// Enforces `max_bytes` download limits and `timeout`.
    /// Returns `(total_bytes, sha256_hex_digest)`.
    fn download_to(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<(u64, String), AcquisitionError>;
}

/// Standard production HTTP transport using `ureq`.
#[derive(Debug, Default, Clone)]
pub struct UreqHttpTransport;

impl HttpTransport for UreqHttpTransport {
    fn download_to(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<(u64, String), AcquisitionError> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(AcquisitionError::UnsupportedScheme(url.to_string()));
        }

        let agent = ureq::AgentBuilder::new()
            .timeout(timeout)
            .redirects(5)
            .build();

        let response = match agent.get(url).call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(code, resp)) => {
                let status_text = resp.status_text().to_string();
                return Err(AcquisitionError::DownloadFailed {
                    url: url.to_string(),
                    status: Some(code),
                    message: status_text,
                });
            }
            Err(ureq::Error::Transport(err)) => {
                return Err(AcquisitionError::DownloadFailed {
                    url: url.to_string(),
                    status: None,
                    message: err.to_string(),
                });
            }
        };

        let mut reader = response.into_reader();
        let mut hasher = Sha256::new();
        let mut total_bytes: u64 = 0;
        let mut buf = [0u8; 16384];

        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| AcquisitionError::DownloadFailed {
                    url: url.to_string(),
                    status: None,
                    message: format!("read error: {e}"),
                })?;
            if n == 0 {
                break;
            }
            total_bytes += n as u64;
            if total_bytes > max_bytes {
                return Err(AcquisitionError::DownloadSizeExceeded { max_bytes });
            }
            hasher.update(&buf[..n]);
            writer.write_all(&buf[..n]).map_err(AcquisitionError::Io)?;
        }

        writer.flush().map_err(AcquisitionError::Io)?;
        let digest = format!("{:x}", hasher.finalize());
        Ok((total_bytes, digest))
    }
}

/// In-memory mock HTTP transport for offline unit and integration testing.
#[derive(Default)]
pub struct MockHttpTransport {
    responses: std::sync::Mutex<std::collections::BTreeMap<String, MockResponse>>,
    request_count: std::sync::atomic::AtomicUsize,
}

/// Encapsulates mocked responses for `MockHttpTransport`.
pub enum MockResponse {
    Success(Vec<u8>),
    Status(u16, String),
    NetworkError(String),
}

impl MockHttpTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a canned successful response for `url`.
    pub fn add_response(&self, url: impl Into<String>, bytes: Vec<u8>) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.into(), MockResponse::Success(bytes));
    }

    /// Registers an HTTP error response for `url`.
    pub fn add_status_error(
        &self,
        url: impl Into<String>,
        status: u16,
        message: impl Into<String>,
    ) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.into(), MockResponse::Status(status, message.into()));
    }

    /// Registers a simulated network/transport error for `url`.
    pub fn add_network_error(&self, url: impl Into<String>, message: impl Into<String>) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.into(), MockResponse::NetworkError(message.into()));
    }

    /// Returns the total number of download requests made through this transport.
    pub fn request_count(&self) -> usize {
        self.request_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl HttpTransport for MockHttpTransport {
    fn download_to(
        &self,
        url: &str,
        writer: &mut dyn Write,
        max_bytes: u64,
        _timeout: Duration,
    ) -> Result<(u64, String), AcquisitionError> {
        self.request_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(AcquisitionError::UnsupportedScheme(url.to_string()));
        }

        let resp_guard = self.responses.lock().unwrap();
        let resp = resp_guard
            .get(url)
            .ok_or_else(|| AcquisitionError::DownloadFailed {
                url: url.to_string(),
                status: Some(404),
                message: "Not Found in MockHttpTransport".to_string(),
            })?;

        match resp {
            MockResponse::Success(bytes) => {
                if (bytes.len() as u64) > max_bytes {
                    return Err(AcquisitionError::DownloadSizeExceeded { max_bytes });
                }
                let mut hasher = Sha256::new();
                hasher.update(bytes);
                let digest = format!("{:x}", hasher.finalize());

                writer.write_all(bytes).map_err(AcquisitionError::Io)?;
                writer.flush().map_err(AcquisitionError::Io)?;
                Ok((bytes.len() as u64, digest))
            }
            MockResponse::Status(code, msg) => Err(AcquisitionError::DownloadFailed {
                url: url.to_string(),
                status: Some(*code),
                message: msg.clone(),
            }),
            MockResponse::NetworkError(msg) => Err(AcquisitionError::DownloadFailed {
                url: url.to_string(),
                status: None,
                message: msg.clone(),
            }),
        }
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
            PackageSourceDefinition::Git { url, .. } => {
                Err(AcquisitionError::InvalidSource(format!(
                    "package '{id}' declares a git source '{url}'; git source acquisition is scheduled for a future milestone"
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
        if !url.starts_with("http://") && !url.starts_with("https://") {
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
        if !force && self.cache.has_valid_source(digest) {
            let source_dir = self.cache.source_dir(digest);
            let archive_path = self.cache.archive_path(digest);
            return Ok(AcquiredSource {
                package_id: id.clone(),
                source_dir,
                archive_path,
                checksum: expected_checksum.clone(),
            });
        }

        // 2. Archive cached and valid: skip download, extract directly
        let archive_path = if !force && self.cache.has_valid_archive(digest) {
            self.cache.archive_path(digest)
        } else {
            // 3. Download archive to temporary file and verify checksum
            let temp_file_guard = self.cache.create_temp_file("archive")?;
            let temp_path = temp_file_guard.path().to_path_buf();

            let mut out_file = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&temp_path)?;

            let (_bytes_downloaded, computed_digest) = self.transport.download_to(
                url,
                &mut out_file,
                self.max_download_bytes,
                self.download_timeout,
            )?;
            drop(out_file);

            if !computed_digest.eq_ignore_ascii_case(digest) {
                // Drop guard cleans up temp file
                return Err(AcquisitionError::ChecksumMismatch {
                    expected: expected_checksum.to_string(),
                    actual: format!("sha256:{computed_digest}"),
                });
            }

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

        // 6. Write completion sentinel marker
        let sentinel_path = staging_path.join(COMPLETION_SENTINEL);
        fs::write(
            &sentinel_path,
            format!("package={id}\nchecksum={expected_checksum}\n"),
        )?;

        // 7. Atomically move staged sources into content-addressed source directory
        let final_source_dir = self.cache.store_source(&staging_path, digest)?;
        let _ = temp_dir_guard.disarm();

        Ok(AcquiredSource {
            package_id: id.clone(),
            source_dir: final_source_dir,
            archive_path,
            checksum: expected_checksum.clone(),
        })
    }
}

/// Validates an entry path from an archive to prevent directory traversal attacks (Zip Slip).
pub fn sanitize_entry_path(path: &Path) -> Result<PathBuf, AcquisitionError> {
    let s = path.to_string_lossy();
    if path.is_absolute()
        || s.starts_with('/')
        || s.starts_with('\\')
        || (s.len() >= 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic())
        || s.contains('\\')
    {
        return Err(AcquisitionError::SecurityViolation(format!(
            "archive entry declares an illegal absolute or Windows drive path: '{}'",
            path.display()
        )));
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(AcquisitionError::SecurityViolation(format!(
                    "archive entry contains forbidden path traversal ('..'): '{}'",
                    path.display()
                )));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(AcquisitionError::SecurityViolation(format!(
                    "archive entry contains forbidden root or prefix component: '{}'",
                    path.display()
                )));
            }
            Component::CurDir => {}
            Component::Normal(c) => clean.push(c),
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(AcquisitionError::SecurityViolation(
            "archive entry specifies an empty path".to_string(),
        ));
    }

    Ok(clean)
}

/// Normalizes a relative path from `base` without touching the filesystem.
fn normalize_relative_path(base: &Path, rel: &Path) -> PathBuf {
    let mut result = base.to_path_buf();
    for component in rel.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::Normal(c) => {
                result.push(c);
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => {}
        }
    }
    result
}

/// Extracts an archive according to its format while applying defense-in-depth safety checks.
pub fn extract_archive(
    archive_path: &Path,
    dest_dir: &Path,
    format: ArchiveFormat,
    limits: &ExtractionLimits,
) -> Result<(), AcquisitionError> {
    match format {
        ArchiveFormat::Zip => extract_zip(archive_path, dest_dir, limits),
        ArchiveFormat::Tar => {
            let file = fs::File::open(archive_path)?;
            extract_tar_reader(file, dest_dir, limits)
        }
        ArchiveFormat::TarGz => {
            let file = fs::File::open(archive_path)?;
            let gz = flate2::read::GzDecoder::new(file);
            extract_tar_reader(gz, dest_dir, limits)
        }
    }
}

/// Extracts a `.zip` archive safely.
fn extract_zip(
    archive_path: &Path,
    dest_dir: &Path,
    limits: &ExtractionLimits,
) -> Result<(), AcquisitionError> {
    let file = fs::File::open(archive_path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        AcquisitionError::ExtractionFailed(format!("failed to read zip archive: {e}"))
    })?;

    let mut total_uncompressed_bytes: u64 = 0;
    let mut entry_count: usize = 0;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| {
            AcquisitionError::ExtractionFailed(format!("failed to read zip entry: {e}"))
        })?;

        entry_count += 1;
        if entry_count > limits.max_files {
            return Err(AcquisitionError::DecompressionBomb(format!(
                "zip archive exceeds maximum allowed file count ({})",
                limits.max_files
            )));
        }

        let raw_name = entry.name().to_string();
        if raw_name.trim().is_empty() {
            continue;
        }

        let sanitized = sanitize_entry_path(Path::new(&raw_name))?;
        let out_path = dest_dir.join(&sanitized);

        if !out_path.starts_with(dest_dir) {
            return Err(AcquisitionError::SecurityViolation(format!(
                "zip entry '{raw_name}' escapes destination directory"
            )));
        }

        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
            set_file_mode(&out_path, 0o755)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&out_path)?;
            let mut buf = [0u8; 16384];
            loop {
                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                total_uncompressed_bytes += n as u64;
                if total_uncompressed_bytes > limits.max_uncompressed_bytes {
                    return Err(AcquisitionError::DecompressionBomb(format!(
                        "zip archive exceeds maximum uncompressed bytes ({} bytes)",
                        limits.max_uncompressed_bytes
                    )));
                }
                outfile.write_all(&buf[..n])?;
            }
            let is_exec = entry.unix_mode().map(|m| m & 0o111 != 0).unwrap_or(false);
            set_file_mode(&out_path, if is_exec { 0o755 } else { 0o644 })?;
        }
    }

    Ok(())
}

/// Extracts a TAR stream safely.
fn extract_tar_reader<R: Read>(
    reader: R,
    dest_dir: &Path,
    limits: &ExtractionLimits,
) -> Result<(), AcquisitionError> {
    let mut archive = tar::Archive::new(reader);
    let mut total_uncompressed_bytes: u64 = 0;
    let mut entry_count: usize = 0;

    let entries = archive.entries().map_err(|e| {
        AcquisitionError::ExtractionFailed(format!("failed to parse tar archive: {e}"))
    })?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| {
            AcquisitionError::ExtractionFailed(format!("failed to read tar entry: {e}"))
        })?;

        entry_count += 1;
        if entry_count > limits.max_files {
            return Err(AcquisitionError::DecompressionBomb(format!(
                "tar archive exceeds maximum allowed file count ({})",
                limits.max_files
            )));
        }

        let path = entry.path().map_err(|e| {
            AcquisitionError::ExtractionFailed(format!("invalid tar entry path: {e}"))
        })?;

        let sanitized = sanitize_entry_path(&path)?;
        let out_path = dest_dir.join(&sanitized);

        if !out_path.starts_with(dest_dir) {
            return Err(AcquisitionError::SecurityViolation(format!(
                "tar entry '{}' escapes destination directory",
                path.display()
            )));
        }

        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            fs::create_dir_all(&out_path)?;
            set_file_mode(&out_path, 0o755)?;
        } else if entry_type.is_file() {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&out_path)?;
            let mut buf = [0u8; 16384];
            loop {
                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                total_uncompressed_bytes += n as u64;
                if total_uncompressed_bytes > limits.max_uncompressed_bytes {
                    return Err(AcquisitionError::DecompressionBomb(format!(
                        "tar archive exceeds maximum uncompressed bytes ({} bytes)",
                        limits.max_uncompressed_bytes
                    )));
                }
                outfile.write_all(&buf[..n])?;
            }
            let mode = entry.header().mode().unwrap_or(0o644);
            let is_exec = mode & 0o111 != 0;
            set_file_mode(&out_path, if is_exec { 0o755 } else { 0o644 })?;
        } else if entry_type.is_symlink() {
            let link_target = entry
                .link_name()
                .map_err(|e| AcquisitionError::ExtractionFailed(e.to_string()))?
                .ok_or_else(|| {
                    AcquisitionError::SecurityViolation(format!(
                        "missing link target for symlink '{}'",
                        path.display()
                    ))
                })?;

            if link_target.is_absolute() || link_target.to_string_lossy().starts_with('/') {
                return Err(AcquisitionError::SecurityViolation(format!(
                    "symlink '{}' points to absolute path '{}'",
                    path.display(),
                    link_target.display()
                )));
            }

            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
                let normalized = normalize_relative_path(parent, &link_target);
                if !normalized.starts_with(dest_dir) {
                    return Err(AcquisitionError::SecurityViolation(format!(
                        "symlink '{}' targets path outside destination: '{}'",
                        path.display(),
                        link_target.display()
                    )));
                }
                #[cfg(unix)]
                {
                    // Remove if existing link/file
                    if out_path.exists() || out_path.is_symlink() {
                        let _ = fs::remove_file(&out_path);
                    }
                    std::os::unix::fs::symlink(&link_target, &out_path)?;
                }
            }
        } else if entry_type.is_hard_link() {
            let link_target = entry
                .link_name()
                .map_err(|e| AcquisitionError::ExtractionFailed(e.to_string()))?
                .ok_or_else(|| {
                    AcquisitionError::SecurityViolation(format!(
                        "missing target for hard link '{}'",
                        path.display()
                    ))
                })?;

            let sanitized_target = sanitize_entry_path(&link_target)?;
            let target_path = dest_dir.join(&sanitized_target);
            if !target_path.starts_with(dest_dir) || !target_path.exists() {
                return Err(AcquisitionError::SecurityViolation(format!(
                    "hard link target '{}' does not exist or escapes destination",
                    link_target.display()
                )));
            }
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if out_path.exists() {
                let _ = fs::remove_file(&out_path);
            }
            fs::hard_link(&target_path, &out_path)?;
        } else {
            return Err(AcquisitionError::SecurityViolation(format!(
                "unsupported entry type '{:?}' for entry '{}'",
                entry_type,
                path.display()
            )));
        }
    }

    Ok(())
}

/// Sets safe Unix file permissions, stripping setuid, setgid, and sticky bits.
fn set_file_mode(path: &Path, mode: u32) -> Result<(), io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let safe_mode = mode & 0o777;
        fs::set_permissions(path, fs::Permissions::from_mode(safe_mode))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        let _ = mode;
    }
    Ok(())
}

/// If `dir` contains exactly one entry which is a subdirectory, unwraps its contents to `dir`.
pub fn unwrap_single_top_level_dir(dir: &Path) -> Result<(), io::Error> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        entries.push(entry);
    }

    if entries.len() == 1 && entries[0].file_type()?.is_dir() {
        let single_dir = entries[0].path();
        let sub_entries: Vec<_> = fs::read_dir(&single_dir)?.collect::<Result<_, _>>()?;
        for sub in sub_entries {
            let target = dir.join(sub.file_name());
            fs::rename(sub.path(), target)?;
        }
        let _ = fs::remove_dir(single_dir);
    }

    Ok(())
}

/// Atomically replaces destination file with source file using same-directory rename.
fn atomic_replace_file(src: &Path, dest: &Path) -> Result<(), io::Error> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    // If src and dest are on the same filesystem, direct rename works
    if fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    // Otherwise fallback to copy and atomic replacement in target directory
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let temp_name = format!(
        ".{}.tmp.{}",
        dest.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    );
    let temp_path = parent.join(temp_name);
    fs::copy(src, &temp_path)?;
    fs::rename(&temp_path, dest)?;
    let _ = fs::remove_file(src);
    Ok(())
}

/// Atomically replaces destination directory with source directory.
fn atomic_replace_dir(src: &Path, dest: &Path) -> Result<(), io::Error> {
    if dest.exists() {
        let _ = fs::remove_dir_all(dest);
    }
    if fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    // Fallback: create destination and move files
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            atomic_replace_dir(&entry.path(), &target)?;
        } else {
            atomic_replace_file(&entry.path(), &target)?;
        }
    }
    let _ = fs::remove_dir_all(src);
    Ok(())
}

/// RAII guard for automatic temporary file deletion.
#[derive(Debug)]
pub struct TempFileGuard {
    path: PathBuf,
    active: bool,
}

impl TempFileGuard {
    pub fn new(path: PathBuf) -> Self {
        Self { path, active: true }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn disarm(mut self) -> PathBuf {
        self.active = false;
        self.path.clone()
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.active && self.path.exists() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// RAII guard for automatic temporary directory deletion.
#[derive(Debug)]
pub struct TempDirGuard {
    path: PathBuf,
    active: bool,
}

impl TempDirGuard {
    pub fn new(path: PathBuf) -> Self {
        Self { path, active: true }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn disarm(mut self) -> PathBuf {
        self.active = false;
        self.path.clone()
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if self.active && self.path.exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
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
        assert!(sanitize_entry_path(Path::new("../evil.txt")).is_err());
        assert!(sanitize_entry_path(Path::new("foo/../../evil.txt")).is_err());
        assert!(sanitize_entry_path(Path::new("/etc/passwd")).is_err());
        assert!(sanitize_entry_path(Path::new("\\windows\\system32")).is_err());
        assert!(sanitize_entry_path(Path::new("C:\\boot.ini")).is_err());

        let ok = sanitize_entry_path(Path::new("include/fmt/format.h")).unwrap();
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
