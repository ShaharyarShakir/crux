//! Dependency linking model for application builds.
//!
//! Application builds link against packages that Crux previously resolved,
//! acquired, built, and installed under `.crux/installed/`. This module turns
//! the validated on-disk installation inventory into a deterministic set of
//! include directories, link directories, and library files that the generated
//! internal CMake project can consume directly.
//!
//! Design constraints:
//!
//! * Only validated installations are used; an incomplete or corrupt install
//!   produces an actionable error instead of silently linking stale files.
//! * Transitive dependencies are included and ordered so that dependents come
//!   before the libraries they depend on (required by single-pass linkers).
//! * Static libraries are preferred when a package installs both static and
//!   shared variants, matching source-first, self-contained link semantics.
//! * Shared library directories are exposed as runtime directories so the
//!   generated build tree records a usable RPATH.

use super::BuildError;
use crate::dependency::Dependency;
use crate::install::{InstalledFile, InstalledFileType};
use crate::integrate::validation::{ValidatedPackageInstallation, validate_package_installation};
use crate::package::PackageName;
use crate::resolution::lockfile::Lockfile;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A single dependency linked into an application build.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AppDependency {
    /// Package name.
    pub name: String,
    /// Resolved package version.
    pub version: semver::Version,
    /// Validated installation fingerprint (directory name).
    pub fingerprint: String,
    /// Absolute installation directory.
    pub install_dir: PathBuf,
    /// Deterministically ordered include roots.
    pub include_dirs: Vec<PathBuf>,
    /// Deterministically ordered library search directories.
    pub link_directories: Vec<PathBuf>,
    /// Absolute library files to place on the link line.
    pub link_libraries: Vec<PathBuf>,
    /// Directories containing shared libraries needed at runtime.
    pub runtime_directories: Vec<PathBuf>,
}

/// The complete, deterministically ordered dependency link plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppDependencyPlan {
    /// Dependencies ordered dependents-before-dependencies.
    pub dependencies: Vec<AppDependency>,
}

impl AppDependencyPlan {
    /// Returns true when the application declares no dependencies.
    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
    }

    /// Returns the number of linked dependencies (including transitives).
    pub fn len(&self) -> usize {
        self.dependencies.len()
    }

    /// Collects all include directories in link order, de-duplicated.
    pub fn include_dirs(&self) -> Vec<PathBuf> {
        dedup_in_order(self.dependencies.iter().flat_map(|d| d.include_dirs.iter()))
    }

    /// Collects all link directories in link order, de-duplicated.
    pub fn link_directories(&self) -> Vec<PathBuf> {
        dedup_in_order(
            self.dependencies
                .iter()
                .flat_map(|d| d.link_directories.iter()),
        )
    }

    /// Collects all library files in link order (duplicates preserved).
    pub fn link_libraries(&self) -> Vec<PathBuf> {
        self.dependencies
            .iter()
            .flat_map(|d| d.link_libraries.iter().cloned())
            .collect()
    }

    /// Collects all runtime shared-library directories, de-duplicated.
    pub fn runtime_directories(&self) -> Vec<PathBuf> {
        dedup_in_order(
            self.dependencies
                .iter()
                .flat_map(|d| d.runtime_directories.iter()),
        )
    }
}

fn dedup_in_order<'a>(items: impl Iterator<Item = &'a PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item.clone());
        }
    }
    out
}

/// Builds the dependency link plan for an application.
///
/// `direct` are the application's declared direct dependencies. The lockfile
/// must contain every direct dependency and every transitive dependency.
pub fn plan_app_dependencies(
    project_root: &Path,
    lockfile: &Lockfile,
    direct: &[Dependency],
    profile: super::metadata::BuildProfile,
) -> Result<AppDependencyPlan, BuildError> {
    if direct.is_empty() {
        return Ok(AppDependencyPlan::default());
    }

    let mut locked_by_name = BTreeMap::new();
    for pkg in &lockfile.packages {
        locked_by_name.insert(pkg.name.as_str(), pkg);
    }

    // Deterministic depth-first traversal. Pushing a package before visiting
    // its dependencies yields a dependents-before-dependencies link order.
    let mut ordered_names = Vec::new();
    let mut visited = BTreeSet::new();
    let mut stack = Vec::new();
    for dep in direct.iter().rev() {
        stack.push(dep.name().as_str().to_string());
    }

    while let Some(name) = stack.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }

        let Some(locked) = locked_by_name.get(name.as_str()) else {
            let requirement = direct
                .iter()
                .find(|d| d.name().as_str() == name)
                .map(|d| d.version_req().to_string())
                .unwrap_or_else(|| "*".to_string());
            return Err(BuildError::MissingDependency {
                name: name.clone(),
                version: requirement,
                profile: profile.to_string(),
                reason: "the dependency is declared in crux.toml but missing from crux.lock; \
                     run 'crux lock' to update the lockfile"
                    .to_string(),
            });
        };

        ordered_names.push(name.clone());

        // Locked dependency lists are already sorted deterministically.
        for dep in locked.dependencies.iter().rev() {
            stack.push(dep.name().as_str().to_string());
        }
    }

    let mut dependencies = Vec::with_capacity(ordered_names.len());
    for name in ordered_names {
        let locked = locked_by_name
            .get(name.as_str())
            .expect("locked package presence validated above");

        let validated = validate_package_installation(
            project_root,
            locked.name.as_str(),
            &locked.version.to_string(),
            profile,
        )
        .map_err(|err| BuildError::MissingDependency {
            name: locked.name.as_str().to_string(),
            version: locked.version.to_string(),
            profile: profile.to_string(),
            reason: format!(
                "{err}\n  run 'crux install --profile {}' to install it",
                profile.as_str()
            ),
        })?;

        dependencies.push(build_link_entry(validated)?);
    }

    Ok(AppDependencyPlan { dependencies })
}

fn build_link_entry(validated: ValidatedPackageInstallation) -> Result<AppDependency, BuildError> {
    let install_dir = validated.install_dir.clone();
    let files = &validated.metadata.installed_files;

    let include_dirs = derive_include_dirs(&install_dir, files);
    let (link_libraries, runtime_directories) = derive_link_libraries(&install_dir, files);

    let mut link_directories = BTreeSet::new();
    for lib in &link_libraries {
        if let Some(parent) = lib.parent() {
            link_directories.insert(parent.to_path_buf());
        }
    }

    let package_name = validated.package_id.name();
    if PackageName::new(package_name.as_str()).is_err() {
        return Err(BuildError::SecurityViolation(format!(
            "validated installation contains an invalid package name '{package_name}'"
        )));
    }

    Ok(AppDependency {
        name: package_name.as_str().to_string(),
        version: validated.package_id.version().clone(),
        fingerprint: validated.fingerprint.hex_digest().to_string(),
        install_dir,
        include_dirs,
        link_directories: link_directories.into_iter().collect(),
        link_libraries,
        runtime_directories,
    })
}

/// Derives include roots from the installed inventory.
///
/// A header under an `include/` component maps to the directory ending at that
/// `include/` component (`prefix/include`). Any other layout falls back to the
/// installation root so the header remains reachable with a relative include.
fn derive_include_dirs(install_dir: &Path, files: &[InstalledFile]) -> Vec<PathBuf> {
    let mut roots = BTreeSet::new();

    for file in files {
        if file.file_type != InstalledFileType::Header {
            continue;
        }

        let mut accumulator = PathBuf::new();
        let mut found_include_root = None;
        for component in file.relative_path.components() {
            accumulator.push(component);
            if component.as_os_str() == "include" {
                found_include_root = Some(install_dir.join(&accumulator));
                break;
            }
        }

        roots.insert(found_include_root.unwrap_or_else(|| install_dir.to_path_buf()));
    }

    roots.into_iter().collect()
}

/// Derives the link libraries and runtime directories from the installed inventory.
///
/// Static libraries are preferred when a package installs both static and shared
/// variants. Shared library parents are returned as runtime directories so the
/// generated build tree can record an RPATH.
fn derive_link_libraries(
    install_dir: &Path,
    files: &[InstalledFile],
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut static_libs = Vec::new();
    let mut shared_libs = Vec::new();

    for file in files {
        match file.file_type {
            InstalledFileType::StaticLibrary => {
                static_libs.push(install_dir.join(&file.relative_path));
            }
            InstalledFileType::SharedLibrary => {
                shared_libs.push(install_dir.join(&file.relative_path));
            }
            _ => {}
        }
    }

    static_libs.sort();
    shared_libs.sort();

    if !static_libs.is_empty() {
        return (static_libs, Vec::new());
    }

    let mut runtime_dirs = BTreeSet::new();
    for lib in &shared_libs {
        if let Some(parent) = lib.parent() {
            runtime_dirs.insert(parent.to_path_buf());
        }
    }

    (shared_libs, runtime_dirs.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::BuildFingerprint;
    use crate::build::metadata::BuildProfile;
    use crate::dependency::Dependency;
    use crate::install::{CURRENT_INSTALL_METADATA_VERSION, InstallMetadata};
    use crate::lock::{LockedPackage, Lockfile, PackageSource};
    use crate::package::PackageId;
    use std::fs;

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "crux_build_dependency_{prefix}_{}_{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_install(
        project_root: &Path,
        name: &str,
        version: &str,
        profile: BuildProfile,
        files: &[(&str, InstalledFileType)],
        seed: u8,
    ) -> BuildFingerprint {
        let fingerprint = BuildFingerprint::new(format!("sha256:{seed:064x}")).unwrap();
        let install_dir = project_root
            .join(".crux/installed")
            .join(name)
            .join(version)
            .join(fingerprint.hex_digest());
        fs::create_dir_all(&install_dir).unwrap();

        let mut installed_files = Vec::new();
        for (relative, file_type) in files {
            let path = install_dir.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, b"content").unwrap();
            installed_files.push(InstalledFile {
                relative_path: PathBuf::from(relative),
                file_type: *file_type,
                size_bytes: 7,
            });
        }

        let metadata = InstallMetadata {
            format_version: CURRENT_INSTALL_METADATA_VERSION,
            package_name: name.to_string(),
            package_version: version.to_string(),
            fingerprint: fingerprint.clone(),
            profile,
            installed_files,
            total_size_bytes: 7,
            success: true,
            installed_at: None,
        };
        fs::write(
            install_dir.join(crate::install::INSTALL_METADATA_FILENAME),
            toml::to_string_pretty(&metadata).unwrap(),
        )
        .unwrap();
        fingerprint
    }

    fn locked(name: &str, version: &str, dependencies: &[&str]) -> LockedPackage {
        let name = PackageName::new(name).unwrap();
        let version = semver::Version::parse(version).unwrap();
        let mut pkg = LockedPackage::new(name.clone(), version, PackageSource::registry("default"));
        pkg.dependencies = dependencies
            .iter()
            .map(|dep| {
                PackageId::new(
                    PackageName::new(*dep).unwrap(),
                    semver::Version::parse("1.0.0").unwrap(),
                )
            })
            .collect();
        pkg
    }

    #[test]
    fn test_empty_direct_dependencies_produce_empty_plan() {
        let root = temp_dir("empty");
        let lockfile = Lockfile::new();
        let plan = plan_app_dependencies(&root, &lockfile, &[], BuildProfile::Debug).unwrap();
        assert!(plan.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn test_missing_lockfile_entry_is_actionable() {
        let root = temp_dir("missing");
        let lockfile = Lockfile::new();
        let deps = vec![Dependency::try_new("fmt", "^11").unwrap()];
        let err = plan_app_dependencies(&root, &lockfile, &deps, BuildProfile::Debug).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("crux.lock"), "{message}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn test_plan_orders_dependents_before_dependencies_and_prefers_static() {
        let root = temp_dir("order");
        write_install(
            &root,
            "appdep",
            "1.0.0",
            BuildProfile::Debug,
            &[
                ("include/appdep/appdep.h", InstalledFileType::Header),
                ("lib/libappdep.a", InstalledFileType::StaticLibrary),
                ("lib/libappdep.so", InstalledFileType::SharedLibrary),
            ],
            1,
        );
        write_install(
            &root,
            "leaf",
            "1.0.0",
            BuildProfile::Debug,
            &[
                ("include/leaf/leaf.h", InstalledFileType::Header),
                ("lib/libleaf.a", InstalledFileType::StaticLibrary),
            ],
            2,
        );

        let mut lockfile = Lockfile::new();
        lockfile.packages = vec![
            locked("appdep", "1.0.0", &["leaf"]),
            locked("leaf", "1.0.0", &[]),
        ];
        lockfile.packages.sort();
        let deps = vec![Dependency::try_new("appdep", "^1").unwrap()];

        let plan = plan_app_dependencies(&root, &lockfile, &deps, BuildProfile::Debug).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan.dependencies[0].name, "appdep");
        assert_eq!(plan.dependencies[1].name, "leaf");
        assert!(
            plan.dependencies[0]
                .include_dirs
                .iter()
                .any(|p| { p.ends_with("include") && p.to_string_lossy().contains("/appdep/") }),
            "unexpected include dirs: {:?}",
            plan.dependencies[0].include_dirs
        );
        assert_eq!(plan.dependencies[0].link_libraries.len(), 1);
        assert!(
            plan.dependencies[0].link_libraries[0]
                .to_string_lossy()
                .ends_with("libappdep.a")
        );
        assert!(plan.dependencies[0].runtime_directories.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn test_shared_only_package_populates_runtime_directories() {
        let root = temp_dir("shared");
        write_install(
            &root,
            "shared",
            "1.0.0",
            BuildProfile::Release,
            &[
                ("include/shared/shared.h", InstalledFileType::Header),
                ("lib/libshared.so", InstalledFileType::SharedLibrary),
            ],
            3,
        );
        let mut lockfile = Lockfile::new();
        lockfile.packages = vec![locked("shared", "1.0.0", &[])];
        let deps = vec![Dependency::try_new("shared", "1").unwrap()];
        let plan = plan_app_dependencies(&root, &lockfile, &deps, BuildProfile::Release).unwrap();
        assert_eq!(plan.dependencies[0].link_libraries.len(), 1);
        assert_eq!(plan.dependencies[0].runtime_directories.len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
