//! Implementation of the `crux lock` and `crux lock --check` commands.

use crate::cli::CliError;
use crate::cli::args::LockArgs;
use crate::cli::commands::{build_checked_lockfile, report_checksum_changes};
use crate::cli::load_active_catalog_with_options;
use crate::project::manifest::{MANIFEST_FILENAME, Manifest};
use crate::registry::GitOps;
use crate::resolution::lockfile::{
    ChecksumAudit, ChecksumFinding, FindingSeverity, LOCKFILE_FILENAME, LockError, Lockfile,
    audit_lockfile_checksums, check_lockfile_sync,
};
use crate::resolution::resolver::Resolver;
use crate::source::ActiveCatalog;
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

    let (catalog, note) =
        load_active_catalog_with_options(project_dir, &[], lock_args.update, stderr)?;

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

    let resolver = Resolver::new(&catalog);

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

    let checked = match build_checked_lockfile(
        &graph,
        existing_lockfile.as_ref(),
        &catalog,
        lock_args.update,
    ) {
        Ok(checked) => checked,
        Err(err) => {
            match &err {
                LockError::ChecksumChanged(_) => writeln!(stderr, "\nStatus: Integrity Error")?,
                _ => write!(stderr, "error constructing lockfile: ")?,
            }
            writeln!(stderr, "{err}")?;
            return Err(CliError::Lock(err));
        }
    };
    let new_lockfile = &checked.lockfile;

    if let Err(err) = new_lockfile.write_atomic(project_dir) {
        writeln!(stderr, "error: failed to write lockfile: {err}")?;
        return Err(CliError::Lock(LockError::Lockfile(err)));
    }

    report_checksum_changes(&checked, stdout, stderr)?;

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
    let (catalog, _) = match load_active_catalog_with_options(project_dir, &[], false, stderr) {
        Ok(pair) => pair,
        Err(err @ CliError::Registry(_)) => {
            writeln!(stderr, "Lockfile: crux.lock")?;
            writeln!(stderr, "Status: Unverified (registry metadata unavailable)")?;
            writeln!(
                stderr,
                "error: the active registry metadata could not be loaded, so locked checksums \
                 could not be compared. crux.lock was not modified. Restore or reconfigure the \
                 registry (see `crux registry list`), refresh it with `crux registry update \
                 <name>`, and re-run `crux lock --check`."
            )?;
            return Err(err);
        }
        Err(err) => return Err(err),
    };
    writeln!(stdout, "Checking Crux lockfile...")?;
    let sync = check_lockfile_sync(project_dir, &catalog);

    // The checksum audit needs a parseable lockfile; without one, report the
    // synchronization error exactly as before.
    let lockfile_path = project_dir.join(LOCKFILE_FILENAME);
    let lockfile = match (&sync, Lockfile::load_from_path(&lockfile_path)) {
        (_, Ok(lockfile)) => lockfile,
        (Err(_), Err(_)) => {
            let err = sync.expect_err("sync failed");
            writeln!(stderr, "Lockfile: crux.lock")?;
            writeln!(stderr, "Status: Invalid")?;
            writeln!(stderr, "{err}")?;
            return Err(CliError::Lock(err));
        }
        (Ok(_), Err(err)) => {
            writeln!(stderr, "Lockfile: crux.lock")?;
            writeln!(stderr, "Status: Invalid")?;
            writeln!(stderr, "{err}")?;
            return Err(CliError::Lock(LockError::Lockfile(err)));
        }
    };

    if let Ok(report) = &sync {
        writeln!(stdout, "Lockfile: crux.lock")?;
        writeln!(stdout, "Format version: {}", report.lockfile_version)?;
        writeln!(stdout, "Packages: {}", report.package_count)?;
    }

    // Read-only checksum audit against the locally available catalog metadata.
    // It runs even when synchronization failed, so a missing registry version
    // or a changed source is classified separately from checksum drift.
    let is_legacy = |id: &crate::package::PackageId, checksum: &crate::lock::Checksum| {
        catalog.is_legacy_git_checksum(id, checksum)
    };
    let audit = audit_lockfile_checksums(&lockfile, &catalog, &is_legacy)
        .map_err(|err| CliError::General(format!("failed to read catalog metadata: {err}")))?;
    write_checksum_audit(&audit, &catalog, stdout)?;

    let describe = |entry: &crate::lock::ChecksumAuditEntry| {
        describe_finding(&entry.package, &entry.source, &entry.finding, &catalog)
    };
    let with_severity = |severity: FindingSeverity| {
        audit
            .entries
            .iter()
            .filter(move |entry| entry.finding.severity() == severity)
    };

    if sync.is_err() || audit.has_errors() {
        writeln!(stderr, "Lockfile: crux.lock")?;
        if sync.is_ok() {
            writeln!(stderr, "Status: Invalid (checksum audit failed)")?;
        } else {
            writeln!(stderr, "Status: Invalid")?;
        }
        if let Err(err) = &sync {
            writeln!(stderr, "{err}")?;
        }
        for entry in with_severity(FindingSeverity::Error) {
            writeln!(stderr, "error: {}", describe(entry))?;
        }
        for entry in with_severity(FindingSeverity::Warning) {
            writeln!(stderr, "warning: {}", describe(entry))?;
        }
        let err = match sync {
            Err(err) => err,
            Ok(_) => {
                let err = LockError::ChecksumAuditFailed(audit.count(FindingSeverity::Error));
                writeln!(stderr, "{err}")?;
                err
            }
        };
        return Err(CliError::Lock(err));
    }

    for entry in with_severity(FindingSeverity::Warning) {
        writeln!(stderr, "warning: {}", describe(entry))?;
    }
    let report = sync.expect("sync succeeded");
    if report.is_synchronized {
        writeln!(stdout, "Status: Valid (synchronized)")?;
    } else {
        writeln!(stdout, "Status: Valid")?;
    }
    Ok(())
}

/// Writes the deterministic per-package checksum audit table and the
/// local-metadata scope note.
fn write_checksum_audit<W: Write>(
    audit: &ChecksumAudit,
    catalog: &ActiveCatalog,
    stdout: &mut W,
) -> Result<(), CliError> {
    let registry = catalog.registry();
    let revision = GitOps::head_commit(registry.path())
        .ok()
        .map(|commit| format!(", commit {}", &commit[..commit.len().min(12)]))
        .unwrap_or_default();
    writeln!(
        stdout,
        "Checksums: compared with registry '{}' (local metadata at {}{revision})",
        registry.name(),
        registry.path().display()
    )?;
    for entry in &audit.entries {
        writeln!(stdout, "  {:<22} {}", entry.finding.label(), entry.package)?;
    }
    writeln!(
        stdout,
        "Checksum audit: {} ok, {} warning(s), {} error(s)",
        audit.count(FindingSeverity::Ok),
        audit.count(FindingSeverity::Warning),
        audit.count(FindingSeverity::Error)
    )?;
    writeln!(
        stdout,
        "note: only locally available registry metadata was compared. Changes published to \
         the remote registry since it was last refreshed are not visible; refresh it (`crux \
         registry update {}`, or update the checkout yourself for CRUX_REGISTRY_PATH) and \
         re-run `crux lock --check`.",
        registry.name()
    )?;
    Ok(())
}

/// Actionable explanation of a non-`Match` finding.
fn describe_finding(
    package: &crate::package::PackageId,
    source: &crate::lock::PackageSource,
    finding: &ChecksumFinding,
    catalog: &ActiveCatalog,
) -> String {
    let registry = catalog.registry_name();
    match finding {
        ChecksumFinding::Match => format!("'{package}' matches"),
        ChecksumFinding::MutablePath => {
            format!("'{package}' is a mutable local path dependency and is not compared")
        }
        ChecksumFinding::Drift { locked, catalog } => {
            let origin = if source.is_registry() {
                "the registry metadata"
            } else {
                "the locally cached direct source"
            };
            format!(
                "checksum drift for '{package}' from '{source}': crux.lock records {locked}, but \
                 {origin} now records {catalog} for the same version and source. The content \
                 changed after you locked it. Verify the new upstream content before trusting \
                 it; if the change is legitimate, accept it explicitly with `crux lock \
                 --update`. Do not use `--update` merely to silence this error."
            )
        }
        ChecksumFinding::PackageMissing => format!(
            "'{package}' is locked from '{source}', but registry '{registry}' no longer provides \
             package '{}'. The local registry metadata may be stale or misconfigured; refresh it \
             with `crux registry update {registry}`. If the package was removed upstream, choose \
             a replacement and re-lock.",
            package.name()
        ),
        ChecksumFinding::VersionMissing { available } => format!(
            "'{package}' is locked, but registry '{registry}' does not provide version {} \
             (available: {}). Refresh the registry with `crux registry update {registry}`; if the \
             version was withdrawn, select an available version and run `crux lock --update`.",
            package.version(),
            available
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ChecksumFinding::SourceChanged { catalog_source } => format!(
            "'{package}' is locked from '{source}', but the active catalog now provides it from \
             '{catalog_source}'. This is a source identity change, not checksum drift; confirm the \
             new source is intended, then re-lock with `crux lock`."
        ),
        ChecksumFinding::CatalogChecksumUnavailable => format!(
            "registry metadata records no checksum for '{package}' ({source}), so checksum drift \
             cannot be detected from the registry for this package"
        ),
        ChecksumFinding::LockChecksumMissing { catalog } => format!(
            "crux.lock records no checksum for '{package}', while the registry records {catalog}; \
             run `crux lock` to record it"
        ),
        ChecksumFinding::LegacyFormat { locked, current } => format!(
            "crux.lock records a legacy-format (v1) source checksum for '{package}' ({locked}; \
             current format: {current}). It describes the same content and remains accepted for \
             compatibility; run `crux lock` to migrate it to the current format, which hashes \
             consistently across machines."
        ),
    }
}
