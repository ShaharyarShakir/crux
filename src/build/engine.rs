//! Public package build interface: requests, results and the CMake engine.

use super::BuildError;
use super::fingerprint::BuildFingerprint;
use super::metadata::{BuildArtifact, BuildConfiguration, BuildProfile};
use super::package_dependencies::BuildDependencies;
use super::pipeline::PackagePipeline;
use super::selection::AmbientEnv;
use super::systems::cmake::CMake;
use super::systems::{BuildSystemKind, PackageBuildSystem};
use super::toolchain::ToolchainInfo;
use crate::lock::Checksum;
use crate::package::PackageId;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Request to build a package.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub package_id: PackageId,
    pub source_dir: PathBuf,
    pub source_checksum: Checksum,
    pub configuration: BuildConfiguration,
    pub force: bool,
    /// Optional live observer of CMake/compiler output (used for `-vv`).
    pub output_observer: Option<crate::core::process::OutputObserver>,
}

impl BuildRequest {
    /// Creates a new build request for a package with default configuration.
    pub fn new(package_id: PackageId, source_dir: PathBuf, source_checksum: Checksum) -> Self {
        Self {
            package_id,
            source_dir,
            source_checksum,
            configuration: BuildConfiguration::default(),
            force: false,
            output_observer: None,
        }
    }

    /// Streams CMake and compiler output to `observer` while it is produced.
    pub fn with_output_observer(mut self, observer: crate::core::process::OutputObserver) -> Self {
        self.output_observer = Some(observer);
        self
    }

    /// Sets the build configuration.
    pub fn with_configuration(mut self, config: BuildConfiguration) -> Self {
        self.configuration = config;
        self
    }

    /// Enables forced rebuild, bypassing any existing build cache.
    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

/// Result of a completed or cache-hit package build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResult {
    pub package_id: PackageId,
    pub fingerprint: BuildFingerprint,
    pub source_dir: PathBuf,
    pub build_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub configure_log_path: PathBuf,
    pub build_log_path: PathBuf,
    pub metadata_path: PathBuf,
    pub artifacts: Vec<BuildArtifact>,
    pub profile: BuildProfile,
    pub duration: Duration,
    pub reused_cache: bool,
}

/// Abstract interface for package build engines.
pub trait BuildEngine {
    fn build(&self, request: &BuildRequest) -> Result<BuildResult, BuildError>;
}

/// CMake-backed build engine implementing out-of-source isolated compilation.
///
/// The build lifecycle is shared by all package build systems; this type
/// binds it to a builds root and the configured CMake tools.
pub struct CMakeBuildEngine {
    builds_root: PathBuf,
    cmake: CMake,
    ambient_env: Option<AmbientEnv>,
}

impl CMakeBuildEngine {
    /// Creates a new CMake build engine managing builds under `builds_root`.
    pub fn new(builds_root: impl Into<PathBuf>) -> Self {
        Self {
            builds_root: builds_root.into(),
            cmake: CMake::default(),
            ambient_env: None,
        }
    }

    /// Selects compilers and flags from `env` instead of the process
    /// environment (`PATH`, `CC`, `CXX`, `CFLAGS`, `CXXFLAGS`, `LDFLAGS`,
    /// `CMAKE_TOOLCHAIN_FILE`, `CMAKE_PREFIX_PATH`, `PKG_CONFIG_PATH`).
    pub fn with_environment<K, V>(mut self, env: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.ambient_env = Some(env.into_iter().map(|(k, v)| (k.into(), v.into())).collect());
        self
    }

    /// Overrides the path to the CMake binary.
    pub fn with_cmake_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.cmake.bin = bin.into();
        self
    }

    /// Overrides the default C compiler binary.
    pub fn with_c_compiler(mut self, bin: impl Into<PathBuf>) -> Self {
        self.cmake.c_compiler = Some(bin.into());
        self
    }

    /// Overrides the default C++ compiler binary.
    pub fn with_cxx_compiler(mut self, bin: impl Into<PathBuf>) -> Self {
        self.cmake.cxx_compiler = Some(bin.into());
        self
    }

    /// Returns the root directory where all builds are managed.
    pub fn builds_root(&self) -> &Path {
        &self.builds_root
    }

    /// Detects toolchain information by executing detection commands.
    pub fn detect_toolchain(
        &self,
        config: &BuildConfiguration,
    ) -> Result<ToolchainInfo, BuildError> {
        Ok(self.cmake.detect(config, self.ambient_env.as_ref())?.0)
    }

    /// The shared lifecycle running the build system `kind`.
    fn pipeline(&self, kind: BuildSystemKind) -> PackagePipeline<'_> {
        PackagePipeline {
            builds_root: &self.builds_root,
            system: match kind {
                BuildSystemKind::Cmake => PackageBuildSystem::Cmake(&self.cmake),
            },
            ambient: self.ambient_env.as_ref(),
        }
    }

    /// Returns `true` when building `request` would reuse a cached result,
    /// so callers can report `Fresh` instead of `Compiling` up front. Runs
    /// the same toolchain detection and fingerprinting as [`BuildEngine::build`].
    pub fn is_fresh(&self, request: &BuildRequest) -> bool {
        self.is_fresh_with_dependencies(
            BuildSystemKind::Cmake,
            request,
            &BuildDependencies::default(),
        )
    }

    pub(crate) fn is_fresh_with_dependencies(
        &self,
        kind: BuildSystemKind,
        request: &BuildRequest,
        dependencies: &BuildDependencies,
    ) -> bool {
        self.pipeline(kind).is_fresh(request, dependencies)
    }

    /// Builds a package against `dependencies` with its build system `kind`.
    pub(crate) fn build_with_dependencies(
        &self,
        kind: BuildSystemKind,
        request: &BuildRequest,
        dependencies: &BuildDependencies,
    ) -> Result<BuildResult, BuildError> {
        self.pipeline(kind).build(request, dependencies)
    }
}

impl BuildEngine for CMakeBuildEngine {
    /// Builds a package without Crux dependencies. The `crux` commands supply
    /// the locked dependencies of a package themselves.
    fn build(&self, request: &BuildRequest) -> Result<BuildResult, BuildError> {
        self.build_with_dependencies(
            BuildSystemKind::Cmake,
            request,
            &BuildDependencies::default(),
        )
    }
}
