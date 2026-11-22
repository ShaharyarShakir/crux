//! Subcommand implementations for the Crux CLI.

use crate::build::{BuildConfiguration, BuildProfile};
use crate::cli::CliError;
use crate::package::PackageId;
use crate::registry::PackageDefinition;
use crate::resolution::lockfile::{LockError, Lockfile};
use crate::resolution::resolver::{
    CheckedLockfile, ChecksumChangeReason, ChecksumPolicy, ResolvedGraph,
};
use crate::source::{ActiveCatalog, LockedChecksumStatus};
use std::collections::BTreeSet;
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

/// Regenerates a lockfile from `graph` without silently replacing existing
/// checksums. `accept_changes` must only be `true` for `crux lock --update`.
pub(crate) fn build_checked_lockfile(
    graph: &ResolvedGraph,
    existing: Option<&Lockfile>,
    catalog: &ActiveCatalog,
    accept_changes: bool,
) -> Result<CheckedLockfile, LockError> {
    let is_legacy = |id: &PackageId, checksum: &crate::lock::Checksum| {
        catalog.is_legacy_git_checksum(id, checksum)
    };
    let policy = ChecksumPolicy {
        accept_changes,
        is_legacy_equivalent: &is_legacy,
    };
    graph.to_lockfile_checked(existing, &policy)
}

/// Reports every checksum replacement made while regenerating a lockfile.
///
/// Legacy-format migrations are informational (stdout). Explicitly accepted
/// content changes are security-relevant and reported as warnings (stderr).
pub(crate) fn report_checksum_changes<W1: Write, W2: Write>(
    checked: &CheckedLockfile,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    for change in &checked.checksum_changes {
        match change.reason {
            ChecksumChangeReason::LegacyFormatMigrated => writeln!(
                stdout,
                "Migrated legacy checksum for '{}' ({}) to the current format:\n  old: {}\n  new: {}",
                change.package, change.source, change.old, change.new
            )?,
            ChecksumChangeReason::ExplicitUpdate => writeln!(
                stderr,
                "warning: accepted a changed checksum for '{}' from '{}' because `crux lock --update` was requested explicitly:\n  old: {}\n  new: {}",
                change.package, change.source, change.old, change.new
            )?,
        }
    }
    Ok(())
}

/// Warns (once per package per command) when a locked Git checksum was only
/// accepted through the legacy `crux-source-tree-v1` compatibility path.
pub(crate) fn warn_legacy_locked_checksum<W: Write>(
    stderr: &mut W,
    id: &PackageId,
    status: LockedChecksumStatus,
    warned: &mut BTreeSet<PackageId>,
) -> Result<(), CliError> {
    if status == LockedChecksumStatus::LegacyFormat && warned.insert(id.clone()) {
        writeln!(
            stderr,
            "warning: crux.lock records a legacy-format (v1) source checksum for '{id}'. \
             It matched and was accepted for compatibility with Crux 0.2.0/0.2.1, but the \
             current format hashes consistently across machines. Run `crux lock` to migrate it."
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::PackageName;

    #[test]
    fn legacy_checksum_warning_is_emitted_once_per_package() {
        let id = PackageId::new(
            PackageName::new("pinned").unwrap(),
            semver::Version::new(1, 0, 0),
        );
        let other = PackageId::new(
            PackageName::new("other").unwrap(),
            semver::Version::new(2, 0, 0),
        );
        let mut warned = BTreeSet::new();
        let mut out = Vec::new();
        for _ in 0..3 {
            warn_legacy_locked_checksum(
                &mut out,
                &id,
                LockedChecksumStatus::LegacyFormat,
                &mut warned,
            )
            .unwrap();
        }
        for status in [
            LockedChecksumStatus::Verified,
            LockedChecksumStatus::NotRecorded,
            LockedChecksumStatus::MutablePath,
        ] {
            warn_legacy_locked_checksum(&mut out, &other, status, &mut warned).unwrap();
        }
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.matches("warning:").count(), 1, "{text}");
        assert!(text.contains("'pinned 1.0.0'"));
        assert!(text.contains("Run `crux lock` to migrate it"));
    }
}
