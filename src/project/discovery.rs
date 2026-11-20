//! Safe, deterministic discovery for importing existing C/C++ projects.
//!
//! Import discovery is read-only. It never executes build scripts and never
//! follows symlinks that resolve outside the selected project root.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Directories that are never inspected during project discovery.
pub const IGNORED_DIRECTORY_NAMES: &[&str] = &[
    ".git",
    ".crux",
    "build",
    "out",
    "dist",
    "node_modules",
    "target",
    "vendor",
];

/// Build system detected at the project root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DetectedBuildSystem {
    Cmake,
    Make,
    None,
}

impl DetectedBuildSystem {
    /// Returns a stable user-facing label.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Cmake => "CMake",
            Self::Make => "Make",
            Self::None => "None detected",
        }
    }
}

impl fmt::Display for DetectedBuildSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Errors that can occur while discovering an existing project.
#[derive(Debug)]
pub enum ProjectDiscoveryError {
    InvalidProjectDirectory { path: PathBuf, reason: String },
    SymlinkOutsideProject { symlink: PathBuf },
    Io(io::Error),
}

impl fmt::Display for ProjectDiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProjectDirectory { path, reason } => {
                write!(
                    f,
                    "invalid project directory '{}': {reason}",
                    path.display()
                )
            }
            Self::SymlinkOutsideProject { symlink } => {
                write!(
                    f,
                    "symlink '{}' resolves outside the project root and will not be followed",
                    symlink.display()
                )
            }
            Self::Io(err) => write!(f, "I/O error during project discovery: {err}"),
        }
    }
}

impl std::error::Error for ProjectDiscoveryError {}

impl From<io::Error> for ProjectDiscoveryError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Facts discovered from an existing project tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDiscovery {
    /// Canonical project root.
    pub root: PathBuf,
    /// Name declared by the top-level CMake `project()` command, when present.
    pub cmake_project_name: Option<String>,
    /// Build system detected at the project root.
    pub build_system: DetectedBuildSystem,
    /// Directories containing compilable source files, as relative paths.
    pub source_dirs: Vec<String>,
    /// Conventional include directories, as relative paths.
    pub include_dirs: Vec<String>,
    /// Compilable C/C++ source files, as relative paths.
    pub source_files: Vec<String>,
    /// Header files, as relative paths.
    pub header_files: Vec<String>,
    /// Files that look like application entry points, as relative paths.
    pub entry_points: Vec<String>,
    /// Likely executable target names.
    pub targets: Vec<String>,
    /// Test directories discovered in the project tree.
    pub test_dirs: Vec<String>,
    /// Non-fatal observations that affect import confidence.
    pub warnings: Vec<String>,
}

/// Inspects `root` without modifying it or executing any project commands.
pub fn discover_project(root: &Path) -> Result<ProjectDiscovery, ProjectDiscoveryError> {
    let canonical_root = root.canonicalize().map_err(|err| {
        let reason = match err.kind() {
            io::ErrorKind::NotFound => "directory does not exist".to_string(),
            _ => format!("cannot resolve directory: {err}"),
        };
        ProjectDiscoveryError::InvalidProjectDirectory {
            path: root.to_path_buf(),
            reason,
        }
    })?;

    if !canonical_root.is_dir() {
        return Err(ProjectDiscoveryError::InvalidProjectDirectory {
            path: root.to_path_buf(),
            reason: "path is not a directory".to_string(),
        });
    }

    let has_cmake = canonical_root.join("CMakeLists.txt").is_file();
    let has_make = ["Makefile", "makefile", "GNUmakefile"]
        .iter()
        .any(|name| canonical_root.join(name).is_file());
    let build_system = if has_cmake {
        DetectedBuildSystem::Cmake
    } else if has_make {
        DetectedBuildSystem::Make
    } else {
        DetectedBuildSystem::None
    };

    let cmake_project_name = if has_cmake {
        parse_cmake_project_name(&canonical_root.join("CMakeLists.txt"))
    } else {
        None
    };

    let mut source_files = BTreeSet::new();
    let mut header_files = BTreeSet::new();
    let mut source_dirs = BTreeSet::new();
    let mut include_dirs = BTreeSet::new();
    let mut entry_points = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut test_dirs = BTreeSet::new();
    let mut warnings = BTreeSet::new();
    let mut visited = BTreeSet::new();

    visit_directory(
        &canonical_root,
        &canonical_root,
        &mut visited,
        &mut source_files,
        &mut header_files,
        &mut source_dirs,
        &mut include_dirs,
        &mut entry_points,
        &mut targets,
        &mut test_dirs,
        &mut warnings,
    )?;

    if source_files.is_empty() {
        warnings.insert("no C or C++ source files were found".to_string());
    }

    if source_files.iter().any(|p| is_cpp_source(p)) && source_files.iter().any(|p| is_c_source(p))
    {
        warnings.insert(
            "both C and C++ compilation units were found; import selects C++, but mixed-language builds require manual review"
                .to_string(),
        );
    }

    if entry_points.is_empty() && !source_files.is_empty() {
        warnings.insert(
            "no conventional main entry point was found; verify the executable target manually"
                .to_string(),
        );
    }

    if targets.len() > 1 {
        warnings.insert(
            "multiple executable entry points were detected; the generated manifest targets a single application"
                .to_string(),
        );
    }

    if build_system == DetectedBuildSystem::Make {
        warnings.insert(
            "Make builds are not executed by Crux; generated sources are built by Crux's internal CMake engine"
                .to_string(),
        );
    }

    if build_system == DetectedBuildSystem::Cmake {
        warnings.insert(
            "existing CMakeLists.txt is preserved but not consumed; Crux builds detected sources with its internal CMake project"
                .to_string(),
        );
    }

    Ok(ProjectDiscovery {
        root: canonical_root,
        cmake_project_name,
        build_system,
        source_dirs: source_dirs.into_iter().collect(),
        include_dirs: include_dirs.into_iter().collect(),
        source_files: source_files.into_iter().collect(),
        header_files: header_files.into_iter().collect(),
        entry_points: entry_points.into_iter().collect(),
        targets: targets.into_iter().collect(),
        test_dirs: test_dirs.into_iter().collect(),
        warnings: warnings.into_iter().collect(),
    })
}

#[allow(clippy::too_many_arguments)]
fn visit_directory(
    current: &Path,
    project_root: &Path,
    visited: &mut BTreeSet<PathBuf>,
    source_files: &mut BTreeSet<String>,
    header_files: &mut BTreeSet<String>,
    source_dirs: &mut BTreeSet<String>,
    include_dirs: &mut BTreeSet<String>,
    entry_points: &mut BTreeSet<String>,
    targets: &mut BTreeSet<String>,
    test_dirs: &mut BTreeSet<String>,
    warnings: &mut BTreeSet<String>,
) -> Result<(), ProjectDiscoveryError> {
    let canonical = current.canonicalize()?;
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(current)? {
        entries.push(entry?);
    }
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with('.') || IGNORED_DIRECTORY_NAMES.contains(&name_str.as_ref()) {
            continue;
        }

        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_symlink() {
            let target = match path.canonicalize() {
                Ok(target) => target,
                Err(_) => {
                    warnings.insert(format!(
                        "skipped broken symlink '{}'",
                        relative_display(project_root, &path)
                    ));
                    continue;
                }
            };
            if !target.starts_with(project_root) {
                warnings.insert(format!(
                    "skipped symlink '{}' pointing outside the project root",
                    relative_display(project_root, &path)
                ));
                continue;
            }
        }

        if file_type.is_dir() || (file_type.is_symlink() && path.is_dir()) {
            if name_str == "include" || name_str == "inc" || name_str == "headers" {
                include_dirs.insert(relative_display(project_root, &path));
            }
            if name_str == "test" || name_str == "tests" {
                test_dirs.insert(relative_display(project_root, &path));
            }
            visit_directory(
                &path,
                project_root,
                visited,
                source_files,
                header_files,
                source_dirs,
                include_dirs,
                entry_points,
                targets,
                test_dirs,
                warnings,
            )?;
            continue;
        }

        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }

        let relative = relative_display(project_root, &path);
        let lower = name_str.to_ascii_lowercase();

        if is_source_file(&lower) {
            source_files.insert(relative.clone());
            source_dirs.insert(parent_display(&relative));
            if is_entry_point(&lower) || contains_main_symbol(&path) {
                entry_points.insert(relative.clone());
                targets.insert(target_name_from_relative(&relative));
            }
        } else if is_header_file(&lower) {
            header_files.insert(relative);
        }
    }

    Ok(())
}

fn relative_display(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".to_string(),
        Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
        Err(_) => path.to_string_lossy().replace('\\', "/"),
    }
}

fn parent_display(relative: &str) -> String {
    match Path::new(relative).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.to_string_lossy().replace('\\', "/")
        }
        _ => ".".to_string(),
    }
}

fn is_source_file(name: &str) -> bool {
    [".c", ".cc", ".cpp", ".cxx"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

fn is_header_file(name: &str) -> bool {
    [".h", ".hh", ".hpp", ".hxx", ".inl", ".ipp"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

fn is_c_source(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".c")
}

fn is_cpp_source(name: &str) -> bool {
    [".cc", ".cpp", ".cxx"]
        .iter()
        .any(|ext| name.to_ascii_lowercase().ends_with(ext))
}

fn is_entry_point(name: &str) -> bool {
    name.starts_with("main.")
}

fn target_name_from_relative(relative: &str) -> String {
    Path::new(relative)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
}

fn contains_main_symbol(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("int main")
            || trimmed.starts_with("auto main")
            || trimmed.starts_with("signed main")
            || trimmed.starts_with("unsigned main")
    })
}

fn parse_cmake_project_name(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if !lower.starts_with("project") {
            continue;
        }
        let after_keyword = trimmed["project".len()..].trim_start();
        if !after_keyword.starts_with('(') {
            continue;
        }
        let inside = &after_keyword[1..];
        let end = inside.find(')').unwrap_or(inside.len());
        let candidate = inside[..end]
            .split(|ch: char| ch.is_whitespace() || ch == '#')
            .next()
            .unwrap_or("")
            .trim_matches(|ch| ch == '"' || ch == '\'');
        if !candidate.is_empty() {
            return Some(candidate.to_string());
        }
    }
    None
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
            let path = std::env::temp_dir().join(format!("crux_discovery_test_{prefix}_{nanos}"));
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
    fn test_discover_project_detects_sources_and_ignored_dirs() {
        let dir = TestDir::new("basic");
        fs::write(dir.path.join("main.c"), "int main(void){return 0;}\n").unwrap();
        fs::create_dir_all(dir.path.join("include")).unwrap();
        fs::write(dir.path.join("include/util.h"), "#pragma once\n").unwrap();
        fs::create_dir_all(dir.path.join("build")).unwrap();
        fs::write(dir.path.join("build/ignored.cpp"), "ignored\n").unwrap();

        let discovery = discover_project(&dir.path).unwrap();
        assert_eq!(discovery.source_files, vec!["main.c".to_string()]);
        assert_eq!(discovery.include_dirs, vec!["include".to_string()]);
        assert!(
            !discovery
                .source_files
                .iter()
                .any(|p| p.starts_with("build/"))
        );
    }

    #[test]
    fn test_parse_cmake_project_name() {
        let dir = TestDir::new("cmake");
        let cmake = dir.path.join("CMakeLists.txt");
        fs::write(
            &cmake,
            "cmake_minimum_required(VERSION 3.20)\nproject(legacy_app LANGUAGES CXX)\n",
        )
        .unwrap();
        assert_eq!(
            parse_cmake_project_name(&cmake),
            Some("legacy_app".to_string())
        );
    }
}
