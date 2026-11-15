//! Git repository inspection and safe execution operations.

use super::RegistryError;
use super::metadata::{
    CURRENT_REGISTRY_FORMAT_VERSION, PACKAGE_DEFINITION_FILENAME, PackageDefinition,
    PackageSourceDefinition, REGISTRY_MANIFEST_FILENAME, RawPackageDefinition, RegistryManifest,
};
use crate::catalog::{CatalogEntry, CatalogError, PackageCatalog};
use crate::dependency::Dependency;
use crate::lock::{Checksum, PackageSource};
use crate::package::{PackageId, PackageMetadata, PackageName};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

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
pub(crate) fn check_path_safety(path: &Path, canonical_root: &Path) -> Result<(), RegistryError> {
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
