//! Crux dependencies of a package build.
//!
//! A package build sees the installations of its locked dependencies, direct
//! and transitive, and its fingerprint covers exactly that set. Everything here
//! comes from validated installations and their `installed.toml` inventories;
//! no directory is guessed or scanned, and no installed `.cmake` or `.pc` file
//! is parsed.
//!
//! Application linking is separate: see [`super::dependency`].

use super::BuildError;
use super::metadata::BuildProfile;
use super::selection::{SEARCH_PATH_VARIABLES, ToolchainSelection};
use crate::install::InstalledFile;
use crate::integrate::validation::validate_package_installation;
use crate::lock::LockedPackage;
use crate::package::PackageId;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One installed dependency of a package build.
#[derive(Debug, Clone)]
struct PackageDependency {
    version: String,
    /// `sha256:<hex>` of the installation selected as current.
    fingerprint: String,
    prefix: PathBuf,
    /// Directories holding `Find*.cmake` modules, or modules under `Modules/`.
    module_dirs: BTreeSet<PathBuf>,
    /// Directories holding `.pc` files.
    pkg_config_dirs: BTreeSet<PathBuf>,
}

/// The dependency closure of one package, ordered by package name.
#[derive(Debug, Clone, Default)]
pub(crate) struct BuildDependencies(BTreeMap<String, PackageDependency>);

/// The locked packages `root` depends on, directly or transitively.
fn locked_closure(
    packages: &[LockedPackage],
    root: &LockedPackage,
    profile: BuildProfile,
) -> Result<Vec<PackageId>, BuildError> {
    let mut closure = BTreeMap::new();
    let mut pending = root.dependencies.clone();
    while let Some(id) = pending.pop() {
        if *id.name() == root.name || closure.contains_key(id.name()) {
            continue;
        }
        let Some(locked) = packages.iter().find(|p| p.name == *id.name()) else {
            return Err(BuildError::MissingDependency {
                name: id.name().to_string(),
                version: id.version().to_string(),
                profile: profile.to_string(),
                reason: "it is not listed in crux.lock; run 'crux lock'".to_string(),
            });
        };
        closure.insert(locked.name.clone(), locked.id());
        pending.extend(locked.dependencies.iter().cloned());
    }
    Ok(closure.into_values().collect())
}

impl BuildDependencies {
    /// The installed dependencies of `root` according to the locked `packages`.
    pub(crate) fn for_locked(
        project_dir: &Path,
        packages: &[LockedPackage],
        root: &LockedPackage,
        profile: BuildProfile,
    ) -> Result<Self, BuildError> {
        let closure = locked_closure(packages, root, profile)?;
        Self::resolve(project_dir, &closure, profile)
    }

    /// Reads the current installation of every package in `closure`, which must
    /// hold the direct and transitive dependencies. Fails when any of them has
    /// no usable installation for `profile`.
    pub(crate) fn resolve<'a>(
        project_dir: &Path,
        closure: impl IntoIterator<Item = &'a PackageId>,
        profile: BuildProfile,
    ) -> Result<Self, BuildError> {
        let mut dependencies = Self::default();
        for id in closure {
            let name = id.name().as_str();
            let version = id.version().to_string();
            let installation = validate_package_installation(project_dir, name, &version, profile)
                .map_err(|err| BuildError::MissingDependency {
                    name: name.to_string(),
                    version: version.clone(),
                    profile: profile.to_string(),
                    reason: err.to_string(),
                })?;
            // CMake and pkg-config resolve relative entries elsewhere.
            let prefix = std::path::absolute(&installation.install_dir)?;
            dependencies.insert(
                name,
                &version,
                installation.fingerprint.as_str(),
                prefix,
                &installation.metadata.installed_files,
            );
        }
        Ok(dependencies)
    }

    fn insert(
        &mut self,
        name: &str,
        version: &str,
        fingerprint: &str,
        prefix: PathBuf,
        files: &[InstalledFile],
    ) {
        let mut module_dirs = BTreeSet::new();
        let mut pkg_config_dirs = BTreeSet::new();
        for file in files {
            let relative = &file.relative_path;
            let (Some(parent), Some(file_name)) = (relative.parent(), relative.file_name()) else {
                continue;
            };
            let file_name = file_name.to_string_lossy();
            if file_name.ends_with(".pc") {
                pkg_config_dirs.insert(prefix.join(parent));
            } else if file_name.ends_with(".cmake")
                && (file_name.starts_with("Find") || parent.ends_with("Modules"))
            {
                module_dirs.insert(prefix.join(parent));
            }
        }
        self.0.insert(
            name.to_string(),
            PackageDependency {
                version: version.to_string(),
                fingerprint: fingerprint.to_string(),
                prefix,
                module_dirs,
                pkg_config_dirs,
            },
        );
    }

    /// Inputs for the package fingerprint: identity and selected installation
    /// only. No paths, so the digest does not depend on where the project is.
    pub(crate) fn fingerprint_fields(&self) -> Vec<(&'static str, &[u8])> {
        self.0
            .iter()
            .flat_map(|(name, dependency)| {
                [
                    ("name", name.as_bytes()),
                    ("version", dependency.version.as_bytes()),
                    ("fingerprint", dependency.fingerprint.as_bytes()),
                ]
            })
            .collect()
    }

    /// What an installation records as having been built against.
    pub(crate) fn record(&self) -> BTreeMap<String, String> {
        self.0
            .iter()
            .map(|(name, d)| (name.clone(), format!("{}@{}", d.version, d.fingerprint)))
            .collect()
    }

    /// Directories holding CMake modules, in dependency name order.
    pub(crate) fn module_dirs(&self) -> impl Iterator<Item = &Path> {
        self.0
            .values()
            .flat_map(|d| &d.module_dirs)
            .map(PathBuf::as_path)
    }

    /// Adds the dependency search paths to a build command. Crux entries go
    /// first, followed by the values `selection` captured and fingerprinted.
    /// Must run after `extra_env` has been applied to `command`.
    pub(crate) fn apply_environment(
        &self,
        command: &mut Command,
        selection: &ToolchainSelection,
    ) -> Result<(), BuildError> {
        let prefixes = self.0.values().map(|d| &d.prefix).collect::<Vec<_>>();
        let pkg_config = self
            .0
            .values()
            .flat_map(|d| &d.pkg_config_dirs)
            .collect::<Vec<_>>();
        for (key, ours) in SEARCH_PATH_VARIABLES
            .into_iter()
            .zip([prefixes, pkg_config])
        {
            if ours.is_empty() {
                continue;
            }
            let mut value = std::env::join_paths(&ours).map_err(|err| {
                BuildError::InvalidConfiguration(format!(
                    "cannot add dependency directories to {key}: {err}"
                ))
            })?;
            if let Some(existing) = selection.search_path(key).filter(|v| !v.is_empty()) {
                value.push(if cfg!(windows) { ";" } else { ":" });
                value.push(existing);
            }
            command.env(key, value);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{BuildConfiguration, BuildFingerprint, ToolchainInfo};
    use crate::install::InstalledFileType;
    use crate::lock::Checksum;
    use crate::package::PackageName;

    fn dependencies(entries: &[(&str, &str, char)]) -> BuildDependencies {
        let mut dependencies = BuildDependencies::default();
        for (name, version, seed) in entries {
            dependencies.insert(
                name,
                version,
                &format!("sha256:{}", seed.to_string().repeat(64)),
                PathBuf::from("/prefix").join(name),
                &[],
            );
        }
        dependencies
    }

    fn digest(dependencies: &BuildDependencies) -> BuildFingerprint {
        let toolchain = ToolchainInfo {
            cmake_version: "cmake version 3.31.0".to_string(),
            c_compiler_version: Some("gcc (GCC) 14.2.0".to_string()),
            cxx_compiler_version: Some("g++ (GCC) 14.2.0".to_string()),
            generator: "Unix Makefiles".to_string(),
        };
        BuildFingerprint::compute_with_inputs(
            &PackageId::new(
                PackageName::new("mid").unwrap(),
                semver::Version::new(1, 0, 0),
            ),
            &Checksum::new_sha256("0".repeat(64)).unwrap(),
            &toolchain,
            &BuildConfiguration::default(),
            &[],
            &dependencies.fingerprint_fields(),
        )
    }

    #[test]
    fn every_dependency_input_changes_the_fingerprint_and_order_does_not() {
        let none = digest(&BuildDependencies::default());
        let base = digest(&dependencies(&[
            ("leaf", "1.0.0", 'a'),
            ("zlib", "1.3.1", 'b'),
        ]));
        assert_eq!(
            base,
            digest(&dependencies(&[
                ("zlib", "1.3.1", 'b'),
                ("leaf", "1.0.0", 'a')
            ]))
        );

        let mut seen = BTreeSet::from([none, base]);
        for variant in [
            &[("leaf", "1.0.0", 'a')][..],
            &[("other", "1.0.0", 'a'), ("zlib", "1.3.1", 'b')],
            &[("leaf", "1.0.1", 'a'), ("zlib", "1.3.1", 'b')],
            &[("leaf", "1.0.0", 'c'), ("zlib", "1.3.1", 'b')],
        ] {
            assert!(
                seen.insert(digest(&dependencies(variant))),
                "unchanged digest for {variant:?}"
            );
        }
    }

    #[test]
    fn the_install_location_is_not_fingerprinted() {
        let mut moved = dependencies(&[("leaf", "1.0.0", 'a')]);
        let original = digest(&moved);
        moved.0.get_mut("leaf").unwrap().prefix = PathBuf::from("/elsewhere");
        assert_eq!(digest(&moved), original);
    }

    #[test]
    fn module_and_pkg_config_directories_come_from_the_inventory() {
        let files = [
            "include/x.h",
            "lib/pkgconfig/x.pc",
            "share/pkgconfig/y.pc",
            "share/cmake/Modules/Helpers.cmake",
            "lib/cmake/x/FindZ.cmake",
            "lib/cmake/x/xConfig.cmake",
        ]
        .map(|path| InstalledFile {
            relative_path: PathBuf::from(path),
            file_type: InstalledFileType::Other,
            size_bytes: 0,
        });
        let mut dependencies = BuildDependencies::default();
        dependencies.insert("x", "1.0.0", "sha256:x", PathBuf::from("/p"), &files);

        assert_eq!(
            dependencies.module_dirs().collect::<Vec<_>>(),
            [
                Path::new("/p/lib/cmake/x"),
                Path::new("/p/share/cmake/Modules")
            ]
        );
        assert_eq!(
            dependencies.0["x"].pkg_config_dirs,
            BTreeSet::from(["/p/lib/pkgconfig".into(), "/p/share/pkgconfig".into()])
        );
        assert_eq!(BuildDependencies::default().module_dirs().count(), 0);
    }
}
