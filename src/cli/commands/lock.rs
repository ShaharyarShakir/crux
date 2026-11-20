//! Implementation of the `crux lock` and `crux lock --check` commands.

use crate::cli::CliError;
use crate::cli::args::LockArgs;
use crate::cli::load_active_catalog;
use crate::project::manifest::{MANIFEST_FILENAME, Manifest};
use crate::resolution::lockfile::{
    LOCKFILE_FILENAME, LockError, Lockfile, LockfileError, check_lockfile_sync,
};
use crate::resolution::resolver::Resolver;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

pub fn run_lock<W1: Write, W2: Write>(
    project_dir: &Path,
    lock_args: &LockArgs,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    let manifest_path = project_dir.join(MANIFEST_FILENAME);
    if !manifest_path.is_file() {
        writeln!(
            stderr,
            "error: manifest not found at '{}'",
            manifest_path.display()
        )?;
        return Err(CliError::Io(format!(
            "manifest not found at '{}'",
            manifest_path.display()
        )));
    }

    let manifest = match Manifest::load_from_path(&manifest_path) {
        Ok(m) => m,
        Err(err) => {
            writeln!(stderr, "{err}")?;
            return Err(CliError::Io(err.to_string()));
        }
    };

    let (catalog, note) = load_active_catalog(project_dir, stderr)?;

    writeln!(
        stdout,
        "Resolving dependencies for '{}' ({})...",
        manifest.package.name, manifest.package.version
    )?;
    if let Some(n) = note {
        writeln!(stdout, "{n}")?;
    }

    let lockfile_path = project_dir.join(LOCKFILE_FILENAME);
    let existing_lockfile = if lockfile_path.is_file() {
        match Lockfile::load_from_path(&lockfile_path) {
            Ok(lf) => Some(lf),
            Err(err) => {
                writeln!(stderr, "error reading existing lockfile: {err}")?;
                return Err(CliError::Lock(LockError::Lockfile(err)));
            }
        }
    } else {
        None
    };

    let mut preferences = BTreeMap::new();
    if !lock_args.update
        && let Some(ref lf) = existing_lockfile
    {
        for pkg in &lf.packages {
            preferences.insert(pkg.name.clone(), pkg.version.clone());
        }
    }

    let resolver = Resolver::new(catalog.as_ref());

    let graph = match resolver.resolve_with_preferences(
        Some(manifest.package.id()),
        &manifest.dependencies,
        &preferences,
    ) {
        Ok(g) => g,
        Err(err) => {
            writeln!(stderr, "\nStatus: Resolution Failed")?;
            writeln!(stderr, "error: {err}")?;
            return Err(CliError::Resolution(Box::new(err)));
        }
    };

    let new_lockfile = match graph.to_lockfile_with_existing(existing_lockfile.as_ref()) {
        Ok(lf) => lf,
        Err(err) => {
            writeln!(stderr, "error constructing lockfile: {err}")?;
            return Err(CliError::Lock(LockError::Lockfile(
                LockfileError::Validation(err),
            )));
        }
    };

    if let Err(err) = new_lockfile.write_atomic(project_dir) {
        writeln!(stderr, "error: failed to write lockfile: {err}")?;
        return Err(CliError::Lock(LockError::Lockfile(err)));
    }

    if existing_lockfile.is_some() {
        if lock_args.update {
            writeln!(
                stdout,
                "\nUpdated crux.lock ({} packages locked)",
                new_lockfile.package_count()
            )?;
        } else {
            writeln!(
                stdout,
                "\nSynchronized crux.lock ({} packages locked)",
                new_lockfile.package_count()
            )?;
        }
    } else {
        writeln!(
            stdout,
            "\nGenerated crux.lock ({} packages locked)",
            new_lockfile.package_count()
        )?;
    }

    for pkg in &new_lockfile.packages {
        writeln!(stdout, "  {} {} ({})", pkg.name, pkg.version, pkg.source)?;
    }
    writeln!(stdout, "\nStatus: Success")?;
    Ok(())
}

pub fn run_lock_check<W1: Write, W2: Write>(
    project_dir: &Path,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    let (catalog, _) = load_active_catalog(project_dir, stderr)?;
    writeln!(stdout, "Checking Crux lockfile...")?;
    match check_lockfile_sync(project_dir, catalog.as_ref()) {
        Ok(report) => {
            writeln!(stdout, "Lockfile: crux.lock")?;
            writeln!(stdout, "Format version: {}", report.lockfile_version)?;
            writeln!(stdout, "Packages: {}", report.package_count)?;
            if report.is_synchronized {
                writeln!(stdout, "Status: Valid (synchronized)")?;
            } else {
                writeln!(stdout, "Status: Valid")?;
            }
            Ok(())
        }
        Err(err) => {
            writeln!(stderr, "Lockfile: crux.lock")?;
            writeln!(stderr, "Status: Invalid")?;
            writeln!(stderr, "{err}")?;
            Err(CliError::Lock(err))
        }
    }
}
