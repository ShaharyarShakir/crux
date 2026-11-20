//! Core implementation of `crux import`.
//!
//! Import is deliberately conservative: it only writes a Crux manifest and a
//! Crux-managed discovery metadata file. Existing sources, headers, build files,
//! and configuration are never modified or removed.

use super::discovery::{ProjectDiscovery, ProjectDiscoveryError, discover_project};
use super::manifest::{
    MANIFEST_FILENAME, Manifest, ManifestValidationError, ProjectLanguage,
    default_standard_for_language,
};
use crate::core::filesystem::write_atomically;
use crate::core::paths::is_safe_relative_path;
use crate::package::NameValidationError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

/// Name of the Crux-managed metadata file written by import.
pub const IMPORT_METADATA_RELATIVE_PATH: &str = ".crux/import.toml";
const IMPORT_METADATA_VERSION: u32 = 1;

/// Crux-managed metadata describing source roots discovered during import.
///
/// This is intentionally not part of `crux.toml`: it records observed layout
/// facts rather than user-authored configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMetadata {
    pub version: u32,
    #[serde(default)]
    pub source_roots: Vec<String>,
    #[serde(default)]
    pub include_dirs: Vec<String>,
}

impl ImportMetadata {
    fn new(source_roots: Vec<String>, include_dirs: Vec<String>) -> Self {
        Self {
            version: IMPORT_METADATA_VERSION,
            source_roots,
            include_dirs,
        }
    }

    /// Reads import metadata from a Crux project, if present.
    pub fn load_from_project(project_root: &Path) -> Result<Option<Self>, ImportMetadataError> {
        let path = project_root.join(IMPORT_METADATA_RELATIVE_PATH);
        if !path.is_file() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(ImportMetadataError::Io)?;
        let metadata: Self = toml::from_str(&content).map_err(ImportMetadataError::Toml)?;
        if metadata.version != IMPORT_METADATA_VERSION {
            return Err(ImportMetadataError::UnsupportedVersion(metadata.version));
        }

        metadata.validate_paths()?;
        Ok(Some(metadata))
    }

    /// Validates metadata include directories and returns them in manifest order.
    ///
    /// The returned values remain project-relative so generated CMake content
    /// stays relocatable. Symlinks escaping the project root are rejected.
    pub fn resolve_include_dirs(
        &self,
        project_root: &Path,
    ) -> Result<Vec<String>, ImportMetadataError> {
        self.validate_paths()?;
        let canonical_root = project_root
            .canonicalize()
            .map_err(ImportMetadataError::Io)?;
        let mut resolved = Vec::new();

        for value in &self.include_dirs {
            let candidate = canonical_root.join(value);
            if !candidate.is_dir() {
                continue;
            }
            let canonical = candidate.canonicalize().map_err(ImportMetadataError::Io)?;
            if !canonical.starts_with(&canonical_root) {
                return Err(ImportMetadataError::PathOutsideProject(value.clone()));
            }
            resolved.push(value.clone());
        }

        Ok(resolved)
    }

    fn validate_paths(&self) -> Result<(), ImportMetadataError> {
        for root in &self.source_roots {
            validate_relative_entry(root)?;
        }
        for include in &self.include_dirs {
            validate_relative_entry(include)?;
        }
        Ok(())
    }

    fn to_toml_string(&self) -> Result<String, ImportMetadataError> {
        toml::to_string(self).map_err(ImportMetadataError::TomlSerialize)
    }
}

/// Errors encountered while reading Crux import metadata.
#[derive(Debug)]
pub enum ImportMetadataError {
    Io(io::Error),
    Toml(toml::de::Error),
    TomlSerialize(toml::ser::Error),
    UnsupportedVersion(u32),
    UnsafePath(String),
    PathOutsideProject(String),
}

impl fmt::Display for ImportMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read import metadata: {err}"),
            Self::Toml(err) => write!(f, "invalid import metadata TOML: {err}"),
            Self::TomlSerialize(err) => {
                write!(f, "failed to serialize import metadata: {err}")
            }
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported import metadata version '{version}'")
            }
            Self::UnsafePath(path) => {
                write!(f, "unsafe relative path in import metadata: '{path}'")
            }
            Self::PathOutsideProject(path) => {
                write!(
                    f,
                    "import metadata path '{path}' resolves outside the project root"
                )
            }
        }
    }
}

impl std::error::Error for ImportMetadataError {}

fn validate_relative_entry(value: &str) -> Result<(), ImportMetadataError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || !is_safe_relative_path(path)
    {
        return Err(ImportMetadataError::UnsafePath(value.to_string()));
    }
    Ok(())
}

/// Options for a single import operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// Print the proposed manifest and report without writing files.
    pub dry_run: bool,
    /// Accept low-confidence discoveries and refresh Crux-managed metadata.
    ///
    /// This never permits overwriting an existing `crux.toml`.
    pub force: bool,
}

/// Errors that can occur during project import.
#[derive(Debug)]
pub enum ImportError {
    InvalidProjectDirectory {
        path: PathBuf,
        reason: String,
    },
    AlreadyInitialized(PathBuf),
    UnableToIdentify {
        reason: String,
        discovery: Box<ProjectDiscovery>,
    },
    NameValidation(NameValidationError),
    Validation(ManifestValidationError),
    Discovery(ProjectDiscoveryError),
    Metadata(ImportMetadataError),
    MetadataConflict(PathBuf),
    WriteFile {
        path: PathBuf,
        source: io::Error,
    },
    Io(io::Error),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProjectDirectory { path, reason } => {
                write!(
                    f,
                    "error: invalid project directory '{}': {reason}",
                    path.display()
                )
            }
            Self::AlreadyInitialized(path) => {
                write!(
                    f,
                    "error: project is already initialized at '{}'; existing crux.toml was preserved",
                    path.display()
                )
            }
            Self::UnableToIdentify { reason, discovery } => {
                writeln!(f, "error: unable to confidently import project: {reason}")?;
                writeln!(f, "Detected:")?;
                writeln!(f, "  build system: {}", discovery.build_system)?;
                writeln!(
                    f,
                    "  source directories: {}",
                    join_or_none(&discovery.source_dirs)
                )?;
                writeln!(
                    f,
                    "  include directories: {}",
                    join_or_none(&discovery.include_dirs)
                )?;
                writeln!(f, "  source files: {}", discovery.source_files.len())?;
                writeln!(f, "  likely targets: {}", join_or_none(&discovery.targets))?;
                writeln!(f, "Detected facts:")?;
                for warning in &discovery.warnings {
                    writeln!(f, "  - {warning}")?;
                }
                write!(
                    f,
                    "Configure these fields manually in crux.toml, or re-run with --force only when the reported limitation is acceptable."
                )
            }
            Self::NameValidation(err) => write!(f, "error: invalid project name: {err}"),
            Self::Validation(err) => write!(f, "error: {err}"),
            Self::Discovery(err) => write!(f, "error: {err}"),
            Self::Metadata(err) => write!(f, "error: {err}"),
            Self::MetadataConflict(path) => {
                write!(
                    f,
                    "error: conflicting Crux import metadata at '{}'; resolve it manually or re-run with --force to refresh this Crux-managed file",
                    path.display()
                )
            }
            Self::WriteFile { path, source } => {
                write!(f, "error: failed to write '{}': {source}", path.display())
            }
            Self::Io(err) => write!(f, "error: I/O error: {err}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<ProjectDiscoveryError> for ImportError {
    fn from(err: ProjectDiscoveryError) -> Self {
        Self::Discovery(err)
    }
}

impl From<NameValidationError> for ImportError {
    fn from(err: NameValidationError) -> Self {
        Self::NameValidation(err)
    }
}

impl From<ManifestValidationError> for ImportError {
    fn from(err: ManifestValidationError) -> Self {
        Self::Validation(err)
    }
}

impl From<ImportMetadataError> for ImportError {
    fn from(err: ImportMetadataError) -> Self {
        Self::Metadata(err)
    }
}

impl From<io::Error> for ImportError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Result of a successful or dry-run import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub project_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest_content: String,
    pub metadata_path: PathBuf,
    pub package_name: String,
    pub language: ProjectLanguage,
    pub standard: String,
    pub discovery: ProjectDiscovery,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
    /// Files actually created on disk. Empty for dry runs.
    pub created_files: Vec<PathBuf>,
    /// Crux-managed files explicitly refreshed with `--force`.
    pub updated_files: Vec<PathBuf>,
    /// Files a dry run would create.
    pub would_create_files: Vec<PathBuf>,
    /// Files a dry run would refresh with `--force`.
    pub would_update_files: Vec<PathBuf>,
    pub preserved_files: Vec<String>,
    pub dry_run: bool,
    pub force: bool,
}

/// Resolves, discovers, and optionally writes a Crux import for `target_dir`.
pub fn import_project(
    target_dir: &Path,
    options: &ImportOptions,
) -> Result<ImportSummary, ImportError> {
    let canonical_dir = target_dir.canonicalize().map_err(|err| {
        let reason = match err.kind() {
            io::ErrorKind::NotFound => "directory does not exist".to_string(),
            _ => format!("cannot resolve directory: {err}"),
        };
        ImportError::InvalidProjectDirectory {
            path: target_dir.to_path_buf(),
            reason,
        }
    })?;

    if !canonical_dir.is_dir() {
        return Err(ImportError::InvalidProjectDirectory {
            path: target_dir.to_path_buf(),
            reason: "path is not a directory".to_string(),
        });
    }

    let manifest_path = canonical_dir.join(MANIFEST_FILENAME);
    if manifest_path.exists() {
        return Err(ImportError::AlreadyInitialized(manifest_path));
    }

    let discovery = discover_project(&canonical_dir)?;

    if discovery.source_files.is_empty() {
        return Err(ImportError::UnableToIdentify {
            reason: "no C or C++ compilation units were found".to_string(),
            discovery: Box::new(discovery),
        });
    }

    let has_c = discovery
        .source_files
        .iter()
        .any(|file| file.to_ascii_lowercase().ends_with(".c"));
    let has_cpp = discovery.source_files.iter().any(|file| {
        let lower = file.to_ascii_lowercase();
        lower.ends_with(".cpp") || lower.ends_with(".cc") || lower.ends_with(".cxx")
    });

    if has_c && has_cpp {
        return Err(ImportError::UnableToIdentify {
            reason: "mixed C and C++ compilation units cannot be represented by the current application build engine"
                .to_string(),
            discovery: Box::new(discovery),
        });
    }

    let language = if has_cpp {
        ProjectLanguage::Cpp
    } else {
        ProjectLanguage::C
    };

    if discovery.entry_points.is_empty() && !options.force {
        return Err(ImportError::UnableToIdentify {
            reason: "no conventional main() entry point was found; Crux creates a single executable target"
                .to_string(),
            discovery: Box::new(discovery),
        });
    }

    if discovery.targets.len() > 1 && !options.force {
        return Err(ImportError::UnableToIdentify {
            reason: format!(
                "{} likely executable entry points were found; Crux creates a single executable target",
                discovery.targets.len()
            ),
            discovery: Box::new(discovery),
        });
    }

    let name_hint = discovery.cmake_project_name.clone().unwrap_or_else(|| {
        canonical_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "legacy-app".to_string())
    });
    let package_name = sanitize_project_name(&name_hint)?;

    let standard = default_standard_for_language(language)
        .unwrap_or("c++20")
        .to_string();

    let mut manifest = Manifest::new(&package_name)?;
    manifest = manifest.with_language(language).with_standard(&standard);
    manifest.validate()?;
    let manifest_content = manifest.to_toml_string();

    let metadata = ImportMetadata::new(
        discovery.source_dirs.clone(),
        discovery.include_dirs.clone(),
    );
    let metadata_content = metadata.to_toml_string()?;
    let metadata_path = canonical_dir.join(IMPORT_METADATA_RELATIVE_PATH);

    let metadata_existed = metadata_path.exists();
    let metadata_changed = if metadata_existed {
        let existing = fs::read_to_string(&metadata_path)?;
        existing != metadata_content
    } else {
        false
    };
    if metadata_changed && !options.force {
        return Err(ImportError::MetadataConflict(metadata_path));
    }

    let mut assumptions = vec![
        format!(
            "project name '{package_name}' derived from {}",
            if discovery.cmake_project_name.is_some() {
                "the CMake project() declaration"
            } else {
                "the project directory name"
            }
        ),
        format!(
            "language '{}' and standard '{standard}' inferred from source extensions",
            language.display_name()
        ),
        format!(
            "source roots {} recorded in Crux-managed import metadata",
            join_or_none(&discovery.source_dirs)
        ),
        "no dependencies were declared because import never guesses dependency edges".to_string(),
    ];
    assumptions.sort();

    let mut limitations = discovery.warnings.clone();
    if discovery.entry_points.is_empty() {
        limitations.push(
            "no conventional main() entry point was found; crux build may fail to link an executable"
                .to_string(),
        );
    }
    if discovery.targets.len() > 1 {
        limitations.push(
            "only one executable target is supported by the current build engine".to_string(),
        );
    }
    limitations.sort();
    limitations.dedup();

    let mut preserved_files = Vec::new();
    for name in ["CMakeLists.txt", "Makefile", "makefile", "GNUmakefile"] {
        if canonical_dir.join(name).is_file() {
            preserved_files.push(name.to_string());
        }
    }
    for dir in &discovery.source_dirs {
        if dir != "." {
            preserved_files.push(format!("{dir}/"));
        }
    }
    for dir in &discovery.include_dirs {
        preserved_files.push(format!("{dir}/"));
    }
    preserved_files.sort();
    preserved_files.dedup();

    let mut created_files = Vec::new();
    let mut updated_files = Vec::new();
    let mut would_create_files = Vec::new();
    let mut would_update_files = Vec::new();

    if options.dry_run {
        if !metadata_existed {
            would_create_files.push(metadata_path.clone());
        } else if metadata_changed {
            would_update_files.push(metadata_path.clone());
        }
        would_create_files.push(manifest_path.clone());
    } else {
        if !metadata_existed {
            write_new_file(&metadata_path, metadata_content.as_bytes())?;
            created_files.push(metadata_path.clone());
        } else if metadata_changed {
            write_atomically(&metadata_path, metadata_content.as_bytes())
                .map_err(ImportError::Io)?;
            updated_files.push(metadata_path.clone());
        }
        write_new_file(&manifest_path, manifest_content.as_bytes())?;
        created_files.push(manifest_path.clone());
    }

    Ok(ImportSummary {
        project_dir: canonical_dir,
        manifest_path,
        manifest_content,
        metadata_path,
        package_name,
        language,
        standard,
        discovery,
        assumptions,
        limitations,
        created_files,
        updated_files,
        would_create_files,
        would_update_files,
        preserved_files,
        dry_run: options.dry_run,
        force: options.force,
    })
}

fn sanitize_project_name(input: &str) -> Result<String, NameValidationError> {
    let mut sanitized = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            if ch == '-' && (sanitized.is_empty() || sanitized.ends_with('-')) {
                continue;
            }
            sanitized.push(ch.to_ascii_lowercase());
        } else if !sanitized.is_empty() && !sanitized.ends_with('-') {
            sanitized.push('-');
        }
        if sanitized.len() >= 128 {
            break;
        }
    }
    while sanitized.ends_with('-') {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        sanitized.push_str("legacy-app");
    }
    crate::package::validate_package_name(&sanitized)?;
    Ok(sanitized)
}

fn write_new_file(path: &Path, content: &[u8]) -> Result<(), ImportError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| ImportError::WriteFile {
            path: path.to_path_buf(),
            source,
        })?;

    let write_result = file
        .write_all(content)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all());
    if let Err(source) = write_result {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(ImportError::WriteFile {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

/// Renders the deterministic import report shown by the CLI.
pub fn format_import_report(summary: &ImportSummary) -> String {
    let mut out = String::new();
    out.push_str("Crux Project Import\n\n");
    out.push_str(&format!("Project: {}\n", summary.package_name));
    out.push_str(&format!("Language: {}\n", summary.language.display_name()));
    out.push_str(&format!(
        "Build system: {}\n",
        summary.discovery.build_system.display_name()
    ));
    out.push_str(&format!(
        "Source directories: {}\n",
        join_or_none(&summary.discovery.source_dirs)
    ));
    out.push_str(&format!(
        "Include directories: {}\n",
        join_or_none(&summary.discovery.include_dirs)
    ));
    out.push_str(&format!(
        "Detected targets: {}\n",
        join_or_none(&summary.discovery.targets)
    ));
    out.push_str("Dependencies: None detected\n\n");

    out.push_str("Detected facts:\n");
    out.push_str(&format!(
        "  - {} C/C++ source files across {} directory/ies\n",
        summary.discovery.source_files.len(),
        summary.discovery.source_dirs.len()
    ));
    out.push_str(&format!(
        "  - {} header files\n",
        summary.discovery.header_files.len()
    ));
    if !summary.discovery.entry_points.is_empty() {
        out.push_str(&format!(
            "  - entry points: {}\n",
            join_or_none(&summary.discovery.entry_points)
        ));
    }
    for dir in &summary.discovery.test_dirs {
        out.push_str(&format!("  - test directory: {dir}\n"));
    }
    if summary.discovery.targets.is_empty() {
        out.push_str("  - no conventional executable entry point detected\n");
    }

    out.push_str("\nAssumptions:\n");
    for assumption in &summary.assumptions {
        out.push_str(&format!("  - {assumption}\n"));
    }

    if !summary.limitations.is_empty() {
        out.push_str("\nLimitations:\n");
        for limitation in &summary.limitations {
            out.push_str(&format!("  - {limitation}\n"));
        }
    }

    if summary.force {
        out.push_str("\nForce mode: existing crux.toml is never overwritten; only Crux-managed discovery metadata may be refreshed after explicit notice.\n");
    }

    if summary.dry_run {
        out.push_str("\nDry run: no files were written.\n");
        if !summary.would_create_files.is_empty() {
            out.push_str("Would create:\n");
            for path in &summary.would_create_files {
                out.push_str(&format!("  {}\n", path.display()));
            }
        }
        if !summary.would_update_files.is_empty() {
            out.push_str("Would update (--force):\n");
            for path in &summary.would_update_files {
                out.push_str(&format!("  {}\n", path.display()));
            }
        }
        out.push_str("\nProposed crux.toml:\n");
        out.push_str(&summary.manifest_content);
        return out;
    }

    if !summary.created_files.is_empty() {
        out.push_str("\nCreated:\n");
        for path in &summary.created_files {
            out.push_str(&format!("  {}\n", path.display()));
        }
    }
    if !summary.updated_files.is_empty() {
        out.push_str("\nUpdated (--force):\n");
        for path in &summary.updated_files {
            out.push_str(&format!("  {}\n", path.display()));
        }
    }
    out.push_str("\nPreserved:\n");
    for path in &summary.preserved_files {
        out.push_str(&format!("  {path}\n"));
    }

    out.push_str("\nNext steps:\n");
    out.push_str("  1. Review crux.toml\n");
    out.push_str("  2. Run crux check\n");
    out.push_str("  3. Run crux build\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_project_name() {
        assert_eq!(sanitize_project_name("Legacy App!").unwrap(), "legacy-app");
        assert_eq!(sanitize_project_name("---").unwrap(), "legacy-app");
        assert_eq!(sanitize_project_name("A__B").unwrap(), "a__b");
    }

    #[test]
    fn test_import_metadata_roundtrip() {
        let metadata = ImportMetadata::new(
            vec!["src".to_string(), "lib".to_string()],
            vec!["include".to_string()],
        );
        let content = metadata.to_toml_string().unwrap();
        let parsed: ImportMetadata = toml::from_str(&content).unwrap();
        assert_eq!(parsed, metadata);
    }

    #[test]
    fn test_validate_relative_entry_rejects_traversal() {
        assert!(validate_relative_entry("../outside").is_err());
        assert!(validate_relative_entry("/absolute").is_err());
        assert!(validate_relative_entry("src").is_ok());
        assert!(validate_relative_entry(".").is_ok());
    }
}
