//! Content-addressed local source and archive cache management.

use super::AcquisitionError;
use super::extraction::COMPLETION_SENTINEL;
use crate::lock::{Checksum, ChecksumAlgorithm};
use crate::registry::CruxConfig;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

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

    /// Reads the provenance sentinel recorded for a cached source tree.
    pub fn source_provenance(&self, digest: &str) -> Option<String> {
        fs::read_to_string(self.source_dir(digest).join(COMPLETION_SENTINEL)).ok()
    }

    /// Checks whether a cached source tree matches an exact provenance marker.
    pub fn has_valid_source_with(&self, digest: &str, expected_provenance: &str) -> bool {
        self.source_provenance(digest)
            .is_some_and(|actual| actual == expected_provenance)
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
    ///
    /// The file is created exclusively (`O_EXCL`), so an existing file or
    /// symlink at the chosen path is never followed or truncated.
    pub fn create_temp_file(&self, prefix: &str) -> Result<TempFileGuard, io::Error> {
        let name = format!(
            "{prefix}_{}.tmp",
            crate::core::filesystem::unique_temp_suffix()
        );
        let path = self.tmp_dir.join(name);
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(TempFileGuard::new(path))
    }

    /// Creates a unique temporary staging directory within the cache temporary directory.
    ///
    /// The directory is created with `create_dir` (not `create_dir_all`), so
    /// an existing path is an error rather than a shared staging area.
    pub fn create_temp_dir(&self, prefix: &str) -> Result<TempDirGuard, io::Error> {
        let name = format!(
            "{prefix}_{}.staged",
            crate::core::filesystem::unique_temp_suffix()
        );
        let path = self.tmp_dir.join(name);
        fs::create_dir_all(&self.tmp_dir)?;
        fs::create_dir(&path)?;
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

/// Atomically replaces destination file with source file using same-directory rename.
pub fn atomic_replace_file(src: &Path, dest: &Path) -> Result<(), io::Error> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    // Cross-filesystem fallback: copy through an exclusively created temp file.
    let content = fs::read(src)?;
    crate::core::filesystem::write_atomically(dest, &content)?;
    let _ = fs::remove_file(src);
    Ok(())
}

/// Copies a directory tree into an existing destination without replacing it.
fn copy_dir_contents(src: &Path, dest: &Path) -> Result<(), io::Error> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_contents(&entry.path(), &target)?;
        } else {
            atomic_replace_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Replaces a destination directory with `src`, preserving the old tree until the
/// replacement is in place so readers never observe a missing cache directory.
pub fn atomic_replace_dir(src: &Path, dest: &Path) -> Result<(), io::Error> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let backup = if fs::symlink_metadata(dest).is_ok() {
        let parent = dest.parent().unwrap_or_else(|| Path::new("."));
        let name = dest.file_name().unwrap_or_default().to_string_lossy();
        let backup = parent.join(format!(
            ".{name}.backup.{}",
            crate::core::filesystem::unique_temp_suffix()
        ));
        fs::rename(dest, &backup)?;
        Some(backup)
    } else {
        None
    };

    match fs::rename(src, dest) {
        Ok(()) => {
            if let Some(backup) = backup {
                let _ = fs::remove_dir_all(backup);
            }
            Ok(())
        }
        // Another writer stored the same content-addressed entry between our
        // existence check and the rename. Keep its complete tree instead of
        // deleting it (readers may already be using it) and discard ours.
        Err(rename_error)
            if backup.is_none()
                && fs::symlink_metadata(dest).is_ok_and(|meta| meta.is_dir())
                && matches!(
                    rename_error.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
                ) =>
        {
            let _ = fs::remove_dir_all(src);
            Ok(())
        }
        Err(rename_error) => {
            let _ = fs::remove_dir_all(dest);
            match copy_dir_contents(src, dest) {
                Ok(()) => {
                    if let Some(backup) = backup {
                        let _ = fs::remove_dir_all(backup);
                    }
                    let _ = fs::remove_dir_all(src);
                    Ok(())
                }
                Err(copy_error) => {
                    if let Some(backup) = backup {
                        let _ = fs::remove_dir_all(dest);
                        let _ = fs::rename(&backup, dest);
                    }
                    Err(if rename_error.kind() == io::ErrorKind::Other {
                        copy_error
                    } else {
                        rename_error
                    })
                }
            }
        }
    }
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
