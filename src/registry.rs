//! Git-backed package registry, package definition models, and local registry management.

use crate::catalog::{CatalogEntry, CatalogError, PackageCatalog};
use crate::dependency::Dependency;
use crate::lock::{Checksum, PackageSource};
use crate::package::{PackageId, PackageMetadata, PackageName};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Standard file name for registry manifest.
pub const REGISTRY_MANIFEST_FILENAME: &str = "registry.toml";

/// Standard file name for package definitions.
pub const PACKAGE_DEFINITION_FILENAME: &str = "package.toml";

/// Supported registry format version.
pub const CURRENT_REGISTRY_FORMAT_VERSION: u32 = 1;

/// Registry manifest located at `registry.toml` in the registry root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryManifest {
    pub format_version: u32,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Represents the `[source]` section of a `package.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSection {
    #[serde(rename = "type")]
    pub source_type: String,
    pub url: String,
    #[serde(default)]
    pub checksum: Option<String>,
}

/// Represents the `[build]` section of a `package.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSection {
    pub system: String,
}

/// Raw deserialization model for `package.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackageDefinition {
    package: RawPackageMetadataSection,
    source: SourceSection,
    #[serde(default)]
    build: Option<BuildSection>,
    #[serde(default)]
    dependencies: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackageMetadataSection {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
}

/// A validated package definition loaded from a registry's `package.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDefinition {
    pub metadata: PackageMetadata,
    pub source: PackageSourceDefinition,
    pub build: Option<BuildSection>,
    pub dependencies: Vec<Dependency>,
}

/// Validated source definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageSourceDefinition {
    Archive { url: String, checksum: Checksum },
    Git { url: String, rev: Option<String> },
}

/// Errors related to Git registry operations, loading, or configuration.
#[derive(Debug)]
pub enum RegistryError {
    Io(io::Error),
    Git(String),
    Toml(String),
    NotAGitRepository(PathBuf),
    MissingRegistryManifest(PathBuf),
    UnsupportedFormatVersion(u32),
    InvalidPackageName(String),
    InvalidVersion(String),
    MissingPackageManifest(PathBuf),
    IdentityMismatch {
        expected_name: String,
        found_name: String,
        expected_version: String,
        found_version: String,
    },
    SecurityViolation(String),
    DuplicatePackage(PackageId),
    NotFound(String),
    AlreadyExists(String),
    Config(String),
    Validation(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Io(err) => write!(f, "I/O error: {err}"),
            RegistryError::Git(msg) => write!(f, "Git error: {msg}"),
            RegistryError::Toml(msg) => write!(f, "TOML error: {msg}"),
            RegistryError::NotAGitRepository(path) => {
                write!(
                    f,
                    "directory '{}' is not a valid Git repository",
                    path.display()
                )
            }
            RegistryError::MissingRegistryManifest(path) => {
                write!(
                    f,
                    "missing registry manifest '{}': directory is not a Crux registry",
                    path.display()
                )
            }
            RegistryError::UnsupportedFormatVersion(ver) => {
                write!(
                    f,
                    "unsupported registry format version '{ver}': this version of Crux supports version {CURRENT_REGISTRY_FORMAT_VERSION}"
                )
            }
            RegistryError::InvalidPackageName(msg) => write!(f, "invalid package name: {msg}"),
            RegistryError::InvalidVersion(msg) => write!(f, "invalid version: {msg}"),
            RegistryError::MissingPackageManifest(path) => {
                write!(f, "missing package definition '{}'", path.display())
            }
            RegistryError::IdentityMismatch {
                expected_name,
                found_name,
                expected_version,
                found_version,
            } => {
                write!(
                    f,
                    "package definition identity mismatch: directory path specifies '{expected_name} {expected_version}' but package.toml declares '{found_name} {found_version}'"
                )
            }
            RegistryError::SecurityViolation(msg) => write!(f, "security violation: {msg}"),
            RegistryError::DuplicatePackage(id) => {
                write!(f, "duplicate package identity '{id}' in registry")
            }
            RegistryError::NotFound(name) => write!(f, "registry '{name}' not found"),
            RegistryError::AlreadyExists(name) => {
                write!(f, "registry '{name}' is already configured")
            }
            RegistryError::Config(msg) => write!(f, "configuration error: {msg}"),
            RegistryError::Validation(msg) => write!(f, "validation error: {msg}"),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<io::Error> for RegistryError {
    fn from(err: io::Error) -> Self {
        RegistryError::Io(err)
    }
}

/// A local Git-backed Crux package registry.
#[derive(Debug, Clone)]
pub struct GitRegistry {
    name: String,
    path: PathBuf,
    manifest: RegistryManifest,
    packages: BTreeMap<PackageName, BTreeMap<semver::Version, PackageDefinition>>,
    catalog_entries: BTreeMap<PackageName, BTreeMap<semver::Version, CatalogEntry>>,
}

impl GitRegistry {
    /// Opens and validates a local Git registry repository at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let registry_path = path.as_ref();
        if !registry_path.is_dir() {
            return Err(RegistryError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "registry directory '{}' does not exist",
                    registry_path.display()
                ),
            )));
        }

        // Canonicalize the registry path to resolve symlinks and ensure safe boundary checks
        let canonical_root = fs::canonicalize(registry_path)?;

        // Verify it is a valid Git repository
        if !GitOps::is_git_repository(&canonical_root) {
            return Err(RegistryError::NotAGitRepository(canonical_root));
        }

        // Load and validate registry.toml
        let manifest_path = canonical_root.join(REGISTRY_MANIFEST_FILENAME);
        if !manifest_path.is_file() {
            return Err(RegistryError::MissingRegistryManifest(manifest_path));
        }

        let manifest_str = fs::read_to_string(&manifest_path)?;
        let manifest: RegistryManifest = toml::from_str(&manifest_str).map_err(|e| {
            RegistryError::Toml(format!(
                "failed to parse '{}': {e}",
                manifest_path.display()
            ))
        })?;

        if manifest.format_version != CURRENT_REGISTRY_FORMAT_VERSION {
            return Err(RegistryError::UnsupportedFormatVersion(
                manifest.format_version,
            ));
        }

        if manifest.name.trim().is_empty() {
            return Err(RegistryError::Validation(
                "registry name in registry.toml cannot be empty".to_string(),
            ));
        }

        let mut packages = BTreeMap::new();
        let mut catalog_entries = BTreeMap::new();

        let packages_dir = canonical_root.join("packages");
        if packages_dir.is_dir() {
            let mut pkg_entries = Vec::new();
            for entry in fs::read_dir(&packages_dir)? {
                pkg_entries.push(entry?);
            }
            // Deterministic lexicographical ordering
            pkg_entries.sort_by_key(|e| e.file_name());

            for pkg_entry in pkg_entries {
                let pkg_name_str = pkg_entry.file_name().to_string_lossy().to_string();
                if pkg_name_str.starts_with('.') {
                    continue;
                }

                let pkg_path = pkg_entry.path();
                check_path_safety(&pkg_path, &canonical_root)?;

                if !pkg_path.is_dir() {
                    return Err(RegistryError::Validation(format!(
                        "expected package directory in 'packages/', found file '{}'",
                        pkg_path.display()
                    )));
                }

                let package_name = PackageName::new(&pkg_name_str).map_err(|e| {
                    RegistryError::InvalidPackageName(format!(
                        "invalid package name '{pkg_name_str}': {e}"
                    ))
                })?;

                let mut ver_entries = Vec::new();
                for entry in fs::read_dir(&pkg_path)? {
                    ver_entries.push(entry?);
                }
                ver_entries.sort_by_key(|e| e.file_name());

                for ver_entry in ver_entries {
                    let ver_str = ver_entry.file_name().to_string_lossy().to_string();
                    if ver_str.starts_with('.') {
                        continue;
                    }

                    let ver_path = ver_entry.path();
                    check_path_safety(&ver_path, &canonical_root)?;

                    if !ver_path.is_dir() {
                        return Err(RegistryError::Validation(format!(
                            "expected version directory in '{}', found file '{}'",
                            pkg_path.display(),
                            ver_path.display()
                        )));
                    }

                    let version = semver::Version::parse(&ver_str).map_err(|e| {
                        RegistryError::InvalidVersion(format!(
                            "invalid semantic version directory '{ver_str}': {e}"
                        ))
                    })?;

                    let def_path = ver_path.join(PACKAGE_DEFINITION_FILENAME);
                    if !def_path.is_file() {
                        return Err(RegistryError::MissingPackageManifest(def_path));
                    }

                    check_path_safety(&def_path, &canonical_root)?;

                    let def_str = fs::read_to_string(&def_path)?;
                    let raw_def: RawPackageDefinition = toml::from_str(&def_str).map_err(|e| {
                        RegistryError::Toml(format!(
                            "failed to parse '{}': {e}",
                            def_path.display()
                        ))
                    })?;

                    // Identity consistency validation
                    if raw_def.package.name != pkg_name_str || raw_def.package.version != ver_str {
                        return Err(RegistryError::IdentityMismatch {
                            expected_name: pkg_name_str.clone(),
                            found_name: raw_def.package.name,
                            expected_version: ver_str.clone(),
                            found_version: raw_def.package.version,
                        });
                    }

                    let metadata = PackageMetadata {
                        name: package_name.clone(),
                        version: version.clone(),
                        description: raw_def.package.description,
                        license: raw_def.package.license,
                        homepage: raw_def.package.homepage,
                        repository: raw_def.package.repository,
                    };

                    let source_def = match raw_def.source.source_type.as_str() {
                        "archive" => {
                            let cs_str = raw_def.source.checksum.ok_or_else(|| {
                                RegistryError::Validation(format!(
                                    "package '{package_name} {version}' has source type 'archive' but lacks a required checksum"
                                ))
                            })?;
                            let checksum = Checksum::parse(&cs_str).map_err(|e| {
                                RegistryError::Validation(format!(
                                    "invalid checksum in '{package_name} {version}': {e}"
                                ))
                            })?;
                            PackageSourceDefinition::Archive {
                                url: raw_def.source.url,
                                checksum,
                            }
                        }
                        "git" => PackageSourceDefinition::Git {
                            url: raw_def.source.url,
                            rev: None,
                        },
                        other => {
                            return Err(RegistryError::Validation(format!(
                                "unsupported source type '{other}' in '{package_name} {version}'"
                            )));
                        }
                    };

                    let mut dependencies = Vec::new();
                    if let Some(deps_map) = raw_def.dependencies {
                        for (dep_name, dep_req) in deps_map {
                            let dep =
                                Dependency::try_new(&dep_name, &dep_req).map_err(|e| {
                                    RegistryError::Validation(format!(
                                        "invalid dependency '{dep_name}' in '{package_name} {version}': {e}"
                                    ))
                                })?;
                            dependencies.push(dep);
                        }
                        dependencies.sort();
                    }

                    let def = PackageDefinition {
                        metadata,
                        source: source_def.clone(),
                        build: raw_def.build,
                        dependencies: dependencies.clone(),
                    };

                    let pkg_id = PackageId::new(package_name.clone(), version.clone());

                    let versions_map = packages
                        .entry(package_name.clone())
                        .or_insert_with(BTreeMap::new);
                    if versions_map.contains_key(&version) {
                        return Err(RegistryError::DuplicatePackage(pkg_id));
                    }
                    versions_map.insert(version.clone(), def);

                    let mut entry = CatalogEntry::new(package_name.clone(), version.clone())
                        .with_source(PackageSource::registry(&manifest.name))
                        .with_dependencies(dependencies);
                    if let PackageSourceDefinition::Archive { ref checksum, .. } = source_def {
                        entry = entry.with_checksum(checksum.clone());
                    }

                    let catalog_map = catalog_entries
                        .entry(package_name.clone())
                        .or_insert_with(BTreeMap::new);
                    catalog_map.insert(version, entry);
                }
            }
        }

        Ok(Self {
            name: manifest.name.clone(),
            path: canonical_root,
            manifest,
            packages,
            catalog_entries,
        })
    }

    /// Returns the name of the registry.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the physical filesystem path to the registry root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the registry manifest.
    pub fn manifest(&self) -> &RegistryManifest {
        &self.manifest
    }

    /// Returns the total number of distinct packages in the registry.
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Returns the total number of distinct package versions in the registry.
    pub fn total_version_count(&self) -> usize {
        self.packages.values().map(|v| v.len()).sum()
    }

    /// Returns a sorted list of all package names in the registry.
    pub fn package_names(&self) -> Vec<PackageName> {
        self.packages.keys().cloned().collect()
    }

    /// Returns a specific package definition if present.
    pub fn get_package_definition(&self, id: &PackageId) -> Option<&PackageDefinition> {
        self.packages.get(id.name())?.get(id.version())
    }
}

impl PackageCatalog for GitRegistry {
    fn get_package_versions(
        &self,
        package: &PackageName,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        match self.catalog_entries.get(package) {
            Some(versions) => Ok(versions.values().cloned().collect()),
            None => Ok(Vec::new()),
        }
    }
}

/// Validates that `path` is not a symbolic link pointing outside `canonical_root`.
fn check_path_safety(path: &Path, canonical_root: &Path) -> Result<(), RegistryError> {
    let symlink_meta = fs::symlink_metadata(path)?;
    if symlink_meta.file_type().is_symlink() {
        let target = fs::canonicalize(path).map_err(|e| {
            RegistryError::SecurityViolation(format!(
                "invalid symbolic link '{}': {e}",
                path.display()
            ))
        })?;
        if !target.starts_with(canonical_root) {
            return Err(RegistryError::SecurityViolation(format!(
                "symbolic link '{}' resolves to '{}', which is outside registry root '{}'",
                path.display(),
                target.display(),
                canonical_root.display()
            )));
        }
    }
    Ok(())
}

/// Safe Git operations using direct executable invocation without shell interpolation.
pub struct GitOps;

impl GitOps {
    /// Checks whether the directory is inside a Git work tree.
    pub fn is_git_repository(path: &Path) -> bool {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("rev-parse")
            .arg("--is-inside-work-tree")
            .output();

        match output {
            Ok(out) => {
                out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
            }
            Err(_) => false,
        }
    }

    /// Clones a Git repository from `url` into `target_path`.
    pub fn clone_repository(url: &str, target_path: &Path) -> Result<(), RegistryError> {
        let output = Command::new("git")
            .arg("clone")
            .arg(url)
            .arg(target_path)
            .output()
            .map_err(|e| RegistryError::Git(format!("failed to execute git clone: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RegistryError::Git(format!(
                "failed to clone repository from '{url}': {stderr}"
            )));
        }

        Ok(())
    }

    /// Updates an existing repository by performing a fast-forward pull.
    pub fn update_repository(path: &Path) -> Result<String, RegistryError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("pull")
            .arg("--ff-only")
            .output()
            .map_err(|e| RegistryError::Git(format!("failed to execute git pull: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RegistryError::Git(format!(
                "failed to update registry repository at '{}': {stderr}",
                path.display()
            )));
        }

        Self::head_commit(path)
    }

    /// Returns the HEAD commit hash.
    pub fn head_commit(path: &Path) -> Result<String, RegistryError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .map_err(|e| RegistryError::Git(format!("failed to get HEAD commit: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RegistryError::Git(format!(
                "failed to query HEAD commit at '{}': {stderr}",
                path.display()
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Persistent registry configuration for Crux (`.crux/config.toml` or `~/.crux/config.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CruxConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<PathBuf>,
    #[serde(default)]
    pub registries: BTreeMap<String, RegistryConfig>,
}

/// Configuration entry for a single package registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub path: PathBuf,
}

impl CruxConfig {
    /// Returns the primary registry to use (defaults to "default", or the first configured one).
    pub fn get_primary_registry(&self) -> Option<(&str, &RegistryConfig)> {
        if let Some(cfg) = self.registries.get("default") {
            return Some(("default", cfg));
        }
        self.registries.iter().next().map(|(k, v)| (k.as_str(), v))
    }
}

/// Resolves the path to the configuration file.
pub fn config_file_path(project_dir: Option<&Path>) -> PathBuf {
    if let Ok(env_path) = std::env::var("CRUX_CONFIG_PATH") {
        return PathBuf::from(env_path);
    }

    if let Some(p) = project_dir {
        let local_config = p.join(".crux").join("config.toml");
        if local_config.is_file() || p.join(crate::manifest::MANIFEST_FILENAME).is_file() {
            return local_config;
        }
    }

    if let Ok(crux_home) = std::env::var("CRUX_HOME") {
        return PathBuf::from(crux_home).join("config.toml");
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".crux").join("config.toml");
    }

    PathBuf::from(".crux").join("config.toml")
}

/// Resolves the base directory where cloned registries are stored.
pub fn registries_dir_path(project_dir: Option<&Path>) -> PathBuf {
    if let Ok(env_dir) = std::env::var("CRUX_REGISTRIES_DIR") {
        return PathBuf::from(env_dir);
    }

    if let Ok(crux_home) = std::env::var("CRUX_HOME") {
        return PathBuf::from(crux_home).join("registries");
    }

    if let Some(p) = project_dir {
        let manifest_candidate = p.join(crate::manifest::MANIFEST_FILENAME);
        if manifest_candidate.is_file() {
            return p.join(".crux").join("registries");
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".crux").join("registries");
    }

    PathBuf::from(".crux").join("registries")
}

/// Loads Crux configuration from disk. Returns default empty config if file doesn't exist.
pub fn load_config(project_dir: Option<&Path>) -> Result<CruxConfig, RegistryError> {
    let cfg_path = config_file_path(project_dir);
    if !cfg_path.is_file() {
        return Ok(CruxConfig::default());
    }

    let content = fs::read_to_string(&cfg_path)?;
    let config: CruxConfig = toml::from_str(&content).map_err(|e| {
        RegistryError::Toml(format!(
            "failed to parse configuration file '{}': {e}",
            cfg_path.display()
        ))
    })?;

    Ok(config)
}

/// Atomically saves Crux configuration to disk.
pub fn save_config(config: &CruxConfig, path: &Path) -> Result<(), RegistryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let toml_str = toml::to_string_pretty(config)
        .map_err(|e| RegistryError::Toml(format!("failed to serialize configuration: {e}")))?;

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let temp_name = format!(".config.tmp.{pid}.{nanos}");
    let temp_path = parent.join(temp_name);

    {
        use std::io::Write;
        let mut f = fs::File::create(&temp_path)?;
        f.write_all(toml_str.as_bytes())?;
        f.flush()?;
        f.sync_data()?;
    }

    fs::rename(&temp_path, path)?;
    Ok(())
}

/// Adds and clones a new registry.
pub fn add_registry(
    name: &str,
    git_url: &str,
    project_dir: Option<&Path>,
) -> Result<GitRegistry, RegistryError> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(RegistryError::Validation(
            "registry name cannot be empty".to_string(),
        ));
    }
    if !trimmed_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(RegistryError::Validation(format!(
            "invalid registry name '{trimmed_name}': must contain only alphanumeric characters, '-', or '_'"
        )));
    }

    let mut config = load_config(project_dir)?;
    if config.registries.contains_key(trimmed_name) {
        return Err(RegistryError::AlreadyExists(trimmed_name.to_string()));
    }

    let target_dir = registries_dir_path(project_dir).join(trimmed_name);
    if target_dir.exists() {
        if !GitOps::is_git_repository(&target_dir) {
            return Err(RegistryError::Validation(format!(
                "target registry directory '{}' exists but is not a valid Git repository",
                target_dir.display()
            )));
        }
    } else {
        if let Some(parent) = target_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        GitOps::clone_repository(git_url, &target_dir)?;
    }

    // Validate the cloned repository by opening it
    let registry = match GitRegistry::open(&target_dir) {
        Ok(reg) => reg,
        Err(err) => {
            let _ = fs::remove_dir_all(&target_dir);
            return Err(err);
        }
    };

    // Save to configuration
    config.registries.insert(
        trimmed_name.to_string(),
        RegistryConfig {
            url: Some(git_url.to_string()),
            path: target_dir,
        },
    );
    save_config(&config, &config_file_path(project_dir))?;

    Ok(registry)
}

/// Status report for a configured package registry.
#[derive(Debug)]
pub struct RegistryStatus {
    pub name: String,
    pub config: RegistryConfig,
    pub registry: Result<GitRegistry, RegistryError>,
}

/// Lists all configured registries.
pub fn list_registries(project_dir: Option<&Path>) -> Result<Vec<RegistryStatus>, RegistryError> {
    let config = load_config(project_dir)?;
    let mut list = Vec::new();
    for (name, reg_cfg) in config.registries {
        let reg_result = GitRegistry::open(&reg_cfg.path);
        list.push(RegistryStatus {
            name,
            config: reg_cfg,
            registry: reg_result,
        });
    }
    Ok(list)
}

/// Updates an existing registry from upstream.
pub fn update_registry(
    name: &str,
    project_dir: Option<&Path>,
) -> Result<(GitRegistry, String), RegistryError> {
    let config = load_config(project_dir)?;
    let reg_cfg = config
        .registries
        .get(name)
        .ok_or_else(|| RegistryError::NotFound(name.to_string()))?;

    let head = GitOps::update_repository(&reg_cfg.path)?;
    let registry = GitRegistry::open(&reg_cfg.path)?;

    Ok((registry, head))
}

/// Seeds a directory with verified initial package definitions (fmt, zlib, catch2) and registry.toml.
pub fn seed_registry_files(dir: &Path) -> Result<(), io::Error> {
    fs::create_dir_all(dir)?;

    fs::write(
        dir.join(REGISTRY_MANIFEST_FILENAME),
        "format_version = 1\nname = \"default\"\ndescription = \"Official Crux package registry\"\n",
    )?;

    let pkgs = [
        (
            "fmt",
            "11.0.2",
            r#"[package]
name = "fmt"
version = "11.0.2"
description = "A modern formatting library"
license = "MIT"
homepage = "https://fmt.dev"
repository = "https://github.com/fmtlib/fmt"

[source]
type = "archive"
url = "https://github.com/fmtlib/fmt/releases/download/11.0.2/fmt-11.0.2.zip"
checksum = "sha256:40fc58bebcf38c759e11a7bd8fdc163507d2423ef5058bba7f26280c5b9c5465"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "fmt",
            "11.1.1",
            r#"[package]
name = "fmt"
version = "11.1.1"
description = "A modern formatting library"
license = "MIT"
homepage = "https://fmt.dev"
repository = "https://github.com/fmtlib/fmt"

[source]
type = "archive"
url = "https://github.com/fmtlib/fmt/releases/download/11.1.1/fmt-11.1.1.zip"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "zlib",
            "1.3.0",
            r#"[package]
name = "zlib"
version = "1.3.0"
description = "A Massively Spiffy Yet Delicately Unobtrusive Compression Library"
license = "Zlib"
homepage = "https://zlib.net"
repository = "https://github.com/madler/zlib"

[source]
type = "archive"
url = "https://github.com/madler/zlib/releases/download/v1.3/zlib-1.3.tar.gz"
checksum = "sha256:ff0ba4c292013dbc27530b3a81e1f9a813cd39de01ca5e0f8bf355702efa593e"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "zlib",
            "1.3.1",
            r#"[package]
name = "zlib"
version = "1.3.1"
description = "A Massively Spiffy Yet Delicately Unobtrusive Compression Library"
license = "Zlib"
homepage = "https://zlib.net"
repository = "https://github.com/madler/zlib"

[source]
type = "archive"
url = "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz"
checksum = "sha256:9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "catch2",
            "3.7.1",
            r#"[package]
name = "catch2"
version = "3.7.1"
description = "A modern, C++-native, test framework for unit-tests, TDD and BDD"
license = "BSL-1.0"
homepage = "https://github.com/catchorg/Catch2"
repository = "https://github.com/catchorg/Catch2"

[source]
type = "archive"
url = "https://github.com/catchorg/Catch2/archive/refs/tags/v3.7.1.tar.gz"
checksum = "sha256:1542950f7d0376938841218a41cb5b1fc138f6262603e07d0062226012fbd577"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "catch2",
            "3.8.0",
            r#"[package]
name = "catch2"
version = "3.8.0"
description = "A modern, C++-native, test framework for unit-tests, TDD and BDD"
license = "BSL-1.0"
homepage = "https://github.com/catchorg/Catch2"
repository = "https://github.com/catchorg/Catch2"

[source]
type = "archive"
url = "https://github.com/catchorg/Catch2/archive/refs/tags/v3.8.0.tar.gz"
checksum = "sha256:1ab2de20460d4641553addfdfe6acd4109d871d5531f8f519a52ea4926303087"

[build]
system = "cmake"

[dependencies]
"#,
        ),
    ];

    for (name, ver, content) in pkgs {
        let pkg_dir = dir.join("packages").join(name).join(ver);
        fs::create_dir_all(&pkg_dir)?;
        fs::write(pkg_dir.join(PACKAGE_DEFINITION_FILENAME), content)?;
    }

    Ok(())
}

/// Initializes a directory as a Git repository and commits the seeded registry definitions.
pub fn init_test_git_registry(dir: &Path) -> Result<GitRegistry, RegistryError> {
    seed_registry_files(dir)?;

    let run_git = |args: &[&str]| -> Result<(), RegistryError> {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(|e| RegistryError::Git(format!("git command failed: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(RegistryError::Git(format!(
                "git {} failed: {err}",
                args.join(" ")
            )));
        }
        Ok(())
    };

    run_git(&["init", "-b", "main"])?;
    run_git(&["config", "user.name", "Crux Test"])?;
    run_git(&["config", "user.email", "test@crux.invalid"])?;
    run_git(&["add", "."])?;
    run_git(&["commit", "-m", "Initial test registry commit"])?;

    GitRegistry::open(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_seed_registry() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("registry");
        let reg = GitRegistry::open(&root).expect("failed to open root seed registry");

        assert_eq!(reg.name(), "default");
        assert_eq!(reg.package_count(), 3);
        assert_eq!(reg.total_version_count(), 6);

        let names = reg.package_names();
        let name_strs: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
        assert_eq!(name_strs, vec!["catch2", "fmt", "zlib"]);

        let fmt_versions = reg
            .get_package_versions(&PackageName::new("fmt").unwrap())
            .unwrap();
        assert_eq!(fmt_versions.len(), 2);
        assert_eq!(fmt_versions[0].version.to_string(), "11.0.2");
        assert_eq!(fmt_versions[1].version.to_string(), "11.1.1");
    }

    #[test]
    fn test_not_a_git_repo_fails() {
        let temp = std::env::temp_dir().join(format!("crux_non_git_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        fs::write(
            temp.join("registry.toml"),
            "format_version = 1\nname = \"test\"\n",
        )
        .unwrap();

        let res = GitRegistry::open(&temp);
        assert!(matches!(res, Err(RegistryError::NotAGitRepository(_))));

        let _ = fs::remove_dir_all(&temp);
    }
}
