//! Application source file discovery for Crux projects.
//!
//! Enforces the canonical Crux rule:
//! **Only source files under `src/` are application source inputs by default.**

use crate::core::paths::is_safe_relative_path;
use crate::project::discovery::IGNORED_DIRECTORY_NAMES;
use crate::project::manifest::ProjectLanguage;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Errors that can occur during application source discovery.
#[derive(Debug)]
pub enum DiscoveryError {
    MissingSourceDirectory(PathBuf),
    NoSourceRootsFound(PathBuf),
    InvalidSourceRoot {
        path: PathBuf,
        reason: String,
    },
    NoSourceFilesFound(PathBuf),
    LanguageMismatch {
        expected: ProjectLanguage,
        found_file: PathBuf,
        reason: String,
    },
    UnsupportedLanguage(ProjectLanguage),
    UnsupportedSourceExtension {
        file: PathBuf,
        extension: String,
    },
    SymlinkEscape {
        symlink: PathBuf,
        target: PathBuf,
    },
    Io(io::Error),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscoveryError::MissingSourceDirectory(path) => {
                write!(
                    f,
                    "source directory not found at '{}': Crux projects must contain a 'src/' directory",
                    path.display()
                )
            }
            DiscoveryError::NoSourceRootsFound(path) => {
                write!(
                    f,
                    "no compilable source roots found in '{}': add sources under src/, source/, app/, or the project root",
                    path.display()
                )
            }
            DiscoveryError::InvalidSourceRoot { path, reason } => {
                write!(f, "invalid source root '{}': {reason}", path.display())
            }
            DiscoveryError::NoSourceFilesFound(path) => {
                write!(
                    f,
                    "no compilable source files found in '{}'",
                    path.display()
                )
            }
            DiscoveryError::LanguageMismatch {
                expected,
                found_file,
                reason,
            } => {
                write!(
                    f,
                    "language mismatch in '{}': expected {} source but found incompatible file ({})",
                    found_file.display(),
                    expected.display_name(),
                    reason
                )
            }
            DiscoveryError::UnsupportedLanguage(lang) => {
                write!(
                    f,
                    "projects with language '{}' cannot be compiled with 'crux build'; only C and C++ are supported",
                    lang.display_name()
                )
            }
            DiscoveryError::UnsupportedSourceExtension { file, extension } => {
                write!(
                    f,
                    "unsupported source file extension '.{}' in '{}'",
                    extension,
                    file.display()
                )
            }
            DiscoveryError::SymlinkEscape { symlink, target } => {
                write!(
                    f,
                    "symlink escape detected at '{}': target '{}' points outside the project source tree",
                    symlink.display(),
                    target.display()
                )
            }
            DiscoveryError::Io(err) => write!(f, "I/O error during source discovery: {err}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

impl From<io::Error> for DiscoveryError {
    fn from(err: io::Error) -> Self {
        DiscoveryError::Io(err)
    }
}

/// Represents the set of discovered source files for an application project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSources {
    /// Primary source root, retained for backwards compatibility.
    pub root_src_dir: PathBuf,
    /// All canonical source roots used during discovery.
    pub source_roots: Vec<PathBuf>,
    /// Deterministically ordered compilation units.
    pub files: Vec<PathBuf>,
}

/// Recursively discovers application source files for the given project.
///
/// Projects with a `src/` directory retain the canonical Crux behavior: only
/// that directory is scanned. Imported legacy projects may instead provide
/// explicit source roots through `.crux/import.toml`, or use conventional
/// `source/`, `app/`, or root-level layouts when no `src/` directory exists.
pub fn discover_sources(
    project_root: &Path,
    language: ProjectLanguage,
) -> Result<DiscoveredSources, DiscoveryError> {
    discover_sources_with_roots(project_root, language, None)
}

/// Discovers sources using optional Crux import metadata source roots.
pub fn discover_sources_with_roots(
    project_root: &Path,
    language: ProjectLanguage,
    import_source_roots: Option<&[String]>,
) -> Result<DiscoveredSources, DiscoveryError> {
    if language == ProjectLanguage::Other {
        return Err(DiscoveryError::UnsupportedLanguage(language));
    }

    let canonical_project_root = project_root.canonicalize()?;
    let roots = resolve_source_roots(&canonical_project_root, import_source_roots)?;
    let mut unique_files = BTreeSet::new();

    for root in &roots {
        let canonical_root = root.canonicalize()?;
        if !canonical_root.starts_with(&canonical_project_root) {
            return Err(DiscoveryError::InvalidSourceRoot {
                path: root.clone(),
                reason: "source root resolves outside the project root".to_string(),
            });
        }

        let mut root_files = Vec::new();
        let mut visited_dirs = BTreeSet::from([canonical_root.clone()]);
        visit_directory(
            root,
            &canonical_root,
            language,
            &mut root_files,
            &mut visited_dirs,
        )?;
        unique_files.extend(root_files);
    }

    if unique_files.is_empty() {
        if roots.is_empty() {
            return Err(DiscoveryError::NoSourceRootsFound(canonical_project_root));
        }
        return Err(DiscoveryError::NoSourceFilesFound(
            roots.first().cloned().unwrap_or(canonical_project_root),
        ));
    }

    Ok(DiscoveredSources {
        root_src_dir: roots.first().cloned().unwrap_or(canonical_project_root),
        source_roots: roots,
        files: unique_files.into_iter().collect(),
    })
}

fn resolve_source_roots(
    project_root: &Path,
    import_source_roots: Option<&[String]>,
) -> Result<Vec<PathBuf>, DiscoveryError> {
    if let Some(roots) = import_source_roots {
        if roots.is_empty() {
            return Err(DiscoveryError::NoSourceRootsFound(
                project_root.to_path_buf(),
            ));
        }
        let mut resolved = Vec::new();
        for value in roots {
            let relative = Path::new(value);
            if !is_safe_relative_path(relative) {
                return Err(DiscoveryError::InvalidSourceRoot {
                    path: relative.to_path_buf(),
                    reason: "source root must be a safe relative path".to_string(),
                });
            }
            let candidate = project_root.join(relative);
            if !candidate.is_dir() {
                return Err(DiscoveryError::InvalidSourceRoot {
                    path: candidate,
                    reason: "source root directory does not exist".to_string(),
                });
            }
            resolved.push(candidate);
        }
        return Ok(resolved);
    }

    let src_dir = project_root.join("src");
    if src_dir.is_dir() {
        return Ok(vec![src_dir]);
    }

    let mut conventional = Vec::new();
    for name in ["source", "app"] {
        let candidate = project_root.join(name);
        if candidate.is_dir() {
            conventional.push(candidate);
        }
    }
    if !conventional.is_empty() {
        return Ok(conventional);
    }

    if has_root_compilation_units(project_root)? {
        return Ok(vec![project_root.to_path_buf()]);
    }

    Err(DiscoveryError::MissingSourceDirectory(src_dir))
}

fn has_root_compilation_units(project_root: &Path) -> Result<bool, DiscoveryError> {
    for entry in fs::read_dir(project_root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".c")
            || name.ends_with(".cc")
            || name.ends_with(".cpp")
            || name.ends_with(".cxx")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn visit_directory(
    current_dir: &Path,
    canonical_src_dir: &Path,
    language: ProjectLanguage,
    files: &mut Vec<PathBuf>,
    visited_dirs: &mut BTreeSet<PathBuf>,
) -> Result<(), DiscoveryError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(current_dir)? {
        entries.push(entry?);
    }

    // Sort directory entries deterministically by file name
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        // Skip hidden and irrelevant directories that can contain generated or
        // third-party sources unrelated to the application.
        if name_str.starts_with('.') || IGNORED_DIRECTORY_NAMES.contains(&name_str.as_ref()) {
            continue;
        }

        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_symlink() {
            let canonical_target = path.canonicalize()?;
            if !canonical_target.starts_with(canonical_src_dir) {
                return Err(DiscoveryError::SymlinkEscape {
                    symlink: path,
                    target: canonical_target,
                });
            }

            let target_metadata = fs::metadata(&canonical_target)?;
            if target_metadata.is_dir() {
                // Guard against symlink cycles that stay inside the source tree.
                if !visited_dirs.insert(canonical_target) {
                    continue;
                }
                visit_directory(&path, canonical_src_dir, language, files, visited_dirs)?;
            } else if target_metadata.is_file() {
                process_file(&path, language, files)?;
            }
        } else if file_type.is_dir() {
            // Canonicalize and track regular directories as well so that a
            // symlinked ancestor cannot cause repeated traversal.
            let canonical = path.canonicalize()?;
            if !visited_dirs.insert(canonical) {
                continue;
            }
            visit_directory(&path, canonical_src_dir, language, files, visited_dirs)?;
        } else if file_type.is_file() {
            process_file(&path, language, files)?;
        }
    }

    Ok(())
}

fn process_file(
    path: &Path,
    language: ProjectLanguage,
    files: &mut Vec<PathBuf>,
) -> Result<(), DiscoveryError> {
    let extension = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => ext.to_ascii_lowercase(),
        None => return Ok(()), // Files without extension are skipped
    };

    // Header files are not compilation units
    if is_header_extension(&extension) {
        return Ok(());
    }

    match language {
        ProjectLanguage::C => {
            if extension == "c" {
                files.push(path.to_path_buf());
            } else if is_cpp_source_extension(&extension) {
                return Err(DiscoveryError::LanguageMismatch {
                    expected: ProjectLanguage::C,
                    found_file: path.to_path_buf(),
                    reason: format!(
                        "C project cannot contain C++ source file '.{extension}'; mixing C and C++ application sources is not supported"
                    ),
                });
            } else if is_known_foreign_source_extension(&extension) {
                return Err(DiscoveryError::UnsupportedSourceExtension {
                    file: path.to_path_buf(),
                    extension,
                });
            }
        }
        ProjectLanguage::Cpp => {
            if is_cpp_source_extension(&extension) {
                files.push(path.to_path_buf());
            } else if extension == "c" {
                return Err(DiscoveryError::LanguageMismatch {
                    expected: ProjectLanguage::Cpp,
                    found_file: path.to_path_buf(),
                    reason: "C++ project cannot contain C source file '.c'; mixing C and C++ application sources is not supported".to_string(),
                });
            } else if is_known_foreign_source_extension(&extension) {
                return Err(DiscoveryError::UnsupportedSourceExtension {
                    file: path.to_path_buf(),
                    extension,
                });
            }
        }
        ProjectLanguage::Other => {
            return Err(DiscoveryError::UnsupportedLanguage(language));
        }
    }

    Ok(())
}

fn is_cpp_source_extension(ext: &str) -> bool {
    matches!(ext, "cpp" | "cc" | "cxx")
}

fn is_header_extension(ext: &str) -> bool {
    matches!(ext, "h" | "hpp" | "hxx" | "hh" | "inl" | "ipp")
}

fn is_known_foreign_source_extension(ext: &str) -> bool {
    matches!(
        ext,
        "rs" | "go" | "py" | "java" | "m" | "mm" | "swift" | "cs" | "rb" | "asm" | "s"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("crux_test_discovery_{prefix}_{nanos}"));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_discover_single_c_source() {
        let test_dir = TestDir::new("single_c");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.c"), "int main() { return 0; }").unwrap();

        let sources = discover_sources(&test_dir.path, ProjectLanguage::C).unwrap();
        assert_eq!(sources.files.len(), 1);
        assert_eq!(sources.files[0], src_dir.join("main.c"));
    }

    #[test]
    fn test_discover_multiple_cpp_sources_nested_and_sorted() {
        let test_dir = TestDir::new("multi_cpp");
        let src_dir = test_dir.path.join("src");
        let sub_dir = src_dir.join("modules").join("math");
        fs::create_dir_all(&sub_dir).unwrap();

        fs::write(src_dir.join("main.cpp"), "// main").unwrap();
        fs::write(src_dir.join("zebra.cc"), "// zebra").unwrap();
        fs::write(sub_dir.join("calc.cxx"), "// calc").unwrap();
        fs::write(sub_dir.join("calc.hpp"), "// header should be ignored").unwrap();

        let sources = discover_sources(&test_dir.path, ProjectLanguage::Cpp).unwrap();
        assert_eq!(sources.files.len(), 3);
        assert_eq!(sources.files[0], src_dir.join("main.cpp"));
        assert_eq!(sources.files[1], sub_dir.join("calc.cxx"));
        assert_eq!(sources.files[2], src_dir.join("zebra.cc"));
    }

    #[test]
    fn test_discover_ignores_hidden_files_and_directories() {
        let test_dir = TestDir::new("hidden");
        let src_dir = test_dir.path.join("src");
        let hidden_dir = src_dir.join(".hidden");
        fs::create_dir_all(&hidden_dir).unwrap();

        fs::write(src_dir.join("main.cpp"), "// main").unwrap();
        fs::write(src_dir.join(".secret.cpp"), "// hidden file").unwrap();
        fs::write(hidden_dir.join("ignored.cpp"), "// hidden dir file").unwrap();

        let sources = discover_sources(&test_dir.path, ProjectLanguage::Cpp).unwrap();
        assert_eq!(sources.files.len(), 1);
        assert_eq!(sources.files[0], src_dir.join("main.cpp"));
    }

    #[test]
    fn test_discover_missing_src_dir_fails() {
        let test_dir = TestDir::new("missing_src");
        let result = discover_sources(&test_dir.path, ProjectLanguage::C);
        assert!(matches!(
            result,
            Err(DiscoveryError::MissingSourceDirectory(_))
        ));
    }

    #[test]
    fn test_discover_empty_src_dir_fails() {
        let test_dir = TestDir::new("empty_src");
        fs::create_dir_all(test_dir.path.join("src")).unwrap();

        let result = discover_sources(&test_dir.path, ProjectLanguage::C);
        assert!(matches!(result, Err(DiscoveryError::NoSourceFilesFound(_))));
    }

    #[test]
    fn test_discover_c_project_with_cpp_source_fails() {
        let test_dir = TestDir::new("mismatch_c");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        fs::write(src_dir.join("main.c"), "// c").unwrap();
        fs::write(src_dir.join("helper.cpp"), "// cpp").unwrap();

        let result = discover_sources(&test_dir.path, ProjectLanguage::C);
        assert!(matches!(
            result,
            Err(DiscoveryError::LanguageMismatch {
                expected: ProjectLanguage::C,
                ..
            })
        ));
    }

    #[test]
    fn test_discover_cpp_project_with_c_source_fails() {
        let test_dir = TestDir::new("mismatch_cpp");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        fs::write(src_dir.join("main.cpp"), "// cpp").unwrap();
        fs::write(src_dir.join("legacy.c"), "// c").unwrap();

        let result = discover_sources(&test_dir.path, ProjectLanguage::Cpp);
        assert!(matches!(
            result,
            Err(DiscoveryError::LanguageMismatch {
                expected: ProjectLanguage::Cpp,
                ..
            })
        ));
    }

    #[test]
    fn test_discover_unsupported_language_other_fails() {
        let test_dir = TestDir::new("other_lang");
        let result = discover_sources(&test_dir.path, ProjectLanguage::Other);
        assert!(matches!(
            result,
            Err(DiscoveryError::UnsupportedLanguage(ProjectLanguage::Other))
        ));
    }

    #[test]
    fn test_discover_unsupported_extension_fails() {
        let test_dir = TestDir::new("unsupported_ext");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        fs::write(src_dir.join("main.cpp"), "// cpp").unwrap();
        fs::write(src_dir.join("plugin.rs"), "// rust").unwrap();

        let result = discover_sources(&test_dir.path, ProjectLanguage::Cpp);
        assert!(matches!(
            result,
            Err(DiscoveryError::UnsupportedSourceExtension { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_discover_symlink_escape_fails() {
        use std::os::unix::fs::symlink;

        let test_dir = TestDir::new("symlink_escape");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        let outside_dir = test_dir.path.join("outside");
        fs::create_dir_all(&outside_dir).unwrap();
        let outside_file = outside_dir.join("outside.c");
        fs::write(&outside_file, "int x = 1;").unwrap();

        symlink(&outside_file, src_dir.join("escaped.c")).unwrap();

        let result = discover_sources(&test_dir.path, ProjectLanguage::C);
        assert!(matches!(result, Err(DiscoveryError::SymlinkEscape { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn test_discover_symlinked_directory_escape_fails() {
        use std::os::unix::fs::symlink;

        let test_dir = TestDir::new("symlink_dir_escape");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.c"), "int main(void) { return 0; }").unwrap();

        let outside_dir = test_dir.path.join("outside");
        fs::create_dir_all(&outside_dir).unwrap();
        fs::write(outside_dir.join("outside.c"), "int x = 1;").unwrap();

        symlink(&outside_dir, src_dir.join("linked")).unwrap();

        let result = discover_sources(&test_dir.path, ProjectLanguage::C);
        assert!(matches!(result, Err(DiscoveryError::SymlinkEscape { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn test_discover_symlink_cycle_terminates() {
        use std::os::unix::fs::symlink;

        let test_dir = TestDir::new("symlink_cycle");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(src_dir.join("nested")).unwrap();
        fs::write(src_dir.join("main.c"), "int main(void) { return 0; }").unwrap();
        fs::write(src_dir.join("nested").join("helper.c"), "int x = 1;").unwrap();

        // Point back to the source root, creating a cycle.
        symlink(&src_dir, src_dir.join("nested").join("loop")).unwrap();

        let sources = discover_sources(&test_dir.path, ProjectLanguage::C).unwrap();
        assert_eq!(sources.files.len(), 2);
    }

    #[test]
    fn test_discover_paths_with_spaces() {
        let test_dir = TestDir::new("spaces");
        let src_dir = test_dir.path.join("src").join("my module");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            test_dir.path.join("src").join("main file.c"),
            "int main(void) { return 0; }",
        )
        .unwrap();
        fs::write(src_dir.join("helper file.c"), "int y = 2;").unwrap();

        let sources = discover_sources(&test_dir.path, ProjectLanguage::C).unwrap();
        assert_eq!(sources.files.len(), 2);
        assert!(
            sources
                .files
                .iter()
                .all(|p| p.to_string_lossy().contains(' '))
        );
    }

    #[test]
    fn test_discover_uppercase_extension() {
        let test_dir = TestDir::new("uppercase");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.CPP"), "// cpp").unwrap();

        let sources = discover_sources(&test_dir.path, ProjectLanguage::Cpp).unwrap();
        assert_eq!(sources.files.len(), 1);
    }

    #[test]
    fn test_discover_ignores_headers_and_extensionless_files() {
        let test_dir = TestDir::new("headers");
        let src_dir = test_dir.path.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.c"), "int main(void) { return 0; }").unwrap();
        fs::write(src_dir.join("main.h"), "int main(void);").unwrap();
        fs::write(src_dir.join("LICENSE"), "not a source file").unwrap();

        let sources = discover_sources(&test_dir.path, ProjectLanguage::C).unwrap();
        assert_eq!(sources.files.len(), 1);
        assert_eq!(sources.files[0], src_dir.join("main.c"));
    }
}
