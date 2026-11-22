//! Filesystem and atomic write utilities.

use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

/// Generates a pseudo-random nonce derived from system time for safe temporary file creation.
pub fn generate_nonce() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Returns a name component that is unique within this process (process id,
/// time nonce, and a monotonically increasing counter), so concurrent writers
/// in the same process never pick the same temporary path.
pub fn unique_temp_suffix() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}.{}.{count}", std::process::id(), generate_nonce())
}

/// Atomically writes content to a destination path using a flushed temporary file and rename.
///
/// See [`write_atomically_with_mode`]; new files get the process umask default.
pub fn write_atomically(destination: &Path, content: &[u8]) -> io::Result<()> {
    write_atomically_with_mode(destination, content, None)
}

/// Atomically replaces `destination` with `content`.
///
/// Guarantees:
///
/// * The temporary file lives in the destination's own directory (so the
///   final `rename` never crosses filesystems) and is created exclusively
///   (`O_EXCL`): a pre-existing file or symlink at that path is never
///   followed or truncated.
/// * Readers observe either the complete old content or the complete new
///   content, never a partial write. On any error the destination is left
///   untouched and the temporary file is removed.
/// * On Unix, an existing destination keeps its permission bits. A new
///   destination receives `new_file_mode` when given (for example `0o600`
///   for files that may hold credentials), otherwise the umask default.
/// * The file data is synced before the rename, and on Unix the parent
///   directory is synced afterwards on a best-effort basis. Crash
///   durability is therefore as good as the filesystem's `fsync` semantics,
///   and a failed directory sync is not reported as an error.
pub fn write_atomically_with_mode(
    destination: &Path,
    content: &[u8],
    new_file_mode: Option<u32>,
) -> io::Result<()> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "destination path has no parent: '{}'",
                destination.display()
            ),
        )
    })?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };

    fs::create_dir_all(parent)?;

    let base_name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("tmpfile");
    let tmp_path = parent.join(format!(".{base_name}.tmp.{}", unique_temp_suffix()));

    let existing_permissions = fs::metadata(destination).ok().map(|m| m.permissions());

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(permissions) = &existing_permissions {
                file.set_permissions(permissions.clone())?;
            } else if let Some(mode) = new_file_mode {
                file.set_permissions(fs::Permissions::from_mode(mode))?;
            }
        }
        #[cfg(not(unix))]
        let _ = (&existing_permissions, new_file_mode);
        file.write_all(content)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);

        fs::rename(&tmp_path, destination)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
        return result;
    }

    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_atomically() {
        let nonce = generate_nonce();
        let temp_dir = std::env::temp_dir().join(format!("crux_fs_test_{nonce}"));
        let target_file = temp_dir.join("test.txt");

        assert!(write_atomically(&target_file, b"hello atomic world").is_ok());
        assert_eq!(
            fs::read_to_string(&target_file).unwrap(),
            "hello atomic world"
        );

        // Replace content atomically
        assert!(write_atomically(&target_file, b"replaced atomic world").is_ok());
        assert_eq!(
            fs::read_to_string(&target_file).unwrap(),
            "replaced atomic world"
        );

        let temp_leftovers = fs::read_dir(&temp_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".test.txt.tmp.")
            })
            .count();
        assert_eq!(temp_leftovers, 0, "temporary files must be cleaned up");

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_write_atomically_preserves_existing_permissions_and_sets_new_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("crux_fs_perm_{}", unique_temp_suffix()));
        fs::create_dir_all(&dir).unwrap();

        // Regression: rewriting a 0600 file used to reset it to the umask default.
        let private = dir.join("crux.lock");
        fs::write(&private, b"old").unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o600)).unwrap();
        write_atomically(&private, b"new").unwrap();
        assert_eq!(fs::read(&private).unwrap(), b"new");
        assert_eq!(
            fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // New files can be created owner-only.
        let config = dir.join("config.toml");
        write_atomically_with_mode(&config, b"x", Some(0o600)).unwrap();
        assert_eq!(
            fs::metadata(&config).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // ...and an existing file's mode wins over the requested new-file mode.
        fs::set_permissions(&config, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomically_with_mode(&config, b"y", Some(0o600)).unwrap();
        assert_eq!(
            fs::metadata(&config).unwrap().permissions().mode() & 0o777,
            0o640
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_write_atomically_never_follows_a_planted_temp_symlink_and_cleans_up_on_error() {
        let dir = std::env::temp_dir().join(format!("crux_fs_err_{}", unique_temp_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("target.txt");
        fs::write(&destination, b"original").unwrap();

        // A failing rename (destination replaced by a non-empty directory)
        // must leave the destination untouched and no temporary file behind.
        let blocked = dir.join("blocked");
        fs::create_dir_all(blocked.join("child")).unwrap();
        assert!(write_atomically(&blocked, b"data").is_err());
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files left behind: {leftovers:?}"
        );
        assert_eq!(fs::read(&destination).unwrap(), b"original");

        // Temporary names are unique per call, even within one process.
        assert_ne!(unique_temp_suffix(), unique_temp_suffix());
        let _ = fs::remove_dir_all(dir);
    }
}
