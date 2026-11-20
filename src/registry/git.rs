//! Git repository inspection and safe execution operations.

use super::RegistryError;
use super::metadata::{
    CURRENT_REGISTRY_FORMAT_VERSION, PACKAGE_DEFINITION_FILENAME, PackageDefinition,
    PackageSourceDefinition, REGISTRY_MANIFEST_FILENAME, RegistryManifest,
    parse_package_definition,
};
use crate::catalog::{CatalogEntry, CatalogError, PackageCatalog};
use crate::lock::PackageSource;
use crate::package::{PackageId, PackageName};
use crate::resolution::GitRef;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    /// Opens a registry described by a [`RegistryConfig`], honoring an optional
    /// registry subdirectory inside the configured Git checkout.
    pub fn open_configured(config: &super::config::RegistryConfig) -> Result<Self, RegistryError> {
        config.validate()?;
        Self::open_with_subdir(&config.path, config.subdir.as_deref())
    }

    /// Opens a registry stored in `path`, optionally below a relative subdirectory.
    ///
    /// The Git repository check is performed against the resolved registry root,
    /// which is also inside the enclosing Git work tree for monorepo layouts such
    /// as the official Crux repository (`<repo>/registry`).
    pub fn open_with_subdir(
        path: impl AsRef<Path>,
        subdir: Option<&Path>,
    ) -> Result<Self, RegistryError> {
        let base = path.as_ref();
        let registry_root = match subdir {
            Some(subdir) => {
                if subdir.as_os_str().is_empty() {
                    return Err(RegistryError::Validation(
                        "registry subdirectory cannot be empty".to_string(),
                    ));
                }
                for component in subdir.components() {
                    match component {
                        std::path::Component::Normal(_) | std::path::Component::CurDir => {}
                        other => {
                            return Err(RegistryError::SecurityViolation(format!(
                                "registry subdirectory '{}' must be relative and cannot contain root, prefix, or parent components (found {other:?})",
                                subdir.display()
                            )));
                        }
                    }
                }
                base.join(subdir)
            }
            None => base.to_path_buf(),
        };

        Self::open(registry_root)
    }

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
                    let def = parse_package_definition(&def_str, &def_path, None)?;

                    // Identity consistency validation
                    if def.metadata.name.as_str() != pkg_name_str
                        || def.metadata.version.to_string() != ver_str
                    {
                        return Err(RegistryError::IdentityMismatch {
                            expected_name: pkg_name_str.clone(),
                            found_name: def.metadata.name.as_str().to_string(),
                            expected_version: ver_str.clone(),
                            found_version: def.metadata.version.to_string(),
                        });
                    }

                    let source_def = def.source.clone();
                    let dependencies = def.dependencies.clone();

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

    /// Clones a repository, resolves the requested ref to a full commit, and checks it out detached.
    ///
    /// Git is invoked with explicit arguments, a null stdin, and a hard timeout.
    /// URL and ref values are validated before reaching this function.
    pub fn clone_and_pin_commit(
        url: &str,
        target_path: &Path,
        requested: &GitRef,
        timeout: Duration,
    ) -> Result<String, RegistryError> {
        Self::run_git(
            timeout,
            &[
                "clone",
                "--no-checkout",
                "--",
                url,
                target_path.to_str().unwrap_or(""),
            ],
        )?;

        let expression = match requested {
            GitRef::Head => "HEAD^{commit}".to_string(),
            GitRef::Tag(value) | GitRef::Branch(value) | GitRef::Rev(value) => {
                format!("{value}^{{commit}}")
            }
            GitRef::Commit(value) => format!("{value}^{{commit}}"),
        };

        let commit = Self::run_git(
            timeout,
            &[
                "-C",
                target_path.to_str().unwrap_or(""),
                "rev-parse",
                "--verify",
                &expression,
            ],
        )?
        .trim()
        .to_string();

        if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(RegistryError::Git(format!(
                "Git ref '{requested:?}' resolved to invalid commit '{commit}'"
            )));
        }

        Self::run_git(
            timeout,
            &[
                "-C",
                target_path.to_str().unwrap_or(""),
                "checkout",
                "--detach",
                &commit,
            ],
        )?;

        Ok(commit.to_ascii_lowercase())
    }

    /// Verifies that a full commit exists in an already cloned repository.
    pub fn verify_commit(
        path: &Path,
        commit: &str,
        timeout: Duration,
    ) -> Result<(), RegistryError> {
        let expression = format!("{commit}^{{commit}}");
        Self::run_git(
            timeout,
            &[
                "-C",
                path.to_str().unwrap_or(""),
                "rev-parse",
                "--verify",
                &expression,
            ],
        )
        .map(|_| ())
    }

    fn run_git(timeout: Duration, args: &[&str]) -> Result<String, RegistryError> {
        let mut child = Command::new("git")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| RegistryError::Git(format!("failed to execute git: {e}")))?;

        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(RegistryError::Git(format!(
                            "git command timed out after {} seconds",
                            timeout.as_secs()
                        )));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(err) => {
                    return Err(RegistryError::Git(format!(
                        "failed to wait for git command: {err}"
                    )));
                }
            }
        }

        let output = child.wait_with_output().map_err(|e| {
            RegistryError::Git(format!("failed to collect git command output: {e}"))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RegistryError::Git(format!(
                "git {} failed: {stderr}",
                args.first().copied().unwrap_or("command")
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
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
