//! Subcommand implementations for the Crux CLI.

use crate::build::{BuildConfiguration, BuildProfile};
use crate::cli::CliError;
use crate::package::PackageId;
use crate::registry::PackageDefinition;
use std::io::Write;

pub mod add;
pub mod build;
pub mod check;
pub mod dependency_edit;
pub mod fetch;
pub mod import;
pub mod init;
pub mod install;
pub mod integrate;
pub mod lock;
pub mod registry;
pub mod remove;
pub mod resolve;
pub mod run;
pub mod toolchain;

pub use add::run_add;
pub use build::{run_build, sort_packages_topologically};
pub use check::run_check;
pub use fetch::run_fetch;
pub use import::run_import;
pub use init::run_init;
pub use install::run_install;
pub use integrate::run_integrate_cmake;
pub use lock::{run_lock, run_lock_check};
pub use registry::{run_registry_add, run_registry_list, run_registry_update};
pub use remove::run_remove;
pub use resolve::run_resolve;
pub use run::run_run;
pub use toolchain::run_toolchain;

/// Rejects packages whose declared build backend is outside the current Crux build engine.
///
/// Registry metadata intentionally records the upstream build system so catalog-only
/// packages can be discovered without pretending Crux can compile them. The current
/// build engine supports CMake only; definitions that omit `[build]` remain accepted
/// for backwards compatibility with older custom registries.
pub(crate) fn require_supported_build_system<W: Write>(
    definition: &PackageDefinition,
    package_id: &PackageId,
    stderr: &mut W,
) -> Result<(), CliError> {
    if let Some(build) = &definition.build
        && build.system != "cmake"
    {
        let err = CliError::General(format!(
            "package '{package_id}' declares build system '{}', but Crux currently supports only 'cmake'; this package is catalog-only",
            build.system
        ));
        writeln!(stderr, "{err}")?;
        return Err(err);
    }
    Ok(())
}

/// Builds the deterministic CMake configuration declared by a registry definition.
///
/// Options are validated and sorted when the registry is loaded, so the same
/// definition always produces the same fingerprint regardless of TOML ordering.
pub(crate) fn build_configuration_for(
    definition: &PackageDefinition,
    profile: BuildProfile,
) -> BuildConfiguration {
    let mut config = BuildConfiguration::with_profile(profile);
    if let Some(build) = &definition.build {
        config.cmake_args = build.cmake_args.clone();
    }
    config
}
