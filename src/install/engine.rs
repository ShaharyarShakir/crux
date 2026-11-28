//! Package installation: staging, inventory, publication and current-record
//! lifecycle. The build system's own install step is run by its adapter.

use super::InstallError;
use super::current::{
    CURRENT_RECORD_FORMAT_VERSION, CurrentInstallation, remove_current_record, write_current_record,
};
use super::inventory::scan_staging_dir;
use super::metadata::{
    CURRENT_INSTALL_METADATA_VERSION, INSTALL_METADATA_FILENAME, InstallMetadata, InstalledFile,
};
use crate::build::package_dependencies::BuildDependencies;
use crate::build::systems::cmake::CMake;
use crate::build::systems::{BuildSystemKind, PackageBuildSystem, REINSTALL_MARKER};
use crate::build::{BuildFingerprint, BuildResult};
use crate::core::paths::normalize_path;
use crate::lock::Checksum;
use crate::package::PackageId;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// Request to install a built package.
#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub build_result: BuildResult,
    pub force: bool,
    /// Checksum of the source that was built, recorded alongside the current
    /// installation so later edits to a mutable source can be detected.
    pub source_checksum: Option<Checksum>,
}

impl InstallRequest {
    /// Creates a new install request for a completed build result.
    pub fn new(build_result: BuildResult) -> Self {
        Self {
            build_result,
            force: false,
            source_checksum: None,
        }
    }

    /// Records the checksum of the source that was built.
    pub fn with_source_checksum(mut self, checksum: Checksum) -> Self {
        self.source_checksum = Some(checksum);
        self
    }

    /// Sets whether to bypass existing installation cache and reinstall forcefully.
    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

/// Result of a completed or cache-hit package installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallResult {
    pub package_id: PackageId,
    pub fingerprint: BuildFingerprint,
    pub install_dir: PathBuf,
    pub metadata_path: PathBuf,
    pub installed_files: Vec<InstalledFile>,
    pub total_size_bytes: u64,
    pub reused_install: bool,
}

/// Abstract interface for package installation engines.
pub trait InstallEngine {
    fn install(&self, request: &InstallRequest) -> Result<InstallResult, InstallError>;
}

/// CMake-backed installation engine executing `cmake --install`.
pub struct CMakeInstallEngine {
    install_root: PathBuf,
    staging_root: PathBuf,
    cmake: CMake,
}

impl CMakeInstallEngine {
    /// Creates a new CMake installation engine managing installations under `install_root`
    /// and staging operations under `staging_root`.
    pub fn new(install_root: impl Into<PathBuf>, staging_root: impl Into<PathBuf>) -> Self {
        Self {
            install_root: install_root.into(),
            staging_root: staging_root.into(),
            cmake: CMake::default(),
        }
    }

    /// Overrides the path to the CMake binary.
    pub fn with_cmake_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.cmake.bin = bin.into();
        self
    }

    /// Returns the root directory where installed packages are managed.
    pub fn install_root(&self) -> &Path {
        &self.install_root
    }

    /// Returns the root directory where staging occurs.
    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    /// Computes the deterministic final installation directory for a given package and fingerprint.
    pub fn destination_dir(
        &self,
        package_id: &PackageId,
        fingerprint: &BuildFingerprint,
    ) -> PathBuf {
        self.install_root
            .join(package_id.name().as_str())
            .join(package_id.version().to_string())
            .join(fingerprint.hex_digest())
    }

    /// Marks the installation of `request` as the current one for its profile.
    /// Must only be called once that installation is complete on disk.
    fn record_current(
        request: &InstallRequest,
        dest_dir: &Path,
        dependencies: &BuildDependencies,
    ) -> Result<(), InstallError> {
        let Some(version_dir) = dest_dir.parent() else {
            return Ok(());
        };
        let build = &request.build_result;
        let record = CurrentInstallation {
            format_version: CURRENT_RECORD_FORMAT_VERSION,
            package_name: build.package_id.name().as_str().to_string(),
            package_version: build.package_id.version().to_string(),
            profile: build.profile,
            fingerprint: build.fingerprint.clone(),
            source_checksum: request.source_checksum.as_ref().map(ToString::to_string),
            dependencies: dependencies.record(),
        };
        write_current_record(version_dir, &record).map_err(InstallError::Io)
    }

    /// Validates an existing installation on disk against metadata.
    fn check_existing_cache(
        &self,
        request: &InstallRequest,
        dest_dir: &Path,
        metadata_path: &Path,
    ) -> Option<InstallResult> {
        if !dest_dir.is_dir() || !metadata_path.is_file() {
            return None;
        }

        let meta_str = fs::read_to_string(metadata_path).ok()?;
        let metadata = toml::from_str::<InstallMetadata>(&meta_str).ok()?;

        if metadata.format_version != CURRENT_INSTALL_METADATA_VERSION
            || metadata.package_name != request.build_result.package_id.name().as_str()
            || metadata.package_version != request.build_result.package_id.version().to_string()
            || metadata.fingerprint != request.build_result.fingerprint
            || metadata.profile != request.build_result.profile
            || !metadata.success
            || metadata.installed_files.is_empty()
        {
            return None;
        }

        // Verify that every recorded file exists on disk and matches size
        for file in &metadata.installed_files {
            // Guard against path traversal in metadata
            if file.relative_path.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                return None;
            }

            let full_path = dest_dir.join(&file.relative_path);
            let meta = fs::symlink_metadata(&full_path).ok()?;

            if meta.is_symlink() {
                // Verify symlink does not escape dest_dir
                if let Ok(target) = fs::read_link(&full_path) {
                    let resolved = if target.is_relative() {
                        let parent = full_path.parent().unwrap_or(dest_dir);
                        normalize_path(&parent.join(target))
                    } else {
                        target
                    };
                    if !resolved.starts_with(dest_dir) {
                        return None;
                    }
                } else {
                    return None;
                }
            } else if meta.is_file() {
                if meta.len() != file.size_bytes {
                    return None;
                }
            } else {
                return None;
            }
        }

        Some(InstallResult {
            package_id: request.build_result.package_id.clone(),
            fingerprint: request.build_result.fingerprint.clone(),
            install_dir: dest_dir.to_path_buf(),
            metadata_path: metadata_path.to_path_buf(),
            installed_files: metadata.installed_files,
            total_size_bytes: metadata.total_size_bytes,
            reused_install: true,
        })
    }

    /// Writes the installation metadata to `installed.toml` atomically inside `staging_dir`.
    fn write_metadata_atomically(
        staging_dir: &Path,
        metadata: &InstallMetadata,
    ) -> Result<PathBuf, InstallError> {
        let serialized = toml::to_string_pretty(metadata).map_err(|e| {
            InstallError::MetadataError(format!("failed to serialize installation metadata: {e}"))
        })?;

        let metadata_path = staging_dir.join(INSTALL_METADATA_FILENAME);
        let temp_path = staging_dir.join(".installed.toml.tmp");

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)
            .map_err(InstallError::Io)?;

        file.write_all(serialized.as_bytes())
            .map_err(InstallError::Io)?;
        file.flush().map_err(InstallError::Io)?;
        file.sync_data().map_err(InstallError::Io)?;
        drop(file);

        fs::rename(&temp_path, &metadata_path).map_err(InstallError::Io)?;
        Ok(metadata_path)
    }
}

impl InstallEngine for CMakeInstallEngine {
    fn install(&self, request: &InstallRequest) -> Result<InstallResult, InstallError> {
        self.install_with_dependencies(
            BuildSystemKind::Cmake,
            request,
            &BuildDependencies::default(),
        )
    }
}

impl CMakeInstallEngine {
    /// Installs, recording the dependency installations the build used.
    pub(crate) fn install_with_dependencies(
        &self,
        kind: BuildSystemKind,
        request: &InstallRequest,
        dependencies: &BuildDependencies,
    ) -> Result<InstallResult, InstallError> {
        let system = match kind {
            BuildSystemKind::Cmake => PackageBuildSystem::Cmake(&self.cmake),
        };

        // 1. Validate build result prerequisites
        let build_dir = &request.build_result.build_dir;
        if !build_dir.is_dir() {
            return Err(InstallError::MissingBuildDirectory(build_dir.clone()));
        }
        system.check_installable(build_dir)?;

        let dest_dir = self.destination_dir(
            &request.build_result.package_id,
            &request.build_result.fingerprint,
        );
        let metadata_path = dest_dir.join(INSTALL_METADATA_FILENAME);

        // 2. Check for reusable cached installation if not forced, unless the
        //    build engine discarded a stale build with this fingerprint
        let reinstall_marker = request
            .build_result
            .metadata_path
            .with_file_name(REINSTALL_MARKER);
        if !request.force
            && !reinstall_marker.exists()
            && let Some(cached_result) =
                self.check_existing_cache(request, &dest_dir, &metadata_path)
        {
            Self::record_current(request, &dest_dir, dependencies)?;
            return Ok(cached_result);
        }

        // 3. Prepare unique staging directory
        fs::create_dir_all(&self.staging_root).map_err(InstallError::Io)?;
        let unique_id = format!(
            "{}_{}_{}_{}",
            request.build_result.package_id.name().as_str(),
            request.build_result.package_id.version(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let staging_dir = self.staging_root.join(&unique_id);
        fs::create_dir_all(&staging_dir).map_err(InstallError::Io)?;

        // Ensure staging directory is cleaned up on error or panic
        let mut staging_guard = StagingGuard {
            path: &staging_dir,
            active: true,
        };

        // 4. Run the build system's install step into the staging directory
        system.install_into(&request.build_result, &staging_dir)?;

        // 5. Scan, validate, and inventory staged files
        let (installed_files, total_size_bytes) = scan_staging_dir(&staging_dir)?;

        // 6. Write `installed.toml` metadata atomically inside staging directory
        let metadata = InstallMetadata {
            format_version: CURRENT_INSTALL_METADATA_VERSION,
            package_name: request.build_result.package_id.name().as_str().to_string(),
            package_version: request.build_result.package_id.version().to_string(),
            fingerprint: request.build_result.fingerprint.clone(),
            profile: request.build_result.profile,
            installed_files: installed_files.clone(),
            total_size_bytes,
            success: true,
            installed_at: None,
        };
        Self::write_metadata_atomically(&staging_dir, &metadata)?;

        // 7. Atomic publication to final destination directory
        // The old record is dropped before publishing: if this process stops
        // between publication and recording, no record may keep naming an
        // older installation as current.
        if let Some(parent) = dest_dir.parent() {
            fs::create_dir_all(parent).map_err(InstallError::Io)?;
            remove_current_record(parent, request.build_result.profile)
                .map_err(InstallError::Io)?;
        }

        if dest_dir.exists() {
            let backup_dir = dest_dir.with_file_name(format!(
                "{}.old.{}",
                dest_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("install"),
                unique_id
            ));

            fs::rename(&dest_dir, &backup_dir).map_err(InstallError::Io)?;

            if let Err(err) = fs::rename(&staging_dir, &dest_dir) {
                // Restore backup on failure
                let _ = fs::rename(&backup_dir, &dest_dir);
                return Err(InstallError::Io(err));
            }

            // Cleanup backup
            let _ = fs::remove_dir_all(&backup_dir);
        } else {
            fs::rename(&staging_dir, &dest_dir).map_err(InstallError::Io)?;
        }

        // Defuse staging cleanup guard now that publication succeeded
        staging_guard.active = false;
        match fs::remove_file(&reinstall_marker) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                return Err(InstallError::Io(err));
            }
            _ => {}
        }
        Self::record_current(request, &dest_dir, dependencies)?;

        Ok(InstallResult {
            package_id: request.build_result.package_id.clone(),
            fingerprint: request.build_result.fingerprint.clone(),
            install_dir: dest_dir,
            metadata_path,
            installed_files,
            total_size_bytes,
            reused_install: false,
        })
    }
}

/// RAII guard to safely remove staging directories upon error or panic.
pub struct StagingGuard<'a> {
    pub path: &'a Path,
    pub active: bool,
}

impl Drop for StagingGuard<'_> {
    fn drop(&mut self) {
        if self.active && self.path.exists() {
            let _ = fs::remove_dir_all(self.path);
        }
    }
}
