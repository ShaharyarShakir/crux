//! Implementation of the `crux fetch` command.

use crate::cli::CliError;
use crate::cli::load_active_catalog;
use crate::package::{PackageId, PackageName};
use crate::project::manifest::{MANIFEST_FILENAME, Manifest};
use crate::registry::load_config;
use crate::resolution::catalog::PackageCatalog;
use crate::resolution::lockfile::{LOCKFILE_FILENAME, LockError, LockedPackage, Lockfile};
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

    let (catalog, _catalog_note) = match load_active_catalog(project_dir, stderr) {
        Ok(pair) => pair,
        Err(err) => {
            writeln!(stderr, "{err}")?;
            return Err(err);
        }
    };
    let reg_name = catalog.registry_name().to_string();

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

        let pkg_name = catalog.resolve_name(&raw_pkg_name);

        let id = if let Some(ver_str) = ver_req {
            let version = semver::Version::parse(ver_str).map_err(|e| {
                let err = CliError::General(format!("invalid version '{ver_str}': {e}"));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            PackageId::new(pkg_name, version)
        } else {
            let entries = catalog
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

        if catalog.definition(&id).is_none() {
            let err = CliError::General(format!(
                "package definition '{id}' not found in active catalog ('{reg_name}')"
            ));
            writeln!(stderr, "{err}").ok();
            return Err(err);
        }

        writeln!(stdout, "Fetching {} from active catalog...", id)?;
        match catalog.acquire_definition(&acquirer, project_dir, &id, force) {
            Ok(acquired) => {
                writeln!(stdout, "\nSuccessfully acquired {}:", id)?;
                if let Some(archive) = acquired.archive() {
                    writeln!(stdout, "  Archive: {}", archive.display())?;
                }
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
        let packages_to_fetch: Vec<(PackageId, Option<LockedPackage>)> = if lockfile_path.is_file()
        {
            let lockfile = Lockfile::load_from_path(&lockfile_path).map_err(|e| {
                let err = CliError::Lock(LockError::Lockfile(e));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            lockfile
                .packages
                .into_iter()
                .map(|locked| (locked.id(), Some(locked)))
                .collect()
        } else {
            writeln!(
                stdout,
                "No lockfile found. Resolving dependencies for '{}'...",
                manifest.package.name
            )?;
            let resolver = Resolver::new(&catalog);
            let graph = resolver
                .resolve(Some(manifest.package.id()), &manifest.dependencies)
                .map_err(|e| {
                    let err = CliError::Resolution(Box::new(e));
                    writeln!(stderr, "{err}").ok();
                    err
                })?;
            graph.packages.values().map(|p| (p.id(), None)).collect()
        };

        if packages_to_fetch.is_empty() {
            writeln!(stdout, "No dependencies to fetch.")?;
            return Ok(());
        }

        writeln!(
            stdout,
            "Fetching {} dependencies from active catalog...",
            packages_to_fetch.len()
        )?;

        let mut acquired_list = Vec::new();
        for (id, locked) in &packages_to_fetch {
            let def = catalog.definition(id).ok_or_else(|| {
                let err = CliError::General(format!(
                    "package definition '{id}' not found in active catalog ('{reg_name}')"
                ));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            if def.metadata.id() != *id {
                return Err(CliError::General(format!(
                    "package definition '{id}' resolved to a different package: {}",
                    def.metadata.id()
                )));
            }
            writeln!(stdout, "  Fetching {}...", id)?;
            let acquired = if let Some(locked) = locked {
                catalog.acquire_locked(&acquirer, project_dir, locked, force)
            } else {
                catalog.acquire_definition(&acquirer, project_dir, id, force)
            }
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
