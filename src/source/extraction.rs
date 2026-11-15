//! Archive detection, verification, and directory-traversal-safe extraction.

use super::AcquisitionError;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

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
pub fn normalize_relative_path(base: &Path, rel: &Path) -> PathBuf {
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
pub fn extract_zip(
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
pub fn extract_tar_reader<R: Read>(
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
        if entry_type.is_pax_global_extensions()
            || entry_type.is_pax_local_extensions()
            || entry_type.is_gnu_longname()
            || entry_type.is_gnu_longlink()
        {
            continue;
        }

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
pub fn set_file_mode(path: &Path, mode: u32) -> Result<(), io::Error> {
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
