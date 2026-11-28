//! Package build system selection and dispatch.
//!
//! The shared lifecycle (`super::pipeline` and the install engine) handles
//! fingerprints, caching, metadata, dependencies and publication. Everything
//! that depends on which build system a package uses goes through
//! [`PackageBuildSystem`]. Application builds do not use this module.
//!
//! The set of build systems is closed: adding one means adding a variant to
//! both enums, and the compiler then points at every decision it has to make.

pub(crate) mod cmake;

use self::cmake::CMake;
use super::package_dependencies::BuildDependencies;
use super::selection::{AmbientEnv, ToolchainSelection};
use super::toolchain::ToolchainInfo;
use super::{BuildError, BuildRequest, BuildResult};
use crate::build::metadata::{BuildConfiguration, SelectedToolchain};
use crate::install::InstallError;
use std::path::Path;

/// Left in a package build root when a stale build was discarded, so that an
/// installation made from it is replaced instead of reused.
pub(crate) const REINSTALL_MARKER: &str = "reinstall-required";

/// The build system a package declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildSystemKind {
    Cmake,
}

impl BuildSystemKind {
    /// The kind for a definition's `[build] system` value. Definitions without
    /// one are CMake packages. An unknown value is `None`, never a fallback.
    pub(crate) fn from_declared(declared: Option<&str>) -> Option<Self> {
        match declared {
            None | Some("cmake") => Some(Self::Cmake),
            Some(_) => None,
        }
    }
}

/// One configured build directory to compile.
pub(crate) struct BuildStep<'a> {
    pub request: &'a BuildRequest,
    pub toolchain: &'a ToolchainInfo,
    pub selection: &'a ToolchainSelection,
    pub dependencies: &'a BuildDependencies,
    pub build_dir: &'a Path,
    pub configure_log_path: &'a Path,
    pub build_log_path: &'a Path,
}

/// Exit codes of a successful configure and build.
pub(crate) struct StepExitCodes {
    pub configure: i32,
    pub build: i32,
}

/// The adapter running a package's build system.
#[derive(Clone, Copy)]
pub(crate) enum PackageBuildSystem<'a> {
    Cmake(&'a CMake),
}

impl PackageBuildSystem<'_> {
    /// Fails when `source_dir` is not a project of this build system.
    pub(crate) fn check_source(self, source_dir: &Path) -> Result<(), BuildError> {
        match self {
            Self::Cmake(cmake) => cmake.check_source(source_dir),
        }
    }

    /// Finds the build tools and resolves the compilers for `config`.
    pub(crate) fn detect(
        self,
        config: &BuildConfiguration,
        ambient: Option<&AmbientEnv>,
    ) -> Result<(ToolchainInfo, ToolchainSelection), BuildError> {
        match self {
            Self::Cmake(cmake) => cmake.detect(config, ambient),
        }
    }

    /// Whether `build_dir` still holds a configured tree to install from.
    pub(crate) fn is_configured(self, build_dir: &Path) -> bool {
        match self {
            Self::Cmake(cmake) => cmake.is_configured(build_dir),
        }
    }

    /// Whether the build in `build_dir` is known to have used another compiler
    /// or other ambient flags than `selection`.
    pub(crate) fn conflicts_with(
        self,
        selection: &ToolchainSelection,
        recorded: Option<&SelectedToolchain>,
        build_dir: &Path,
        config: &BuildConfiguration,
    ) -> bool {
        match self {
            Self::Cmake(cmake) => cmake.conflicts_with(selection, recorded, build_dir, config),
        }
    }

    /// Whether `build_dir` cannot be reconfigured in place for `selection`.
    pub(crate) fn tree_needs_reset(self, selection: &ToolchainSelection, build_dir: &Path) -> bool {
        match self {
            Self::Cmake(cmake) => cmake.tree_needs_reset(selection, build_dir),
        }
    }

    /// Configures and compiles one package.
    pub(crate) fn configure_and_build(
        self,
        step: &BuildStep<'_>,
    ) -> Result<StepExitCodes, BuildError> {
        match self {
            Self::Cmake(cmake) => cmake.configure_and_build(step),
        }
    }

    /// Whether a build tree entry is bookkeeping rather than a possible artifact.
    pub(crate) fn is_build_internal(self, file_name: &str) -> bool {
        match self {
            Self::Cmake(cmake) => cmake.is_build_internal(file_name),
        }
    }

    /// Fails when `build_dir` has nothing the install step could use.
    pub(crate) fn check_installable(self, build_dir: &Path) -> Result<(), InstallError> {
        match self {
            Self::Cmake(cmake) => cmake.check_installable(build_dir),
        }
    }

    /// Installs `build` into the empty `staging_dir`, laid out as a prefix.
    pub(crate) fn install_into(
        self,
        build: &BuildResult,
        staging_dir: &Path,
    ) -> Result<(), InstallError> {
        match self {
            Self::Cmake(cmake) => cmake.install_into(build, staging_dir),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_and_explicit_cmake_select_cmake_and_nothing_else_does() {
        for declared in [None, Some("cmake")] {
            assert_eq!(
                BuildSystemKind::from_declared(declared),
                Some(BuildSystemKind::Cmake)
            );
        }
        for declared in ["meson", "autotools", "custom", "catalog-only", "CMake", ""] {
            assert_eq!(BuildSystemKind::from_declared(Some(declared)), None);
        }
    }
}
