//! Deterministic source-tree validation, hashing, and copying.

use super::AcquisitionError;
use super::extraction::normalize_relative_path;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const TREE_HASH_DOMAIN: &[u8] = b"crux-source-tree-v1\0";

#[derive(Debug, Clone)]
enum TreeEntry {
    File { mode: u32 },
    Symlink { target: PathBuf },
}

fn is_ignored_name(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name == ".git"
        || name == ".crux"
        || name == "target"
        || name == "build"
        || name.starts_with("cmake-build-")
        || name == super::COMPLETION_SENTINEL
}

fn collect_entries(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<PathBuf, TreeEntry>,
) -> Result<(), AcquisitionError> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if is_ignored_name(&name) {
            continue;
        }

        let metadata = fs::symlink_metadata(&path)?;
        let relative = path.strip_prefix(root).map_err(|_| {
            AcquisitionError::SecurityViolation(
                "source entry escaped its root directory".to_string(),
            )
        })?;

        if metadata.is_symlink() {
            let target = fs::read_link(&path)?;
            validate_symlink(root, relative, &target)?;
            out.insert(relative.to_path_buf(), TreeEntry::Symlink { target });
            continue;
        }

        if metadata.is_dir() {
            collect_entries(root, &path, out)?;
            continue;
        }

        if !metadata.is_file() {
            return Err(AcquisitionError::SecurityViolation(format!(
                "unsupported filesystem object in source tree: '{}'",
                relative.display()
            )));
        }

        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o777
        };
        #[cfg(not(unix))]
        let mode = 0o644;

        out.insert(relative.to_path_buf(), TreeEntry::File { mode });
    }

    Ok(())
}

fn validate_symlink(root: &Path, relative: &Path, target: &Path) -> Result<(), AcquisitionError> {
    if target.is_absolute() {
        return Err(AcquisitionError::SecurityViolation(format!(
            "symlink '{}' has an absolute target '{}'",
            relative.display(),
            target.display()
        )));
    }

    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let resolved = normalize_relative_path(parent, target);
    let destination = root.join(&resolved);
    let canonical_root = fs::canonicalize(root)?;
    let canonical_destination = fs::canonicalize(&destination).unwrap_or(destination);
    if !canonical_destination.starts_with(&canonical_root) {
        return Err(AcquisitionError::SecurityViolation(format!(
            "symlink '{}' escapes the source root",
            relative.display()
        )));
    }
    Ok(())
}

fn hash_path(hasher: &mut Sha256, path: &Path) {
    let text = path.to_string_lossy();
    hasher.update((text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
}

/// Computes a deterministic SHA-256 digest over a validated source tree.
pub fn compute_tree_sha256(root: &Path) -> Result<String, AcquisitionError> {
    let mut entries = BTreeMap::new();
    collect_entries(root, root, &mut entries)?;

    let mut hasher = Sha256::new();
    hasher.update(TREE_HASH_DOMAIN);
    hasher.update((entries.len() as u64).to_le_bytes());

    for (relative, entry) in entries {
        hash_path(&mut hasher, &relative);
        match entry {
            TreeEntry::File { mode } => {
                hasher.update(b"F");
                hasher.update(mode.to_le_bytes());
                let bytes = fs::read(root.join(&relative))?;
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            TreeEntry::Symlink { target } => {
                hasher.update(b"L");
                hash_path(&mut hasher, &target);
            }
        }
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Validates and copies a source tree into a destination directory.
pub fn copy_validated_tree(source: &Path, destination: &Path) -> Result<(), AcquisitionError> {
    let mut entries = BTreeMap::new();
    collect_entries(source, source, &mut entries)?;
    fs::create_dir_all(destination)?;

    for (relative, entry) in entries {
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        match entry {
            TreeEntry::File { mode } => {
                fs::copy(source.join(&relative), &target)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&target, fs::Permissions::from_mode(mode))?;
                }
                #[cfg(not(unix))]
                let _ = mode;
            }
            TreeEntry::Symlink {
                target: link_target,
            } => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::symlink;
                    symlink(&link_target, &target)?;
                }
                #[cfg(not(unix))]
                {
                    let linked_source = source
                        .join(&relative)
                        .parent()
                        .unwrap_or(source)
                        .join(&link_target);
                    if linked_source.is_file() {
                        fs::copy(&linked_source, &target)?;
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_hash_is_deterministic_and_content_sensitive() {
        let base = std::env::temp_dir().join(format!(
            "crux_tree_hash_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = base.join("first");
        let second = base.join("second");
        fs::create_dir_all(first.join("include")).unwrap();
        fs::create_dir_all(second.join("include")).unwrap();
        fs::write(
            first.join("CMakeLists.txt"),
            b"cmake_minimum_required(VERSION 3.16)",
        )
        .unwrap();
        fs::write(first.join("include/lib.h"), b"int f();").unwrap();
        fs::write(
            second.join("CMakeLists.txt"),
            b"cmake_minimum_required(VERSION 3.16)",
        )
        .unwrap();
        fs::write(second.join("include/lib.h"), b"int f();").unwrap();

        let a = compute_tree_sha256(&first).unwrap();
        let b = compute_tree_sha256(&second).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);

        fs::write(second.join("include/lib.h"), b"int g();").unwrap();
        assert_ne!(a, compute_tree_sha256(&second).unwrap());
        let _ = fs::remove_dir_all(base);
    }
}
