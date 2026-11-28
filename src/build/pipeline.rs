//! The package build lifecycle shared by every build system: toolchain
//! selection, fingerprint, build directory layout, cache validation, artifact
//! discovery and build metadata. The build system itself is reached only
//! through [`PackageBuildSystem`].

use super::BuildError;
use super::engine::{BuildRequest, BuildResult};
use super::fingerprint::BuildFingerprint;
use super::metadata::{ArtifactType, BuildArtifact, BuildMetadata, CURRENT_BUILD_METADATA_VERSION};
use super::package_dependencies::BuildDependencies;
use super::selection::{AmbientEnv, ToolchainSelection};
use super::systems::{BuildStep, PackageBuildSystem, REINSTALL_MARKER};
use super::toolchain::ToolchainInfo;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One package build with a chosen build system.
pub(crate) struct PackagePipeline<'a> {
    pub builds_root: &'a Path,
    pub system: PackageBuildSystem<'a>,
    /// Replaces the process environment for toolchain selection, when set.
    pub ambient: Option<&'a AmbientEnv>,
}

/// Where one build of a package lives:
/// `<builds_root>/<pkg_name>/<pkg_version>/<fingerprint_hex>/`.
struct BuildLayout {
    root: PathBuf,
    build_dir: PathBuf,
    logs_dir: PathBuf,
    configure_log_path: PathBuf,
    build_log_path: PathBuf,
    metadata_path: PathBuf,
}

impl PackagePipeline<'_> {
    fn fingerprint(
        request: &BuildRequest,
        toolchain: &ToolchainInfo,
        selection: &ToolchainSelection,
        dependencies: &BuildDependencies,
    ) -> BuildFingerprint {
        BuildFingerprint::compute_with_inputs(
            &request.package_id,
            &request.source_checksum,
            toolchain,
            &request.configuration,
            &selection.fingerprint_fields(),
            &dependencies.fingerprint_fields(),
        )
    }

    fn layout(&self, request: &BuildRequest, fingerprint: &BuildFingerprint) -> BuildLayout {
        let root = self
            .builds_root
            .join(request.package_id.name().as_str())
            .join(request.package_id.version().to_string())
            .join(fingerprint.hex_digest());
        let logs_dir = root.join("logs");
        BuildLayout {
            build_dir: root.join("build"),
            configure_log_path: logs_dir.join("configure.log"),
            build_log_path: logs_dir.join("build.log"),
            metadata_path: root.join("metadata.toml"),
            logs_dir,
            root,
        }
    }

    fn read_metadata(metadata_path: &Path) -> Option<BuildMetadata> {
        toml::from_str(&fs::read_to_string(metadata_path).ok()?).ok()
    }

    /// Returns the cached result for `request` when a previous successful
    /// build with the same fingerprint is still complete on disk.
    fn cached_result(
        &self,
        request: &BuildRequest,
        fingerprint: &BuildFingerprint,
        layout: &BuildLayout,
        selection: &ToolchainSelection,
    ) -> Option<BuildResult> {
        if !request.force
            && layout.build_dir.is_dir()
            && let Some(metadata) = Self::read_metadata(&layout.metadata_path)
            && metadata.success
            && metadata.format_version == CURRENT_BUILD_METADATA_VERSION
            && metadata.configure_exit_code == 0
            && metadata.build_exit_code == 0
            && metadata.fingerprint == *fingerprint
            && !self.system.conflicts_with(
                selection,
                metadata.selected_toolchain.as_ref(),
                &layout.build_dir,
                &request.configuration,
            )
        {
            // A build may legitimately produce nothing (header-only packages);
            // its configured tree must then still be there to install from.
            // Recorded artifacts must still exist and match their sizes.
            let all_artifacts_valid = (!metadata.artifacts.is_empty()
                || self.system.is_configured(&layout.build_dir))
                && metadata.artifacts.iter().all(|a| {
                    if let Ok(meta) = fs::metadata(&a.path) {
                        meta.is_file() && meta.len() == a.size_bytes
                    } else {
                        false
                    }
                });

            if all_artifacts_valid {
                return Some(BuildResult {
                    package_id: request.package_id.clone(),
                    fingerprint: fingerprint.clone(),
                    source_dir: request.source_dir.clone(),
                    build_dir: layout.build_dir.clone(),
                    logs_dir: layout.logs_dir.clone(),
                    configure_log_path: layout.configure_log_path.clone(),
                    build_log_path: layout.build_log_path.clone(),
                    metadata_path: layout.metadata_path.clone(),
                    artifacts: metadata.artifacts,
                    profile: request.configuration.profile,
                    duration: Duration::from_millis(metadata.duration_millis),
                    reused_cache: true,
                });
            }
        }
        None
    }

    /// Whether [`PackagePipeline::build`] would reuse a cached result.
    pub(crate) fn is_fresh(
        &self,
        request: &BuildRequest,
        dependencies: &BuildDependencies,
    ) -> bool {
        if request.force
            || !request.source_dir.is_dir()
            || self.system.check_source(&request.source_dir).is_err()
        {
            return false;
        }
        let Ok((toolchain, selection)) = self.system.detect(&request.configuration, self.ambient)
        else {
            return false;
        };
        let fingerprint = Self::fingerprint(request, &toolchain, &selection, dependencies);
        let layout = self.layout(request, &fingerprint);
        self.cached_result(request, &fingerprint, &layout, &selection)
            .is_some()
    }

    /// Builds `request` against `dependencies`, which are exposed to the build
    /// and covered by the fingerprint, or reuses an identical earlier build.
    pub(crate) fn build(
        &self,
        request: &BuildRequest,
        dependencies: &BuildDependencies,
    ) -> Result<BuildResult, BuildError> {
        let start_time = Instant::now();

        // 1. Validate source directory
        if !request.source_dir.is_dir() {
            return Err(BuildError::MissingSource(request.source_dir.clone()));
        }
        self.system.check_source(&request.source_dir)?;

        // 2. Detect Toolchain and compute BuildFingerprint
        let (toolchain, selection) = self.system.detect(&request.configuration, self.ambient)?;
        let fingerprint = Self::fingerprint(request, &toolchain, &selection, dependencies);

        // 3. Isolated directory structure
        let layout = self.layout(request, &fingerprint);

        // 4. Check for reusable cached build result
        if let Some(result) = self.cached_result(request, &fingerprint, &layout, &selection) {
            return Ok(result);
        }

        // 5. Prepare build directory: ensure build/ and logs/ exist and are clean
        let build_dir = &layout.build_dir;
        let previous = Self::read_metadata(&layout.metadata_path);
        let stale = build_dir.is_dir()
            && self.system.conflicts_with(
                &selection,
                previous
                    .as_ref()
                    .and_then(|m| m.selected_toolchain.as_ref()),
                build_dir,
                &request.configuration,
            );
        if stale {
            // Written first, so an interruption cannot lose it.
            crate::core::filesystem::write_atomically(&layout.root.join(REINSTALL_MARKER), b"")?;
        }
        // A build that fails or is interrupted must leave no success record.
        match fs::remove_file(&layout.metadata_path) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.into()),
            _ => {}
        }
        if build_dir.exists()
            && (request.force || stale || self.system.tree_needs_reset(&selection, build_dir))
        {
            fs::remove_dir_all(build_dir).map_err(BuildError::Io)?;
        }
        fs::create_dir_all(build_dir).map_err(BuildError::Io)?;
        fs::create_dir_all(&layout.logs_dir).map_err(BuildError::Io)?;

        // 6. Configure and compile with the package's build system
        let exit_codes = self.system.configure_and_build(&BuildStep {
            request,
            toolchain: &toolchain,
            selection: &selection,
            dependencies,
            build_dir,
            configure_log_path: &layout.configure_log_path,
            build_log_path: &layout.build_log_path,
        })?;

        // 7. Scan for produced artifacts
        let artifacts = self.scan_artifacts(build_dir)?;
        let duration = start_time.elapsed();

        // 8. Serialize and save BuildMetadata atomically
        let metadata = BuildMetadata {
            format_version: CURRENT_BUILD_METADATA_VERSION,
            package_name: request.package_id.name().to_string(),
            package_version: request.package_id.version().to_string(),
            source_checksum: request.source_checksum.to_string(),
            fingerprint: fingerprint.clone(),
            profile: request.configuration.profile,
            toolchain,
            configuration: request.configuration.clone(),
            source_dir: request.source_dir.clone(),
            build_dir: build_dir.clone(),
            configure_exit_code: exit_codes.configure,
            build_exit_code: exit_codes.build,
            duration_millis: duration.as_millis() as u64,
            artifacts: artifacts.clone(),
            success: true,
            selected_toolchain: Some(selection.record()),
        };

        let metadata_toml = toml::to_string_pretty(&metadata).map_err(|e| {
            BuildError::MetadataError(format!("failed to serialize build metadata: {e}"))
        })?;
        crate::core::filesystem::write_atomically(&layout.metadata_path, metadata_toml.as_bytes())?;

        Ok(BuildResult {
            package_id: request.package_id.clone(),
            fingerprint,
            source_dir: request.source_dir.clone(),
            build_dir: layout.build_dir,
            logs_dir: layout.logs_dir,
            configure_log_path: layout.configure_log_path,
            build_log_path: layout.build_log_path,
            metadata_path: layout.metadata_path,
            artifacts,
            profile: request.configuration.profile,
            duration,
            reused_cache: false,
        })
    }

    /// Scans a build directory and collects all produced artifacts.
    fn scan_artifacts(&self, build_dir: &Path) -> Result<Vec<BuildArtifact>, BuildError> {
        let mut artifacts = Vec::new();
        if !build_dir.is_dir() {
            return Ok(artifacts);
        }

        self.scan_dir_recursive(build_dir, build_dir, &mut artifacts)?;
        artifacts.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(artifacts)
    }

    fn scan_dir_recursive(
        &self,
        current_dir: &Path,
        build_root: &Path,
        artifacts: &mut Vec<BuildArtifact>,
    ) -> Result<(), BuildError> {
        for entry in fs::read_dir(current_dir).map_err(BuildError::Io)? {
            let entry = entry.map_err(BuildError::Io)?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if self.system.is_build_internal(&file_name) {
                continue;
            }

            let file_type = entry.file_type().map_err(BuildError::Io)?;
            if file_type.is_dir() {
                self.scan_dir_recursive(&path, build_root, artifacts)?;
            } else if file_type.is_file() {
                let artifact_type = ArtifactType::from_path(&path);
                // Include static/shared libraries, executables, headers, or any other relevant files
                if artifact_type != ArtifactType::Other || file_name.ends_with(".pc") {
                    let size_bytes = entry.metadata().map_err(BuildError::Io)?.len();
                    let relative_path =
                        path.strip_prefix(build_root).unwrap_or(&path).to_path_buf();

                    artifacts.push(BuildArtifact {
                        name: file_name,
                        relative_path,
                        path,
                        artifact_type,
                        size_bytes,
                    });
                }
            }
        }

        Ok(())
    }
}
