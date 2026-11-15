//! Package installation verification and dependency collection for CMake integration.

use super::IntegrationError;
use crate::build::{BuildFingerprint, BuildProfile};
use crate::core::paths::normalize_path;
use crate::install::{
    CURRENT_INSTALL_METADATA_VERSION, INSTALL_METADATA_FILENAME, InstallMetadata,
};
use crate::lock::{LockedPackage, Lockfile};
use crate::package::{PackageId, PackageName};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// A validated package installation verified on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPackageInstallation {
    pub package_id: PackageId,
    pub fingerprint: BuildFingerprint,
    pub profile: BuildProfile,
    pub install_dir: PathBuf,
    pub metadata: InstallMetadata,
}

/// Validates an installed package on disk against its metadata and security constraints.
pub fn validate_package_installation(
    project_root: &Path,
    package_name: &str,
    package_version: &str,
    profile: BuildProfile,
) -> Result<ValidatedPackageInstallation, IntegrationError> {
    let pkg_name = PackageName::new(package_name).map_err(|e| {
        IntegrationError::InvalidPath(format!("invalid package name '{package_name}': {e}"))
    })?;

    let version = semver::Version::parse(package_version).map_err(|e| {
        IntegrationError::InvalidPath(format!("invalid package version '{package_version}': {e}"))
    })?;

    let install_root = project_root.join(".crux").join("installed");
    let pkg_version_dir = install_root
        .join(pkg_name.as_str())
        .join(version.to_string());

    if !pkg_version_dir.is_dir() {
        return Err(IntegrationError::PackageNotInstalled {
            package: package_name.to_string(),
            version: package_version.to_string(),
        });
    }

    let entries = fs::read_dir(&pkg_version_dir).map_err(IntegrationError::Io)?;
    let mut candidate_dirs = Vec::new();

    for entry in entries {
        let entry = entry.map_err(IntegrationError::Io)?;
        let path = entry.path();
        if path.is_dir() {
            candidate_dirs.push(path);
        }
    }

    if candidate_dirs.is_empty() {
        return Err(IntegrationError::PackageNotInstalled {
            package: package_name.to_string(),
            version: package_version.to_string(),
        });
    }

    // Sort candidate fingerprint directories for determinism
    candidate_dirs.sort();

    let mut matched_candidate = None;
    let mut last_error = None;

    for candidate_dir in candidate_dirs {
        let metadata_path = candidate_dir.join(INSTALL_METADATA_FILENAME);
        if !metadata_path.is_file() {
            continue;
        }

        let meta_str = match fs::read_to_string(&metadata_path) {
            Ok(s) => s,
            Err(e) => {
                last_error = Some(IntegrationError::CorruptInstallation {
                    package: package_name.to_string(),
                    reason: format!(
                        "failed to read metadata at '{}': {e}",
                        metadata_path.display()
                    ),
                });
                continue;
            }
        };

        let metadata: InstallMetadata = match toml::from_str(&meta_str) {
            Ok(m) => m,
            Err(e) => {
                last_error = Some(IntegrationError::CorruptInstallation {
                    package: package_name.to_string(),
                    reason: format!(
                        "failed to parse installed.toml at '{}': {e}",
                        metadata_path.display()
                    ),
                });
                continue;
            }
        };

        if metadata.profile != profile {
            continue;
        }

        if metadata.format_version != CURRENT_INSTALL_METADATA_VERSION {
            return Err(IntegrationError::CorruptInstallation {
                package: package_name.to_string(),
                reason: format!(
                    "unsupported metadata version {} (expected {})",
                    metadata.format_version, CURRENT_INSTALL_METADATA_VERSION
                ),
            });
        }

        if metadata.package_name != pkg_name.as_str() {
            return Err(IntegrationError::CorruptInstallation {
                package: package_name.to_string(),
                reason: format!(
                    "metadata package name '{}' does not match directory package '{}'",
                    metadata.package_name,
                    pkg_name.as_str()
                ),
            });
        }

        if metadata.package_version != version.to_string() {
            return Err(IntegrationError::CorruptInstallation {
                package: package_name.to_string(),
                reason: format!(
                    "metadata package version '{}' does not match directory version '{}'",
                    metadata.package_version, version
                ),
            });
        }

        let dir_name = candidate_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if dir_name != metadata.fingerprint.hex_digest() {
            return Err(IntegrationError::CorruptInstallation {
                package: package_name.to_string(),
                reason: format!(
                    "fingerprint directory name '{dir_name}' does not match metadata digest '{}'",
                    metadata.fingerprint.hex_digest()
                ),
            });
        }

        if !metadata.success {
            return Err(IntegrationError::CorruptInstallation {
                package: package_name.to_string(),
                reason: "installation metadata records failure (success = false)".to_string(),
            });
        }

        if metadata.installed_files.is_empty() {
            return Err(IntegrationError::CorruptInstallation {
                package: package_name.to_string(),
                reason: "installation contains no files".to_string(),
            });
        }

        let norm_candidate = normalize_path(&candidate_dir);
        let norm_install_root = normalize_path(&install_root);
        if !norm_candidate.starts_with(&norm_install_root) {
            return Err(IntegrationError::SecurityViolation {
                package: package_name.to_string(),
                reason: format!(
                    "installation directory '{}' escapes install root '{}'",
                    norm_candidate.display(),
                    norm_install_root.display()
                ),
            });
        }

        for file in &metadata.installed_files {
            if file.relative_path.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                return Err(IntegrationError::SecurityViolation {
                    package: package_name.to_string(),
                    reason: format!(
                        "relative path '{}' contains traversal components",
                        file.relative_path.display()
                    ),
                });
            }

            let full_path = candidate_dir.join(&file.relative_path);
            let meta = match fs::symlink_metadata(&full_path) {
                Ok(m) => m,
                Err(e) => {
                    return Err(IntegrationError::CorruptInstallation {
                        package: package_name.to_string(),
                        reason: format!(
                            "indexed file '{}' is missing on disk: {e}",
                            file.relative_path.display()
                        ),
                    });
                }
            };

            if meta.is_symlink() {
                let target = match fs::read_link(&full_path) {
                    Ok(t) => t,
                    Err(e) => {
                        return Err(IntegrationError::CorruptInstallation {
                            package: package_name.to_string(),
                            reason: format!(
                                "failed to read symlink target for '{}': {e}",
                                file.relative_path.display()
                            ),
                        });
                    }
                };
                let resolved = if target.is_relative() {
                    let parent = full_path.parent().unwrap_or(&candidate_dir);
                    normalize_path(&parent.join(&target))
                } else {
                    target
                };
                let norm_candidate_dir = normalize_path(&candidate_dir);
                if !resolved.starts_with(&norm_candidate_dir) {
                    return Err(IntegrationError::SecurityViolation {
                        package: package_name.to_string(),
                        reason: format!(
                            "symlink '{}' points outside install root to '{}'",
                            file.relative_path.display(),
                            resolved.display()
                        ),
                    });
                }
            } else if meta.is_file() && meta.len() != file.size_bytes {
                return Err(IntegrationError::CorruptInstallation {
                    package: package_name.to_string(),
                    reason: format!(
                        "file '{}' size mismatch: expected {} bytes, found {} bytes",
                        file.relative_path.display(),
                        file.size_bytes,
                        meta.len()
                    ),
                });
            }
        }

        let pkg_id = PackageId::new(pkg_name.clone(), version.clone());
        matched_candidate = Some(ValidatedPackageInstallation {
            package_id: pkg_id,
            fingerprint: metadata.fingerprint.clone(),
            profile: metadata.profile,
            install_dir: candidate_dir,
            metadata,
        });
        break;
    }

    if let Some(valid) = matched_candidate {
        Ok(valid)
    } else if let Some(err) = last_error {
        Err(err)
    } else {
        Err(IntegrationError::PackageNotInstalled {
            package: package_name.to_string(),
            version: package_version.to_string(),
        })
    }
}

/// Collects a package and all of its transitive dependencies declared in the lockfile.
pub fn collect_transitive_dependencies(
    lockfile: &Lockfile,
    target_name: &str,
) -> Result<Vec<LockedPackage>, IntegrationError> {
    let mut packages_by_name = BTreeMap::new();
    for pkg in &lockfile.packages {
        packages_by_name.insert(pkg.name.as_str(), pkg);
    }

    let root_pkg = packages_by_name
        .get(target_name)
        .ok_or_else(|| IntegrationError::PackageNotFoundInLockfile(target_name.to_string()))?;

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut result = Vec::new();

    visited.insert(root_pkg.name.as_str());
    queue.push_back(*root_pkg);

    while let Some(current) = queue.pop_front() {
        result.push(current.clone());

        for dep in &current.dependencies {
            let dep_name = dep.name().as_str();
            if !visited.contains(dep_name)
                && let Some(dep_pkg) = packages_by_name.get(dep_name)
            {
                visited.insert(dep_name);
                queue.push_back(*dep_pkg);
            }
        }
    }

    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}
