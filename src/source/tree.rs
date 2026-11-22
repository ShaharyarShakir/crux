//! Deterministic source-tree validation, hashing, and copying.
//!
//! # Tree digest format (`crux-source-tree-v2`)
//!
//! The digest recorded for Git and local-path sources covers, in sorted
//! relative-path order, every regular file and symlink of the tree:
//!
//! * paths are hashed as raw OS bytes (Unix) so distinct non-UTF-8 names can
//!   never collide;
//! * regular files contribute only their *executable* bit plus their contents,
//!   so the digest is independent of the user's umask (`0644` vs `0664`);
//! * symlinks contribute their raw target bytes;
//! * `.git` metadata is skipped at every depth, while local build outputs
//!   (`build/`, `target/`, `cmake-build-*`, `.crux/`) are skipped **only at the
//!   tree root**, so nested source directories with those names are kept.
//!
//! The previous `crux-source-tree-v1` digest is still computable through
//! [`compute_legacy_tree_sha256_v1`] so lockfiles written by Crux 0.2.0 and
//! 0.2.1 can be verified without being rewritten.

use super::AcquisitionError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const TREE_HASH_DOMAIN: &[u8] = b"crux-source-tree-v2\0";
const LEGACY_V1_TREE_HASH_DOMAIN: &[u8] = b"crux-source-tree-v1\0";

#[derive(Debug, Clone)]
enum TreeEntry {
    /// A regular file with its full permission bits (preserved on copy).
    File {
        mode: u32,
    },
    Symlink {
        target: PathBuf,
    },
}

/// Which entries a traversal skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IgnoreRules {
    /// Current rules: build outputs are only skipped at the tree root.
    RootOnlyBuildOutputs,
    /// Crux 0.2.0/0.2.1 rules: names skipped at every depth (legacy digests only).
    LegacyRecursive,
}

fn is_ignored_name(name: &std::ffi::OsStr, at_root: bool, rules: IgnoreRules) -> bool {
    if name == ".git" {
        return true;
    }
    let is_build_output = |name: &std::ffi::OsStr| {
        let text = name.to_string_lossy();
        text == ".crux" || text == "target" || text == "build" || text.starts_with("cmake-build-")
    };
    match rules {
        IgnoreRules::RootOnlyBuildOutputs => {
            at_root && (is_build_output(name) || name == super::COMPLETION_SENTINEL)
        }
        IgnoreRules::LegacyRecursive => is_build_output(name) || name == super::COMPLETION_SENTINEL,
    }
}

fn collect_entries(
    root: &Path,
    canonical_root: &Path,
    dir: &Path,
    rules: IgnoreRules,
    out: &mut BTreeMap<PathBuf, TreeEntry>,
) -> Result<(), AcquisitionError> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let at_root = dir == root;

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if is_ignored_name(&name, at_root, rules) {
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
            validate_symlink(canonical_root, relative, &target)?;
            out.insert(relative.to_path_buf(), TreeEntry::Symlink { target });
            continue;
        }

        if metadata.is_dir() {
            collect_entries(root, canonical_root, &path, rules, out)?;
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

fn collect_tree(
    root: &Path,
    rules: IgnoreRules,
) -> Result<BTreeMap<PathBuf, TreeEntry>, AcquisitionError> {
    let canonical_root = fs::canonicalize(root)?;
    let mut entries = BTreeMap::new();
    collect_entries(root, &canonical_root, root, rules, &mut entries)?;
    Ok(entries)
}

/// Rejects symlinks whose target is absolute or that resolve outside the tree.
///
/// Resolution is *physical*: existing symlinks along the target are followed,
/// so a chain such as `x/s -> ..` plus `x/s2 -> s/..` (lexically inside,
/// physically the parent of the root) is rejected.
fn validate_symlink(
    canonical_root: &Path,
    relative: &Path,
    target: &Path,
) -> Result<(), AcquisitionError> {
    if target.is_absolute() || target.has_root() {
        return Err(AcquisitionError::SecurityViolation(format!(
            "symlink '{}' has an absolute target '{}'",
            relative.display(),
            target.display()
        )));
    }

    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let resolved = crate::core::paths::resolve_physical_path(&canonical_root.join(parent), target)
        .map_err(|err| {
            AcquisitionError::SecurityViolation(format!(
                "symlink '{}' target '{}' cannot be resolved safely: {err}",
                relative.display(),
                target.display()
            ))
        })?;
    if !resolved.starts_with(canonical_root) {
        return Err(AcquisitionError::SecurityViolation(format!(
            "symlink '{}' has a target that escapes the source root: '{}'",
            relative.display(),
            target.display()
        )));
    }
    Ok(())
}

/// Hashes a path as length-prefixed raw bytes.
fn hash_path_bytes(hasher: &mut Sha256, path: &Path) {
    #[cfg(unix)]
    let bytes: std::borrow::Cow<'_, [u8]> = {
        use std::os::unix::ffi::OsStrExt;
        std::borrow::Cow::Borrowed(path.as_os_str().as_bytes())
    };
    #[cfg(not(unix))]
    let bytes: std::borrow::Cow<'_, [u8]> = {
        use std::os::windows::ffi::OsStrExt;
        std::borrow::Cow::Owned(
            path.as_os_str()
                .encode_wide()
                .flat_map(u16::to_le_bytes)
                .collect(),
        )
    };
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(&bytes);
}

/// Hashes a path as length-prefixed lossy UTF-8 (legacy v1 only).
fn hash_path_lossy(hasher: &mut Sha256, path: &Path) {
    let text = path.to_string_lossy();
    hasher.update((text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
}

/// Computes a deterministic SHA-256 digest over a validated source tree.
///
/// See the module documentation for the exact `crux-source-tree-v2` format.
pub fn compute_tree_sha256(root: &Path) -> Result<String, AcquisitionError> {
    let entries = collect_tree(root, IgnoreRules::RootOnlyBuildOutputs)?;

    let mut hasher = Sha256::new();
    hasher.update(TREE_HASH_DOMAIN);
    hasher.update((entries.len() as u64).to_le_bytes());

    for (relative, entry) in entries {
        hash_path_bytes(&mut hasher, &relative);
        match entry {
            TreeEntry::File { mode } => {
                hasher.update(b"F");
                hasher.update([u8::from(mode & 0o111 != 0)]);
                let bytes = fs::read(root.join(&relative))?;
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            TreeEntry::Symlink { target } => {
                hasher.update(b"L");
                hash_path_bytes(&mut hasher, &target);
            }
        }
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Computes the legacy `crux-source-tree-v1` digest written by Crux 0.2.0 and 0.2.1.
///
/// The v1 format hashed full permission bits (umask-dependent), lossy UTF-8
/// paths, and skipped build-output names at every depth. It is used only to
/// accept lockfiles produced by that release; new digests always use
/// [`compute_tree_sha256`].
pub fn compute_legacy_tree_sha256_v1(root: &Path) -> Result<String, AcquisitionError> {
    let entries = collect_tree(root, IgnoreRules::LegacyRecursive)?;

    let mut hasher = Sha256::new();
    hasher.update(LEGACY_V1_TREE_HASH_DOMAIN);
    hasher.update((entries.len() as u64).to_le_bytes());

    for (relative, entry) in entries {
        hash_path_lossy(&mut hasher, &relative);
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
                hash_path_lossy(&mut hasher, &target);
            }
        }
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Validates and copies a source tree into a destination directory.
///
/// Regular files keep their permission bits so the copy is a faithful
/// snapshot (and legacy v1 digests of the copy match the original).
pub fn copy_validated_tree(source: &Path, destination: &Path) -> Result<(), AcquisitionError> {
    let entries = collect_tree(source, IgnoreRules::RootOnlyBuildOutputs)?;
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

    static TEMP_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn temp_tree(label: &str) -> PathBuf {
        let sequence = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("crux_tree_test_{label}_{sequence}_{nanos}"))
    }

    #[cfg(unix)]
    fn create_source_tree(root: &Path) {
        use std::os::unix::fs::symlink;

        fs::create_dir_all(root.join("include")).unwrap();
        fs::write(
            root.join("CMakeLists.txt"),
            b"cmake_minimum_required(VERSION 3.16)",
        )
        .unwrap();
        fs::write(root.join("include/lib.h"), b"int f();").unwrap();
        symlink("../CMakeLists.txt", root.join("include/valid-link")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_root_level_escaping_symlink() {
        use std::os::unix::fs::symlink;

        let root = temp_tree("root_escape");
        create_source_tree(&root);
        symlink("../../../../../../../etc/hostname", root.join("escape")).unwrap();

        assert!(matches!(
            compute_tree_sha256(&root),
            Err(AcquisitionError::SecurityViolation(_))
        ));
        assert!(matches!(
            copy_validated_tree(&root, &temp_tree("root_escape_dest")),
            Err(AcquisitionError::SecurityViolation(_))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_nested_escaping_symlink() {
        use std::os::unix::fs::symlink;

        let root = temp_tree("nested_escape");
        create_source_tree(&root);
        symlink("../../../etc/hostname", root.join("include/escape")).unwrap();

        assert!(matches!(
            compute_tree_sha256(&root),
            Err(AcquisitionError::SecurityViolation(_))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn allows_relative_symlink_inside_source_root() {
        let root = temp_tree("valid_link");
        create_source_tree(&root);

        let digest = compute_tree_sha256(&root).expect("in-root symlink should be accepted");
        assert_eq!(digest.len(), 64);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_physical_symlink_chain_escape() {
        use std::os::unix::fs::symlink;

        let root = temp_tree("chain_escape");
        create_source_tree(&root);
        fs::create_dir_all(root.join("x")).unwrap();
        symlink("..", root.join("x/s")).unwrap();
        // Lexically `x/s2` -> `x`; physically the parent of the root.
        symlink("s/..", root.join("x/s2")).unwrap();

        assert!(matches!(
            compute_tree_sha256(&root),
            Err(AcquisitionError::SecurityViolation(_))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_build_and_target_directories_are_kept() {
        let root = temp_tree("nested_build");
        fs::create_dir_all(root.join("libs/core/build")).unwrap();
        fs::create_dir_all(root.join("tools/target")).unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::create_dir_all(root.join("cmake-build-debug")).unwrap();
        fs::create_dir_all(root.join("sub/.git")).unwrap();
        fs::write(root.join("libs/core/build/Jamfile"), b"lib core ;").unwrap();
        fs::write(root.join("tools/target/spec.txt"), b"spec").unwrap();
        fs::write(root.join("build/CMakeCache.txt"), b"local output").unwrap();
        fs::write(root.join("cmake-build-debug/out.o"), b"obj").unwrap();
        fs::write(root.join("sub/.git/HEAD"), b"ref").unwrap();

        let dest = temp_tree("nested_build_dest");
        copy_validated_tree(&root, &dest).unwrap();
        assert!(dest.join("libs/core/build/Jamfile").is_file());
        assert!(dest.join("tools/target/spec.txt").is_file());
        assert!(!dest.join("build").exists(), "root build output is skipped");
        assert!(!dest.join("cmake-build-debug").exists());
        assert!(
            !dest.join("sub/.git").exists(),
            ".git is skipped at any depth"
        );

        // Nested build directories contribute to the digest.
        let before = compute_tree_sha256(&root).unwrap();
        fs::write(
            root.join("libs/core/build/Jamfile"),
            b"lib core : changed ;",
        )
        .unwrap();
        assert_ne!(before, compute_tree_sha256(&root).unwrap());
        // Root build output does not.
        let before = compute_tree_sha256(&root).unwrap();
        fs::write(root.join("build/CMakeCache.txt"), b"other output").unwrap();
        assert_eq!(before, compute_tree_sha256(&root).unwrap());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(dest);
    }

    #[cfg(unix)]
    #[test]
    fn digest_depends_only_on_executable_bit() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_tree("modes");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("configure");
        fs::write(&file, b"#!/bin/sh\n").unwrap();

        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let umask_022 = compute_tree_sha256(&root).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o664)).unwrap();
        let umask_002 = compute_tree_sha256(&root).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let private = compute_tree_sha256(&root).unwrap();
        assert_eq!(umask_022, umask_002);
        assert_eq!(umask_022, private);

        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        let exec_755 = compute_tree_sha256(&root).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o775)).unwrap();
        let exec_775 = compute_tree_sha256(&root).unwrap();
        assert_ne!(umask_022, exec_755, "executable bit is significant");
        assert_eq!(exec_755, exec_775);

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_names_do_not_collide() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let first = temp_tree("non_utf8_a");
        let second = temp_tree("non_utf8_b");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        // Both names are lossily rendered as "file-\u{FFFD}.h".
        let name_a = OsStr::from_bytes(b"file-\xff.h");
        let name_b = OsStr::from_bytes(b"file-\xfe.h");
        if fs::write(first.join(name_a), b"same").is_err() {
            return; // Filesystem rejects non-UTF-8 names; nothing to verify.
        }
        fs::write(second.join(name_b), b"same").unwrap();

        assert_eq!(
            compute_legacy_tree_sha256_v1(&first).unwrap(),
            compute_legacy_tree_sha256_v1(&second).unwrap(),
            "legacy v1 digests collided on lossy names"
        );
        assert_ne!(
            compute_tree_sha256(&first).unwrap(),
            compute_tree_sha256(&second).unwrap()
        );

        let _ = fs::remove_dir_all(first);
        let _ = fs::remove_dir_all(second);
    }

    #[test]
    fn legacy_v1_digest_matches_released_v1_algorithm() {
        let root = temp_tree("legacy_v1");
        fs::create_dir_all(root.join("include")).unwrap();
        fs::write(root.join("CMakeLists.txt"), b"project(x)").unwrap();
        fs::write(root.join("include/x.h"), b"int x;").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for file in ["CMakeLists.txt", "include/x.h"] {
                fs::set_permissions(root.join(file), fs::Permissions::from_mode(0o644)).unwrap();
            }
        }

        // Independent re-implementation of the v0.2.0/v0.2.1 hashing loop.
        let mut hasher = Sha256::new();
        hasher.update(b"crux-source-tree-v1\0");
        hasher.update(2u64.to_le_bytes());
        for (path, bytes) in [
            ("CMakeLists.txt", &b"project(x)"[..]),
            ("include/x.h", &b"int x;"[..]),
        ] {
            hasher.update((path.len() as u64).to_le_bytes());
            hasher.update(path.as_bytes());
            hasher.update(b"F");
            hasher.update(0o644u32.to_le_bytes());
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        let expected = format!("{:x}", hasher.finalize());
        assert_eq!(compute_legacy_tree_sha256_v1(&root).unwrap(), expected);
        assert_ne!(compute_tree_sha256(&root).unwrap(), expected);

        let _ = fs::remove_dir_all(root);
    }
}
