//! Implementation of the `crux fetch` command.

use crate::cli::CliError;
use crate::cli::load_active_registry;
use crate::package::{PackageId, PackageName};
use crate::project::manifest::{MANIFEST_FILENAME, Manifest};
use crate::registry::load_config;
use crate::resolution::catalog::PackageCatalog;
use crate::resolution::lockfile::{LOCKFILE_FILENAME, LockError, Lockfile};
use crate::resolution::resolver::Resolver;
use crate::source::{AcquisitionError, SourceAcquirer, SourceCache};
use std::io::Write;
use std::path::Path;

pub fn run_fetch<W: Write, E: Write>(
    project_dir: &Path,
    package_spec: Option<&str>,
    force: bool,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    let config = match load_config(Some(project_dir)) {
        Ok(cfg) => cfg,
        Err(err) => {
            let cli_err = CliError::Registry(Box::new(err));
            writeln!(stderr, "{cli_err}").ok();
            return Err(cli_err);
        }
    };
    let cache = SourceCache::from_config(&config).map_err(AcquisitionError::Io)?;
    let acquirer = SourceAcquirer::new(cache);

    let (registry, reg_name) = match load_active_registry(project_dir) {
        Ok(pair) => pair,
        Err(err) => {
            writeln!(stderr, "{err}")?;
            return Err(err);
        }
    };

    if let Some(spec) = package_spec {
        let (name_str, ver_req) = if let Some((n, v)) = spec.split_once('@') {
            (n, Some(v))
        } else {
            (spec, None)
        };

        let raw_pkg_name = PackageName::new(name_str).map_err(|e| {
            let err = CliError::General(format!("invalid package name '{name_str}': {e}"));
            writeln!(stderr, "{err}").ok();
            err
        })?;

        let pkg_name = if registry.package_names().contains(&raw_pkg_name) {
            raw_pkg_name
        } else if let Some(matching) = registry
            .package_names()
            .iter()
            .find(|p| p.as_str().eq_ignore_ascii_case(raw_pkg_name.as_str()))
        {
            matching.clone()
        } else {
            raw_pkg_name
        };

        let id = if let Some(ver_str) = ver_req {
            let version = semver::Version::parse(ver_str).map_err(|e| {
                let err = CliError::General(format!("invalid version '{ver_str}': {e}"));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            PackageId::new(pkg_name, version)
        } else {
            let entries = registry
                .get_package_versions(&pkg_name)
                .map_err(|e| CliError::General(format!("error querying package versions: {e}")))?;
            if entries.is_empty() {
                let err = CliError::General(format!(
                    "package '{pkg_name}' not found in registry '{reg_name}'"
                ));
                writeln!(stderr, "{err}")?;
                return Err(err);
            }
            let highest = entries.last().unwrap().version.clone();
            PackageId::new(pkg_name, highest)
        };

        let def = registry.get_package_definition(&id).ok_or_else(|| {
            let err = CliError::General(format!(
                "package definition '{id}' not found in registry '{reg_name}'"
            ));
            writeln!(stderr, "{err}").ok();
            err
        })?;

        writeln!(stdout, "Fetching {} from registry '{}'...", id, reg_name)?;
        match acquirer.acquire_package_definition(&id, def, force) {
            Ok(acquired) => {
                writeln!(stdout, "\nSuccessfully acquired {}:", id)?;
                writeln!(stdout, "  Archive: {}", acquired.archive_path.display())?;
                writeln!(stdout, "  Source:  {}", acquired.source_dir.display())?;
                writeln!(stdout, "  SHA-256: {}", acquired.checksum)?;
                Ok(())
            }
            Err(err) => {
                writeln!(stderr, "error: failed to acquire package '{id}': {err}")?;
                Err(CliError::Acquisition(Box::new(err)))
            }
        }
    } else {
        let manifest_path = project_dir.join(MANIFEST_FILENAME);
        if !manifest_path.is_file() {
            let err = CliError::General(format!(
                "manifest not found at '{}'",
                manifest_path.display()
            ));
            writeln!(stderr, "{err}")?;
            return Err(err);
        }

        let manifest = Manifest::load_from_path(&manifest_path).map_err(|e| {
            let err = CliError::Io(e.to_string());
            writeln!(stderr, "{err}").ok();
            err
        })?;

        let lockfile_path = project_dir.join(LOCKFILE_FILENAME);
        let packages_to_fetch: Vec<PackageId> = if lockfile_path.is_file() {
            let lockfile = Lockfile::load_from_path(&lockfile_path).map_err(|e| {
                let err = CliError::Lock(LockError::Lockfile(e));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            lockfile.packages.into_iter().map(|p| p.id()).collect()
        } else {
            writeln!(
                stdout,
                "No lockfile found. Resolving dependencies for '{}'...",
                manifest.package.name
            )?;
            let resolver = Resolver::new(&registry);
            let graph = resolver
                .resolve(Some(manifest.package.id()), &manifest.dependencies)
                .map_err(|e| {
                    let err = CliError::Resolution(Box::new(e));
                    writeln!(stderr, "{err}").ok();
                    err
                })?;
            graph.packages.values().map(|p| p.id()).collect()
        };

        if packages_to_fetch.is_empty() {
            writeln!(stdout, "No dependencies to fetch.")?;
            return Ok(());
        }

        writeln!(
            stdout,
            "Fetching {} dependencies from registry '{}'...",
            packages_to_fetch.len(),
            reg_name
        )?;

        let mut acquired_list = Vec::new();
        for id in &packages_to_fetch {
            let def = registry.get_package_definition(id).ok_or_else(|| {
                let err = CliError::General(format!(
                    "package definition '{id}' not found in registry '{reg_name}'"
                ));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            writeln!(stdout, "  Fetching {}...", id)?;
            let acquired = acquirer
                .acquire_package_definition(id, def, force)
                .map_err(|e| {
                    let err = CliError::Acquisition(Box::new(e));
                    writeln!(stderr, "error: failed to acquire '{id}': {err}").ok();
                    err
                })?;
            acquired_list.push(acquired);
        }

        writeln!(
            stdout,
            "\nSuccessfully acquired {} dependencies:",
            acquired_list.len()
        )?;
        for acq in &acquired_list {
            writeln!(
                stdout,
                "  {} -> {}",
                acq.package_id,
                acq.source_dir.display()
            )?;
        }
        Ok(())
    }
}
