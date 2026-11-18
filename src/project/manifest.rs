//! Package manifest definitions, validation, parsing, and serialization.

pub use crate::package::{
    Dependency, DependencyError, NameValidationError, PackageId, PackageIdError, PackageMetadata,
    PackageName, validate_package_name,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// The canonical manifest file name used by Crux projects.
pub const MANIFEST_FILENAME: &str = "crux.toml";

/// Supported programming languages for Crux projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectLanguage {
    #[serde(rename = "c")]
    C,
    #[serde(rename = "cpp", alias = "c++")]
    Cpp,
    #[serde(rename = "other")]
    Other,
}

impl ProjectLanguage {
    /// Returns the canonical user-facing display name of the language (e.g. "C", "C++", "Other").
    pub fn display_name(&self) -> &'static str {
        match self {
            ProjectLanguage::C => "C",
            ProjectLanguage::Cpp => "C++",
            ProjectLanguage::Other => "Other",
        }
    }
}

impl fmt::Display for ProjectLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectLanguage::C => write!(f, "c"),
            ProjectLanguage::Cpp => write!(f, "cpp"),
            ProjectLanguage::Other => write!(f, "other"),
        }
    }
}

impl std::str::FromStr for ProjectLanguage {
    type Err = ManifestValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "c" => Ok(ProjectLanguage::C),
            "cpp" | "c++" => Ok(ProjectLanguage::Cpp),
            "other" => Ok(ProjectLanguage::Other),
            other => Err(ManifestValidationError::UnsupportedLanguage(
                other.to_string(),
            )),
        }
    }
}

/// Returns the default language standard for a given project language.
pub fn default_standard_for_language(lang: ProjectLanguage) -> Option<&'static str> {
    match lang {
        ProjectLanguage::C => Some("c17"),
        ProjectLanguage::Cpp => Some("c++20"),
        ProjectLanguage::Other => None,
    }
}

/// Checks if a string is a recognized C language standard.
pub fn is_valid_c_standard(std: &str) -> bool {
    matches!(
        std.trim().to_ascii_lowercase().as_str(),
        "c89" | "c90" | "c99" | "c11" | "c17" | "c18" | "c23"
    )
}

/// Checks if a string is a recognized C++ language standard.
pub fn is_valid_cpp_standard(std: &str) -> bool {
    matches!(
        std.trim().to_ascii_lowercase().as_str(),
        "c++98"
            | "cpp98"
            | "c++03"
            | "cpp03"
            | "c++11"
            | "cpp11"
            | "c++14"
            | "cpp14"
            | "c++17"
            | "cpp17"
            | "c++20"
            | "cpp20"
            | "c++23"
            | "cpp23"
            | "c++26"
            | "cpp26"
    )
}

/// Normalizes a language standard string (e.g. "cpp20" -> "c++20", "C17" -> "c17").
pub fn normalize_standard(std: &str) -> String {
    let lower = std.trim().to_ascii_lowercase();
    if let Some(stripped) = lower.strip_prefix("cpp") {
        format!("c++{stripped}")
    } else {
        lower
    }
}

/// Errors that can occur when validating manifest semantic constraints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestValidationError {
    InvalidPackageName(NameValidationError),
    InvalidVersion(String),
    UnsupportedLanguage(String),
    UnsupportedStandard {
        language: ProjectLanguage,
        standard: String,
    },
    StandardNotAllowedForLanguage {
        language: ProjectLanguage,
        standard: String,
    },
    InvalidDependencyName {
        name: String,
        error: NameValidationError,
    },
    InvalidDependencyVersionReq {
        name: String,
        error: String,
    },
    EmptyDependencyVersionReq {
        name: String,
    },
}

impl fmt::Display for ManifestValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestValidationError::InvalidPackageName(err) => {
                write!(f, "invalid package name: {err}")
            }
            ManifestValidationError::InvalidVersion(err) => {
                write!(f, "{err}")
            }
            ManifestValidationError::UnsupportedLanguage(lang) => {
                write!(
                    f,
                    "unsupported project language '{lang}': supported languages are 'c', 'cpp', and 'other'"
                )
            }
            ManifestValidationError::UnsupportedStandard { language, standard } => {
                write!(
                    f,
                    "unsupported standard '{standard}' for language '{language}'"
                )
            }
            ManifestValidationError::StandardNotAllowedForLanguage { language, standard } => {
                write!(
                    f,
                    "language '{language}' does not support a standard (found '{standard}')"
                )
            }
            ManifestValidationError::InvalidDependencyName { name, error } => {
                write!(f, "invalid dependency name '{name}': {error}")
            }
            ManifestValidationError::InvalidDependencyVersionReq { name, error } => {
                write!(
                    f,
                    "invalid version requirement for dependency '{name}': {error}"
                )
            }
            ManifestValidationError::EmptyDependencyVersionReq { name } => {
                write!(
                    f,
                    "version requirement for dependency '{name}' cannot be empty"
                )
            }
        }
    }
}

impl std::error::Error for ManifestValidationError {}

impl From<NameValidationError> for ManifestValidationError {
    fn from(err: NameValidationError) -> Self {
        ManifestValidationError::InvalidPackageName(err)
    }
}

/// Errors encountered when loading, parsing, or validating a manifest.
#[derive(Debug)]
pub enum ManifestError {
    Io(io::Error),
    Toml(toml::de::Error),
    Validation(ManifestValidationError),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Io(err) => write!(f, "failed to read manifest file: {err}"),
            ManifestError::Toml(err) => write!(f, "failed to parse manifest TOML: {err}"),
            ManifestError::Validation(err) => write!(f, "manifest validation error: {err}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<io::Error> for ManifestError {
    fn from(err: io::Error) -> Self {
        ManifestError::Io(err)
    }
}

impl From<toml::de::Error> for ManifestError {
    fn from(err: toml::de::Error) -> Self {
        ManifestError::Toml(err)
    }
}

impl From<ManifestValidationError> for ManifestError {
    fn from(err: ManifestValidationError) -> Self {
        ManifestError::Validation(err)
    }
}

/// Raw representation used during initial TOML deserialization.
///
/// Permits explicit semantic validation error reporting for package names, SemVer strings,
/// language, standard, and dependency requirements.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackageMetadata {
    name: String,
    version: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    standard: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    #[serde(default)]
    package: Option<RawPackageMetadata>,
    #[serde(default)]
    project: Option<RawPackageMetadata>,
    #[serde(default)]
    dependencies: Option<BTreeMap<String, toml::Value>>,
}

/// Represents the top-level Crux manifest (`crux.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub package: PackageMetadata,
    pub dependencies: Vec<Dependency>,
    pub language: Option<ProjectLanguage>,
    pub standard: Option<String>,
    pub is_project: bool,
}

impl Manifest {
    /// Creates a new manifest with initial default values for a given valid package name.
    pub fn new(name: impl Into<String>) -> Result<Self, NameValidationError> {
        let name = PackageName::new(name)?;
        let version = semver::Version::new(0, 1, 0);

        Ok(Self {
            package: PackageMetadata::new(name, version),
            dependencies: Vec::new(),
            language: None,
            standard: None,
            is_project: false,
        })
    }

    /// Sets the project language and marks the manifest as a `[project]`.
    pub fn with_language(mut self, language: ProjectLanguage) -> Self {
        self.language = Some(language);
        self.is_project = true;
        self
    }

    /// Sets the language standard after normalizing.
    pub fn with_standard(mut self, standard: impl Into<String>) -> Self {
        self.standard = Some(normalize_standard(&standard.into()));
        self
    }

    /// Returns the declared project language, if set.
    pub fn language(&self) -> Option<ProjectLanguage> {
        self.language
    }

    /// Returns the declared language standard, if set.
    pub fn standard(&self) -> Option<&str> {
        self.standard.as_deref()
    }

    /// Returns the declared dependencies.
    pub fn dependencies(&self) -> &[Dependency] {
        &self.dependencies
    }

    /// Adds a dependency to the manifest, maintaining alphabetical sorting by name.
    pub fn add_dependency(&mut self, dep: Dependency) {
        self.dependencies.push(dep);
        self.dependencies.sort();
    }

    /// Looks up a dependency by package name.
    pub fn get_dependency(&self, name: &str) -> Option<&Dependency> {
        self.dependencies.iter().find(|d| d.name().as_str() == name)
    }

    /// Validates the semantic constraints of the manifest.
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        validate_package_name(&self.package.name)
            .map_err(ManifestValidationError::InvalidPackageName)?;

        if let Some(lang) = self.language {
            if let Some(std) = &self.standard {
                match lang {
                    ProjectLanguage::C => {
                        if !is_valid_c_standard(std) {
                            return Err(ManifestValidationError::UnsupportedStandard {
                                language: lang,
                                standard: std.clone(),
                            });
                        }
                    }
                    ProjectLanguage::Cpp => {
                        if !is_valid_cpp_standard(std) {
                            return Err(ManifestValidationError::UnsupportedStandard {
                                language: lang,
                                standard: std.clone(),
                            });
                        }
                    }
                    ProjectLanguage::Other => {
                        return Err(ManifestValidationError::StandardNotAllowedForLanguage {
                            language: lang,
                            standard: std.clone(),
                        });
                    }
                }
            }
        } else if let Some(std) = &self.standard {
            return Err(ManifestValidationError::UnsupportedStandard {
                language: ProjectLanguage::Other,
                standard: std.clone(),
            });
        }

        for dep in &self.dependencies {
            validate_package_name(dep.name()).map_err(|err| {
                ManifestValidationError::InvalidDependencyName {
                    name: dep.name().to_string(),
                    error: err,
                }
            })?;
        }

        Ok(())
    }

    /// Parses a manifest from a TOML string and verifies its semantic validity.
    pub fn from_toml_str(content: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest = toml::from_str(content).map_err(ManifestError::Toml)?;

        let (raw_pkg, is_project) = match (raw.project, raw.package) {
            (Some(proj), None) => (proj, true),
            (None, Some(pkg)) => (pkg, false),
            (Some(_), Some(_)) => {
                return Err(ManifestError::Toml(serde::de::Error::custom(
                    "manifest cannot contain both [project] and [package] sections",
                )));
            }
            (None, None) => {
                return Err(ManifestError::Toml(serde::de::Error::custom(
                    "missing [project] or [package] section in manifest",
                )));
            }
        };

        let name =
            PackageName::new(&raw_pkg.name).map_err(ManifestValidationError::InvalidPackageName)?;

        let version = semver::Version::parse(&raw_pkg.version).map_err(|err| {
            ManifestValidationError::InvalidVersion(format!(
                "invalid semantic version '{}': {err}",
                raw_pkg.version
            ))
        })?;

        let language = match raw_pkg.language {
            Some(ref lang_str) => Some(lang_str.parse::<ProjectLanguage>()?),
            None => None,
        };

        let standard = raw_pkg.standard.map(|s| normalize_standard(&s));

        let mut package = PackageMetadata::new(name, version);
        if let Some(desc) = raw_pkg.description {
            package = package.with_description(desc);
        }
        if let Some(lic) = raw_pkg.license {
            package = package.with_license(lic);
        }
        if let Some(hp) = raw_pkg.homepage {
            package = package.with_homepage(hp);
        }
        if let Some(repo) = raw_pkg.repository {
            package = package.with_repository(repo);
        }

        let mut dependencies = Vec::new();
        if let Some(raw_deps) = raw.dependencies {
            for (dep_name, dep_val) in raw_deps {
                let pkg_name = PackageName::new(&dep_name).map_err(|err| {
                    ManifestValidationError::InvalidDependencyName {
                        name: dep_name.clone(),
                        error: err,
                    }
                })?;

                let req_str = match dep_val {
                    toml::Value::String(s) => s,
                    other => {
                        return Err(ManifestError::Validation(
                            ManifestValidationError::InvalidDependencyVersionReq {
                                name: dep_name,
                                error: format!(
                                    "expected string version requirement, found {other}"
                                ),
                            },
                        ));
                    }
                };

                let trimmed = req_str.trim();
                if trimmed.is_empty() {
                    return Err(ManifestError::Validation(
                        ManifestValidationError::EmptyDependencyVersionReq { name: dep_name },
                    ));
                }

                let version_req = semver::VersionReq::parse(trimmed).map_err(|err| {
                    ManifestError::Validation(
                        ManifestValidationError::InvalidDependencyVersionReq {
                            name: dep_name,
                            error: format!(
                                "invalid semantic version requirement '{trimmed}': {err}"
                            ),
                        },
                    )
                })?;

                dependencies.push(Dependency::new(pkg_name, version_req));
            }
        }

        dependencies.sort();

        let manifest = Manifest {
            package,
            dependencies,
            language,
            standard,
            is_project,
        };

        manifest.validate()?;

        Ok(manifest)
    }

    /// Reads and parses a manifest file from disk.
    pub fn load_from_path(path: &Path) -> Result<Self, ManifestError> {
        let content = fs::read_to_string(path).map_err(ManifestError::Io)?;
        Self::from_toml_str(&content)
    }

    /// Serializes the manifest into valid TOML format.
    pub fn to_toml_string(&self) -> String {
        let section = if self.is_project || self.language.is_some() {
            "project"
        } else {
            "package"
        };

        let mut out = format!(
            "[{section}]\nname = \"{}\"\nversion = \"{}\"\n",
            self.package.name, self.package.version
        );

        if let Some(lang) = self.language {
            out.push_str(&format!("language = \"{lang}\"\n"));
        }
        if let Some(std) = &self.standard {
            out.push_str(&format!("standard = \"{std}\"\n"));
        }
        if let Some(desc) = &self.package.description {
            if !desc.is_empty() {
                out.push_str(&format!("description = \"{desc}\"\n"));
            }
        }
        if let Some(lic) = &self.package.license {
            out.push_str(&format!("license = \"{lic}\"\n"));
        }
        if let Some(hp) = &self.package.homepage {
            out.push_str(&format!("homepage = \"{hp}\"\n"));
        }
        if let Some(repo) = &self.package.repository {
            out.push_str(&format!("repository = \"{repo}\"\n"));
        }

        if self.is_project || self.language.is_some() || !self.dependencies.is_empty() {
            out.push_str("\n[dependencies]\n");
            let mut sorted_deps = self.dependencies.clone();
            sorted_deps.sort();
            for dep in sorted_deps {
                out.push_str(&format!("{} = \"{}\"\n", dep.name(), dep.version_req()));
            }
        }

        out
    }
}

/// Renders a single `[dependencies]` entry exactly as Crux writes it.
pub fn render_dependency_entry(dep: &Dependency) -> String {
    format!("{} = \"{}\"", dep.name(), dep.version_req())
}

/// Returns the section header used by the manifest's dependency table.
const DEPENDENCIES_SECTION: &str = "[dependencies]";

fn section_bounds(lines: &[String], section: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|line| line.trim() == section)?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, line)| {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                Some(index)
            } else {
                None
            }
        })
        .unwrap_or(lines.len());
    Some((start, end))
}

fn line_matches_dependency(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return false;
    }

    let Some((raw_key, _)) = trimmed.split_once('=') else {
        return false;
    };
    let key = raw_key.trim();
    let unquoted = if key.len() >= 2
        && ((key.starts_with('"') && key.ends_with('"'))
            || (key.starts_with('\'') && key.ends_with('\'')))
    {
        &key[1..key.len() - 1]
    } else {
        key
    };
    unquoted == name
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Inserts or updates a dependency entry in raw `crux.toml` content while
/// preserving unrelated comments, ordering, and formatting.
///
/// Only the `[dependencies]` table is touched. When the table does not exist it
/// is appended. Existing entries keep their original order.
pub fn upsert_dependency_in_toml(content: &str, dep: &Dependency) -> Result<String, ManifestError> {
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let entry = render_dependency_entry(dep);
    let name = dep.name().as_str();

    match section_bounds(&lines, DEPENDENCIES_SECTION) {
        Some((start, end)) => {
            if let Some(index) =
                (start + 1..end).find(|&index| line_matches_dependency(&lines[index], name))
            {
                let indent = leading_whitespace(&lines[index]).to_string();
                lines[index] = format!("{indent}{entry}");
            } else {
                // Insert after the last actual entry so trailing comments at the
                // end of the table keep their position instead of ending up
                // above the new dependency.
                let insert_at = (start + 1..end)
                    .rev()
                    .find(|&index| {
                        let trimmed = lines[index].trim();
                        !trimmed.is_empty() && !trimmed.starts_with('#')
                    })
                    .map(|index| index + 1)
                    .unwrap_or(start + 1);
                lines.insert(insert_at, entry);
            }
        }
        None => {
            if let Some(last) = lines.last()
                && !last.trim().is_empty()
            {
                lines.push(String::new());
            }
            lines.push(DEPENDENCIES_SECTION.to_string());
            lines.push(entry);
        }
    }

    let mut rendered = lines.join("\n");
    rendered.push('\n');
    Ok(rendered)
}

/// Removes a dependency entry from raw `crux.toml` content.
///
/// Returns `Ok(None)` when the dependency is not present in the dependency
/// table. Unrelated comments, ordering, and formatting are preserved.
pub fn remove_dependency_from_toml(
    content: &str,
    name: &str,
) -> Result<Option<String>, ManifestError> {
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();

    let Some((start, end)) = section_bounds(&lines, DEPENDENCIES_SECTION) else {
        return Ok(None);
    };

    let Some(index) = (start + 1..end).find(|&index| line_matches_dependency(&lines[index], name))
    else {
        return Ok(None);
    };

    lines.remove(index);
    let mut rendered = lines.join("\n");
    rendered.push('\n');
    Ok(Some(rendered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_manifest_parsing() {
        let toml_str = r#"
[package]
name = "my-project"
version = "1.2.3"
description = "A test project"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.package.name.as_str(), "my-project");
        assert_eq!(manifest.package.version.to_string(), "1.2.3");
        assert_eq!(
            manifest.package.description.as_deref(),
            Some("A test project")
        );
        assert_eq!(manifest.package.id().to_string(), "my-project 1.2.3");
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_valid_manifest_optional_description() {
        let toml_str = r#"
[package]
name = "no-desc"
version = "0.1.0"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.package.description, None);
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_valid_manifest_all_optional_fields() {
        let toml_str = r#"
[package]
name = "full-metadata"
version = "2.0.0-rc.1"
description = "Full metadata example"
license = "MIT OR Apache-2.0"
homepage = "https://crux-pm.org"
repository = "https://github.com/crux/full-metadata"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.package.name.as_str(), "full-metadata");
        assert_eq!(manifest.package.version.to_string(), "2.0.0-rc.1");
        assert_eq!(
            manifest.package.description.as_deref(),
            Some("Full metadata example")
        );
        assert_eq!(
            manifest.package.license.as_deref(),
            Some("MIT OR Apache-2.0")
        );
        assert_eq!(
            manifest.package.homepage.as_deref(),
            Some("https://crux-pm.org")
        );
        assert_eq!(
            manifest.package.repository.as_deref(),
            Some("https://github.com/crux/full-metadata")
        );
        assert_eq!(
            manifest.package.id().to_string(),
            "full-metadata 2.0.0-rc.1"
        );
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_manifest_with_dependencies() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"
description = "My C++ application"

[dependencies]
fmt = "^11.1"
zlib = ">=1.2, <2.0"
openssl = "~3.4"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.dependencies.len(), 3);

        // Dependencies are sorted alphabetically by package name
        assert_eq!(manifest.dependencies[0].name().as_str(), "fmt");
        assert_eq!(manifest.dependencies[0].version_req().to_string(), "^11.1");

        assert_eq!(manifest.dependencies[1].name().as_str(), "openssl");
        assert_eq!(manifest.dependencies[1].version_req().to_string(), "~3.4");

        assert_eq!(manifest.dependencies[2].name().as_str(), "zlib");
        assert_eq!(
            manifest.dependencies[2].version_req().to_string(),
            ">=1.2, <2.0"
        );

        // Serialization roundtrip
        let serialized = manifest.to_toml_string();
        let parsed_again = Manifest::from_toml_str(&serialized).unwrap();
        assert_eq!(parsed_again, manifest);
    }

    #[test]
    fn test_manifest_with_empty_dependencies_section() {
        let toml_str = r#"
[package]
name = "empty-deps"
version = "0.1.0"

[dependencies]
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_manifest_rejects_invalid_dependency_name() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
"-invalid-dep" = "^1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidDependencyName { .. }
            ))
        ));
    }

    #[test]
    fn test_manifest_rejects_empty_dependency_version_req() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = ""
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::EmptyDependencyVersionReq { .. }
            ))
        ));
    }

    #[test]
    fn test_manifest_rejects_invalid_dependency_version_req() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = "1.0.0.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidDependencyVersionReq { .. }
            ))
        ));
    }

    #[test]
    fn test_manifest_rejects_malformed_dependency_type() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = 123
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidDependencyVersionReq { .. }
            ))
        ));
    }

    #[test]
    fn test_invalid_toml_syntax() {
        let toml_str = "not valid toml ::::";
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_missing_package_section() {
        let toml_str = r#"
[other]
name = "no-package-section"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_missing_required_field_name() {
        let toml_str = r#"
[package]
version = "0.1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_invalid_package_name_in_manifest() {
        let toml_str = r#"
[package]
name = "invalid name with spaces"
version = "0.1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidPackageName(_)
            ))
        ));
    }

    #[test]
    fn test_invalid_semantic_version() {
        let toml_str = r#"
[package]
name = "bad-ver"
version = "1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::InvalidVersion(_)
            ))
        ));
    }

    #[test]
    fn test_rejects_unknown_fields_in_package() {
        let toml_str = r#"
[package]
name = "test-pkg"
version = "0.1.0"
unknown_field = "unexpected"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_rejects_unknown_top_level_section() {
        let toml_str = r#"
[package]
name = "test-pkg"
version = "0.1.0"

[unexpected_section]
foo = "bar"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_valid_project_manifest_c() {
        let toml_str = r#"
[project]
name = "my-c-app"
version = "0.1.0"
language = "c"
standard = "c17"

[dependencies]
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.package.name.as_str(), "my-c-app");
        assert_eq!(manifest.package.version.to_string(), "0.1.0");
        assert_eq!(manifest.language, Some(ProjectLanguage::C));
        assert_eq!(manifest.standard.as_deref(), Some("c17"));
        assert!(manifest.is_project);
        assert!(manifest.dependencies.is_empty());

        let serialized = manifest.to_toml_string();
        assert!(serialized.contains("[project]"));
        assert!(serialized.contains("language = \"c\""));
        assert!(serialized.contains("standard = \"c17\""));
        assert!(serialized.contains("[dependencies]"));
    }

    #[test]
    fn test_valid_project_manifest_cpp() {
        let toml_str = r#"
[project]
name = "my-cpp-app"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
fmt = "^11.1"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.language, Some(ProjectLanguage::Cpp));
        assert_eq!(manifest.standard.as_deref(), Some("c++20"));
        assert_eq!(manifest.dependencies.len(), 1);

        let serialized = manifest.to_toml_string();
        assert!(serialized.contains("[project]"));
        assert!(serialized.contains("language = \"cpp\""));
        assert!(serialized.contains("standard = \"c++20\""));
        assert!(serialized.contains("fmt = \"^11.1\""));
    }

    #[test]
    fn test_valid_project_manifest_other() {
        let toml_str = r#"
[project]
name = "my-other-project"
version = "0.1.0"
language = "other"

[dependencies]
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        assert_eq!(manifest.language, Some(ProjectLanguage::Other));
        assert_eq!(manifest.standard, None);

        let serialized = manifest.to_toml_string();
        assert!(serialized.contains("[project]"));
        assert!(serialized.contains("language = \"other\""));
        assert!(!serialized.contains("standard ="));
    }

    #[test]
    fn test_project_manifest_rejects_both_package_and_project_sections() {
        let toml_str = r#"
[package]
name = "pkg-part"
version = "0.1.0"

[project]
name = "proj-part"
version = "0.1.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_project_manifest_rejects_missing_sections() {
        let toml_str = r#"
[dependencies]
fmt = "^11.0"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(res, Err(ManifestError::Toml(_))));
    }

    #[test]
    fn test_project_manifest_rejects_invalid_language() {
        let toml_str = r#"
[project]
name = "bad-lang"
version = "0.1.0"
language = "rust"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::UnsupportedLanguage(_)
            ))
        ));
    }

    #[test]
    fn test_project_manifest_rejects_invalid_standard_for_c() {
        let toml_str = r#"
[project]
name = "bad-c-std"
version = "0.1.0"
language = "c"
standard = "c++20"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::UnsupportedStandard { .. }
            ))
        ));
    }

    #[test]
    fn test_project_manifest_rejects_invalid_standard_for_cpp() {
        let toml_str = r#"
[project]
name = "bad-cpp-std"
version = "0.1.0"
language = "cpp"
standard = "c17"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::UnsupportedStandard { .. }
            ))
        ));
    }

    #[test]
    fn test_project_manifest_rejects_standard_for_other() {
        let toml_str = r#"
[project]
name = "other-with-std"
version = "0.1.0"
language = "other"
standard = "c17"
"#;
        let res = Manifest::from_toml_str(toml_str);
        assert!(matches!(
            res,
            Err(ManifestError::Validation(
                ManifestValidationError::StandardNotAllowedForLanguage { .. }
            ))
        ));
    }

    #[test]
    fn test_upsert_dependency_preserves_comments_and_ordering() {
        let content = r#"# top comment
[project]
name = "demo"
version = "0.1.0"

# dependency comment
[dependencies]
fmt = "^11"

# trailing comment
"#;
        let dep = Dependency::try_new("zlib", "*").unwrap();
        let updated = upsert_dependency_in_toml(content, &dep).unwrap();

        assert!(updated.starts_with("# top comment\n"));
        assert!(
            updated.contains("# dependency comment\n[dependencies]\nfmt = \"^11\"\nzlib = \"*\"")
        );
        assert!(updated.ends_with("\n# trailing comment\n"));

        // The generated document must still parse and contain both entries.
        let manifest = Manifest::from_toml_str(&updated).unwrap();
        assert_eq!(manifest.dependencies.len(), 2);
        assert_eq!(manifest.dependencies[0].name().as_str(), "fmt");
        assert_eq!(manifest.dependencies[1].name().as_str(), "zlib");
    }

    #[test]
    fn test_upsert_dependency_updates_in_place_and_keeps_indentation() {
        let content = "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\n    fmt = \"^10\"\n";
        let dep = Dependency::try_new("fmt", "^11").unwrap();
        let updated = upsert_dependency_in_toml(content, &dep).unwrap();
        assert!(updated.contains("    fmt = \"^11\"\n"));
        assert_eq!(updated.matches("fmt =").count(), 1);
    }

    #[test]
    fn test_upsert_dependency_appends_section_when_absent() {
        let content = "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n";
        let dep = Dependency::try_new("fmt", "*").unwrap();
        let updated = upsert_dependency_in_toml(content, &dep).unwrap();
        assert!(updated.ends_with("\n[dependencies]\nfmt = \"*\"\n"));
        let manifest = Manifest::from_toml_str(&updated).unwrap();
        assert_eq!(manifest.dependencies.len(), 1);
    }

    #[test]
    fn test_remove_dependency_only_touches_named_entry() {
        let content = "# header\n[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nfmt = \"^11\"\n# keep me\nzlib = \"*\"\n";
        let updated = remove_dependency_from_toml(content, "fmt")
            .unwrap()
            .unwrap();

        assert!(updated.contains("# header\n"));
        assert!(!updated.contains("fmt ="));
        assert!(updated.contains("# keep me\nzlib = \"*\"\n"));
        let manifest = Manifest::from_toml_str(&updated).unwrap();
        assert_eq!(manifest.dependencies.len(), 1);
        assert_eq!(manifest.dependencies[0].name().as_str(), "zlib");
    }

    #[test]
    fn test_remove_dependency_returns_none_when_absent() {
        let content =
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nzlib = \"*\"\n";
        assert!(
            remove_dependency_from_toml(content, "fmt")
                .unwrap()
                .is_none()
        );
    }
}
