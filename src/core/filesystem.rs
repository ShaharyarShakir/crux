//! Filesystem and atomic write utilities.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Generates a pseudo-random nonce derived from system time for safe temporary file creation.
pub fn generate_nonce() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Atomically writes content to a destination path using a flushed temporary file and rename.
pub fn write_atomically(destination: &Path, content: &[u8]) -> io::Result<()> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "destination path has no parent: '{}'",
                destination.display()
            ),
        )
    })?;

    fs::create_dir_all(parent)?;

    let base_name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("tmpfile");
    let tmp_name = format!(".{base_name}.tmp.{}", generate_nonce());
    let tmp_path = parent.join(tmp_name);

    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(content)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp_path, destination)
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

        let _ = fs::remove_dir_all(temp_dir);
    }
}
