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

#[cfg(test)]
mod tests {
    use super::*;

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
