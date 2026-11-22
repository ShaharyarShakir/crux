//! Path manipulation and validation utilities.

use std::path::{Component, Path, PathBuf};

/// Normalizes a path by resolving `.` and `..` components lexically without accessing the filesystem.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(p) => normalized.push(Component::Prefix(p)),
            Component::RootDir => normalized.push(Component::RootDir),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(c) => normalized.push(c),
        }
    }
    normalized
}

/// Checks whether a relative path contains any traversal or root components (`..`, `/`, `C:`).
pub fn is_safe_relative_path(path: &Path) -> bool {
    !path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

/// Maximum number of symbolic links followed while resolving one path, matching
/// the conventional `ELOOP` limit used by Linux path resolution.
pub const MAX_SYMLINK_HOPS: usize = 40;

/// Resolves `relative` starting from the existing directory `start`, following
/// symbolic links *physically* the way the operating system would.
///
/// Unlike lexical normalization, a `..` component that follows a symlinked
/// directory moves to the parent of the symlink's *target*. Components that do
/// not exist yet are appended lexically, so the result describes where a write
/// through the path would land. Absolute symlink targets restart resolution at
/// the filesystem root. Resolution fails with `ErrorKind::InvalidInput` once more
/// than [`MAX_SYMLINK_HOPS`] links have been followed.
pub fn resolve_physical_path(start: &Path, relative: &Path) -> std::io::Result<PathBuf> {
    use std::collections::VecDeque;
    use std::ffi::OsString;

    enum Step {
        Root(PathBuf),
        Parent,
        Name(OsString),
    }

    fn push_components(queue: &mut VecDeque<Step>, path: &Path, front: bool) {
        let mut steps = Vec::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => {
                    let mut root = PathBuf::new();
                    root.push(component.as_os_str());
                    if let Some(Step::Root(existing)) = steps.last_mut() {
                        existing.push(component.as_os_str());
                    } else {
                        steps.push(Step::Root(root));
                    }
                }
                Component::CurDir => {}
                Component::ParentDir => steps.push(Step::Parent),
                Component::Normal(name) => steps.push(Step::Name(name.to_os_string())),
            }
        }
        if front {
            for step in steps.into_iter().rev() {
                queue.push_front(step);
            }
        } else {
            queue.extend(steps);
        }
    }

    let mut current = std::fs::canonicalize(start)?;
    let mut queue = VecDeque::new();
    push_components(&mut queue, relative, false);
    let mut hops = 0usize;

    while let Some(step) = queue.pop_front() {
        match step {
            Step::Root(root) => current = root,
            Step::Parent => {
                current.pop();
            }
            Step::Name(name) => {
                let candidate = current.join(&name);
                match std::fs::symlink_metadata(&candidate) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        hops += 1;
                        if hops > MAX_SYMLINK_HOPS {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                format!(
                                    "too many levels of symbolic links while resolving '{}'",
                                    candidate.display()
                                ),
                            ));
                        }
                        let target = std::fs::read_link(&candidate)?;
                        // The link target is interpreted relative to the
                        // directory containing the link (`current`).
                        push_components(&mut queue, &target, true);
                    }
                    Ok(_) => current = candidate,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        current = candidate;
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }

    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_resolve_physical_path_follows_symlink_chains() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!(
            "crux_paths_resolve_{}_{}",
            std::process::id(),
            crate::core::filesystem::generate_nonce()
        ));
        let root = base.join("a/b/root");
        std::fs::create_dir_all(root.join("x")).unwrap();
        let root = std::fs::canonicalize(&root).unwrap();

        // Lexically `x/s2` -> `x`, but physically `s` is `root`, so `s/..` is
        // the parent of root.
        symlink("..", root.join("x/s")).unwrap();
        symlink("s/..", root.join("x/s2")).unwrap();

        let resolved = resolve_physical_path(&root.join("x"), Path::new("s2/new.txt")).unwrap();
        assert_eq!(resolved, root.parent().unwrap().join("new.txt"));
        assert!(!resolved.starts_with(&root));

        let inside = resolve_physical_path(&root.join("x"), Path::new("s/x")).unwrap();
        assert_eq!(inside, root.join("x"));

        symlink("loop_b", root.join("loop_a")).unwrap();
        symlink("loop_a", root.join("loop_b")).unwrap();
        assert!(resolve_physical_path(&root, Path::new("loop_a")).is_err());

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path(Path::new("a/b/../c")), PathBuf::from("a/c"));
        assert_eq!(
            normalize_path(Path::new("/a/./b/../../c")),
            PathBuf::from("/c")
        );
    }

    #[test]
    fn test_is_safe_relative_path() {
        assert!(is_safe_relative_path(Path::new("include/fmt/core.h")));
        assert!(!is_safe_relative_path(Path::new("../secret.txt")));
        assert!(!is_safe_relative_path(Path::new("/etc/passwd")));
    }
}
