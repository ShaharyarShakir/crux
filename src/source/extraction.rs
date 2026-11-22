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

/// Returns `true` for an entry that names the archive root itself (`.`, `./`),
/// as emitted by `tar -C dir .`. Only directory entries may be skipped this way.
fn names_archive_root(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::CurDir))
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
///
/// Every entry is written with the following invariants, which together make
/// symlink-chain and hard-link escapes impossible regardless of entry order:
///
/// 1. No entry is ever created or written *through* a symbolic link: every
///    ancestor directory of an entry is checked with `symlink_metadata` and
///    created one level at a time, so a previously extracted symlink can never
///    redirect a later write.
/// 2. An existing file, symlink, or hard link at an entry's own path is
///    unlinked first and the replacement is created with `O_EXCL`
///    (`create_new`), so overwriting never follows a link or truncates an
///    inode shared with another path.
/// 3. Symlink targets are resolved *physically* (following existing links)
///    when created, and every created symlink is re-validated after the whole
///    archive has been extracted, catching chains whose meaning only changes
///    once later links exist.
/// 4. Hard links may only reference regular files already extracted into the
///    destination, reached without traversing any symlink.
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

/// Write-side boundary enforcement shared by the zip and tar extractors.
struct ExtractionRoot {
    /// Canonical (symlink-free) destination directory.
    root: PathBuf,
    /// Relative paths of every symlink created during extraction.
    symlinks: Vec<PathBuf>,
}

impl ExtractionRoot {
    fn new(dest_dir: &Path) -> Result<Self, AcquisitionError> {
        fs::create_dir_all(dest_dir)?;
        Ok(Self {
            root: fs::canonicalize(dest_dir)?,
            symlinks: Vec::new(),
        })
    }

    /// Ensures every ancestor directory of `relative` exists as a real
    /// directory (never a symlink), creating missing ones one level at a time.
    /// Returns the absolute path of the entry itself.
    fn prepare_parents(&self, relative: &Path) -> Result<PathBuf, AcquisitionError> {
        let mut current = self.root.clone();
        let components: Vec<_> = relative.components().collect();
        let Some((_, parents)) = components.split_last() else {
            return Err(AcquisitionError::SecurityViolation(
                "archive entry specifies an empty path".to_string(),
            ));
        };
        for component in parents {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(AcquisitionError::SecurityViolation(format!(
                        "archive entry '{}' traverses symbolic link '{}'",
                        relative.display(),
                        self.display_relative(&current)
                    )));
                }
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => {
                    return Err(AcquisitionError::ExtractionFailed(format!(
                        "archive entry '{}' requires '{}' to be a directory, but it is a file",
                        relative.display(),
                        self.display_relative(&current)
                    )));
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&current)?;
                    set_file_mode(&current, 0o755)?;
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(self.root.join(relative))
    }

    /// Unlinks an existing non-directory at `path` so a replacement can be
    /// created exclusively. Refuses to replace a directory with a non-directory.
    fn clear_existing(&self, path: &Path, relative: &Path) -> Result<(), AcquisitionError> {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.is_dir() => Err(AcquisitionError::ExtractionFailed(format!(
                "archive entry '{}' would replace an existing directory",
                relative.display()
            ))),
            Ok(_) => {
                fs::remove_file(path)?;
                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn create_directory(&self, relative: &Path) -> Result<(), AcquisitionError> {
        let out_path = self.prepare_parents(relative)?;
        match fs::symlink_metadata(&out_path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(AcquisitionError::SecurityViolation(format!(
                    "directory entry '{}' collides with an existing symbolic link",
                    relative.display()
                )));
            }
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(AcquisitionError::ExtractionFailed(format!(
                    "directory entry '{}' collides with an existing file",
                    relative.display()
                )));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => fs::create_dir(&out_path)?,
            Err(err) => return Err(err.into()),
        }
        set_file_mode(&out_path, 0o755)?;
        Ok(())
    }

    /// Creates a fresh regular file for `relative`, never following links.
    fn create_file(&self, relative: &Path) -> Result<(PathBuf, fs::File), AcquisitionError> {
        let out_path = self.prepare_parents(relative)?;
        self.clear_existing(&out_path, relative)?;
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out_path)?;
        Ok((out_path, file))
    }

    fn create_symlink(&mut self, relative: &Path, target: &Path) -> Result<(), AcquisitionError> {
        if target.is_absolute() || target.has_root() || target.as_os_str().is_empty() {
            return Err(AcquisitionError::SecurityViolation(format!(
                "symlink '{}' points to absolute or empty path '{}'",
                relative.display(),
                target.display()
            )));
        }
        let out_path = self.prepare_parents(relative)?;
        self.validate_link_target(relative, target)?;
        self.clear_existing(&out_path, relative)?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, &out_path)?;
            self.symlinks.push(relative.to_path_buf());
        }
        #[cfg(not(unix))]
        {
            let _ = out_path;
        }
        Ok(())
    }

    fn create_hard_link(
        &self,
        relative: &Path,
        link_target: &Path,
    ) -> Result<(), AcquisitionError> {
        let sanitized_target = sanitize_entry_path(link_target)?;
        let mut current = self.root.clone();
        for component in sanitized_target.components() {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(AcquisitionError::SecurityViolation(format!(
                        "hard link '{}' target '{}' traverses or is a symbolic link",
                        relative.display(),
                        link_target.display()
                    )));
                }
                Ok(_) => {}
                Err(_) => {
                    return Err(AcquisitionError::SecurityViolation(format!(
                        "hard link '{}' target '{}' does not exist in the destination",
                        relative.display(),
                        link_target.display()
                    )));
                }
            }
        }
        if !fs::symlink_metadata(&current)?.is_file() {
            return Err(AcquisitionError::SecurityViolation(format!(
                "hard link '{}' target '{}' is not a regular file",
                relative.display(),
                link_target.display()
            )));
        }
        let out_path = self.prepare_parents(relative)?;
        if out_path == current {
            return Err(AcquisitionError::SecurityViolation(format!(
                "hard link '{}' refers to itself",
                relative.display()
            )));
        }
        self.clear_existing(&out_path, relative)?;
        fs::hard_link(&current, &out_path)?;
        Ok(())
    }

    /// Checks that a symlink at `relative` with `target` resolves (physically,
    /// following existing links) to a location inside the destination.
    fn validate_link_target(&self, relative: &Path, target: &Path) -> Result<(), AcquisitionError> {
        let parent = self
            .root
            .join(relative.parent().unwrap_or_else(|| Path::new("")));
        // Lexical pre-check keeps the long-standing error for obvious escapes.
        if !normalize_relative_path(&parent, target).starts_with(&self.root) {
            return Err(AcquisitionError::SecurityViolation(format!(
                "symlink '{}' targets path outside destination: '{}'",
                relative.display(),
                target.display()
            )));
        }
        let resolved = crate::core::paths::resolve_physical_path(&parent, target).map_err(|e| {
            AcquisitionError::SecurityViolation(format!(
                "symlink '{}' target '{}' cannot be resolved safely: {e}",
                relative.display(),
                target.display()
            ))
        })?;
        if !resolved.starts_with(&self.root) {
            return Err(AcquisitionError::SecurityViolation(format!(
                "symlink '{}' resolves outside destination through a symlink chain: '{}'",
                relative.display(),
                target.display()
            )));
        }
        Ok(())
    }

    /// Re-validates every created symlink against the final extracted tree.
    fn finish(self) -> Result<(), AcquisitionError> {
        for relative in &self.symlinks {
            let path = self.root.join(relative);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let target = fs::read_link(&path)?;
                    self.validate_link_target(relative, &target)?;
                }
                // Replaced by a later entry, which was validated on its own.
                _ => {}
            }
        }
        Ok(())
    }

    fn display_relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

fn copy_limited(
    reader: &mut dyn Read,
    outfile: &mut fs::File,
    total_uncompressed_bytes: &mut u64,
    limits: &ExtractionLimits,
    kind: &str,
) -> Result<(), AcquisitionError> {
    let mut buf = [0u8; 16384];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        *total_uncompressed_bytes += n as u64;
        if *total_uncompressed_bytes > limits.max_uncompressed_bytes {
            return Err(AcquisitionError::DecompressionBomb(format!(
                "{kind} archive exceeds maximum uncompressed bytes ({} bytes)",
                limits.max_uncompressed_bytes
            )));
        }
        outfile.write_all(&buf[..n])?;
    }
    Ok(())
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
    let root = ExtractionRoot::new(dest_dir)?;

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
        if raw_name.trim().is_empty()
            || (entry.is_dir() && names_archive_root(Path::new(&raw_name)))
        {
            continue;
        }

        let sanitized = sanitize_entry_path(Path::new(&raw_name))?;

        if entry.is_dir() {
            root.create_directory(&sanitized)?;
        } else {
            // Zip symlink entries are deliberately materialized as regular
            // files containing the link text; no links are created from zips.
            let (out_path, mut outfile) = root.create_file(&sanitized)?;
            copy_limited(
                &mut entry,
                &mut outfile,
                &mut total_uncompressed_bytes,
                limits,
                "zip",
            )?;
            let is_exec = entry.unix_mode().map(|m| m & 0o111 != 0).unwrap_or(false);
            set_file_mode(&out_path, if is_exec { 0o755 } else { 0o644 })?;
        }
    }

    root.finish()
}

/// Extracts a TAR stream safely.
pub fn extract_tar_reader<R: Read>(
    reader: R,
    dest_dir: &Path,
    limits: &ExtractionLimits,
) -> Result<(), AcquisitionError> {
    let mut archive = tar::Archive::new(reader);
    let mut root = ExtractionRoot::new(dest_dir)?;
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

        let entry_type = entry.header().entry_type();
        if entry_type.is_pax_global_extensions()
            || entry_type.is_pax_local_extensions()
            || entry_type.is_gnu_longname()
            || entry_type.is_gnu_longlink()
        {
            continue;
        }

        let path = entry
            .path()
            .map_err(|e| {
                AcquisitionError::ExtractionFailed(format!("invalid tar entry path: {e}"))
            })?
            .into_owned();
        if entry_type.is_dir() && names_archive_root(&path) {
            continue;
        }
        let sanitized = sanitize_entry_path(&path)?;

        if entry_type.is_dir() {
            root.create_directory(&sanitized)?;
        } else if entry_type.is_file() {
            let (out_path, mut outfile) = root.create_file(&sanitized)?;
            copy_limited(
                &mut entry,
                &mut outfile,
                &mut total_uncompressed_bytes,
                limits,
                "tar",
            )?;
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
                })?
                .into_owned();
            root.create_symlink(&sanitized, &link_target)?;
        } else if entry_type.is_hard_link() {
            let link_target = entry
                .link_name()
                .map_err(|e| AcquisitionError::ExtractionFailed(e.to_string()))?
                .ok_or_else(|| {
                    AcquisitionError::SecurityViolation(format!(
                        "missing target for hard link '{}'",
                        path.display()
                    ))
                })?
                .into_owned();
            root.create_hard_link(&sanitized, &link_target)?;
        } else {
            return Err(AcquisitionError::SecurityViolation(format!(
                "unsupported entry type '{:?}' for entry '{}'",
                entry_type,
                path.display()
            )));
        }
    }

    root.finish()
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

/// If `dir` contains exactly one non-hidden entry which is a subdirectory,
/// unwraps its contents to `dir`.
///
/// The wrapper directory is first moved aside under a unique name, so a
/// nested root such as `pkg/pkg/...` unwraps correctly. Moving never
/// overwrites: if an unwrapped entry would replace an existing top-level
/// entry (for example a hidden file present both at the archive root and in
/// the wrapper directory), unwrapping fails instead of silently discarding
/// either file.
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

    if entries.len() != 1 || !entries[0].file_type()?.is_dir() {
        return Ok(());
    }

    let wrapper = dir.join(format!(
        ".crux-unwrap-{}-{}",
        std::process::id(),
        crate::core::filesystem::generate_nonce()
    ));
    if fs::symlink_metadata(&wrapper).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "temporary unwrap directory '{}' already exists",
                wrapper.display()
            ),
        ));
    }
    fs::rename(entries[0].path(), &wrapper)?;

    let mut sub_entries: Vec<_> = fs::read_dir(&wrapper)?.collect::<Result<_, _>>()?;
    sub_entries.sort_by_key(|entry| entry.file_name());
    for sub in &sub_entries {
        let target = dir.join(sub.file_name());
        if fs::symlink_metadata(&target).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "archive entry '{}' exists both at the archive root and inside its top-level directory; refusing to overwrite it",
                    sub.file_name().to_string_lossy()
                ),
            ));
        }
        fs::rename(sub.path(), target)?;
    }
    fs::remove_dir(&wrapper)?;

    Ok(())
}
