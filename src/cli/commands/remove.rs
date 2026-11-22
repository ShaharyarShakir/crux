//! Implementation of the `crux remove <package>` command.
//!
//! # Behaviour
//!
//! 1. Load and validate `crux.toml`.
//! 2. Locate the named package in the direct dependency list.
//! 3. Re-resolve the remaining dependency graph *before* writing anything, so a
//!    failed resolution never leaves the project in an inconsistent state.
//! 4. Construct the updated lockfile in memory.
//! 5. Remove only the direct dependency entry from `crux.toml`, preserving
//!    comments, ordering, and unrelated formatting.
//! 6. Write `crux.toml` then `crux.lock`, restoring the original manifest if the
//!    lockfile write fails.
//!
//! Shared transitive dependencies that are still required remain in the
//! lockfile. Installed files are deliberately left untouched: cleaning the
//! installation cache is out of scope for dependency editing and Crux never
//! deletes files it cannot confidently attribute to itself.

use crate::cli::CliError;
use crate::cli::args::RemoveArgs;
use crate::cli::commands::dependency_edit::commit_manifest_and_lockfile;
use crate::cli::commands::{build_checked_lockfile, report_checksum_changes};
use crate::cli::load_catalog_for_dependency_edits;
use crate::package::Dependency;
use crate::project::manifest::{MANIFEST_FILENAME, Manifest, remove_dependency_from_toml};
use crate::resolution::lockfile::{LOCKFILE_FILENAME, Lockfile};
use crate::resolution::resolver::Resolver;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn run_remove<W1: Write, W2: Write>(
    project_dir: &Path,
    args: &RemoveArgs,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    // ── 1. Load manifest ──────────────────────────────────────────────────
    let manifest_path = project_dir.join(MANIFEST_FILENAME);
    if !manifest_path.is_file() {
        let msg = format!(
            "manifest not found at '{}'; run 'crux init' to initialize a project",
            manifest_path.display()
        );
        writeln!(stderr, "error: {msg}")?;
        return Err(CliError::General(msg));
    }

    let original_content = match fs::read_to_string(&manifest_path) {
        Ok(content) => content,
        Err(err) => {
            writeln!(stderr, "error: failed to read manifest: {err}")?;
            return Err(CliError::Io(err.to_string()));
        }
    };

    let manifest = match Manifest::load_from_path(&manifest_path) {
        Ok(m) => m,
        Err(err) => {
            writeln!(stderr, "error: failed to load manifest: {err}")?;
            return Err(CliError::Io(err.to_string()));
        }
    };

    // ── 2. Locate the dependency ──────────────────────────────────────────
    let pkg_name = args.name.trim();
    let pos = manifest
        .dependencies
        .iter()
        .position(|d| d.name().as_str() == pkg_name);

    let pos = match pos {
        Some(p) => p,
        None => {
            let msg = format!("dependency '{pkg_name}' is not declared in crux.toml");
            writeln!(stderr, "error: {msg}")?;
            if !manifest.dependencies.is_empty() {
                let names: Vec<_> = manifest
                    .dependencies
                    .iter()
                    .map(|d| d.name().as_str())
                    .collect();
                writeln!(stderr, "  declared dependencies: {}", names.join(", "))?;
            } else {
                writeln!(stderr, "  no dependencies are currently declared")?;
            }
            return Err(CliError::General(msg));
        }
    };

    writeln!(stdout, "Removing dependency: {pkg_name}")?;

    // ── 3. Re-resolve the remaining dependency graph ──────────────────────
    let mut remaining_deps: Vec<Dependency> = manifest.dependencies.clone();
    remaining_deps.remove(pos);

    let (catalog, reg_note) =
        load_catalog_for_dependency_edits(project_dir, &remaining_deps, stderr)?;
    if let Some(note) = reg_note {
        writeln!(stdout, "{note}")?;
    }

    // Reuse existing lockfile preferences to avoid unnecessary version churn.
    let lockfile_path = project_dir.join(LOCKFILE_FILENAME);
    let existing_lockfile = if lockfile_path.is_file() {
        Lockfile::load_from_path(&lockfile_path).ok()
    } else {
        None
    };
    let mut preferences = BTreeMap::new();
    if let Some(ref lf) = existing_lockfile {
        for pkg in &lf.packages {
            preferences.insert(pkg.name.clone(), pkg.version.clone());
        }
    }

    writeln!(stdout, "\nRe-resolving remaining dependencies...")?;
    let resolver = Resolver::new(&catalog);
    let graph = match resolver.resolve_with_preferences(
        Some(manifest.package.id()),
        &remaining_deps,
        &preferences,
    ) {
        Ok(g) => g,
        Err(err) => {
            writeln!(stderr, "\nResolution failed: {err}")?;
            return Err(CliError::Resolution(Box::new(err)));
        }
    };

    // ── 4. Build the updated lockfile in memory ───────────────────────────
    let new_lockfile =
        match build_checked_lockfile(&graph, existing_lockfile.as_ref(), &catalog, false) {
            Ok(checked) => {
                report_checksum_changes(&checked, stdout, stderr)?;
                checked.lockfile
            }
            Err(err) => {
                if !matches!(
                    err,
                    crate::resolution::lockfile::LockError::ChecksumChanged(_)
                ) {
                    write!(stderr, "error constructing lockfile: ")?;
                }
                writeln!(stderr, "{err}")?;
                return Err(CliError::Lock(err));
            }
        };

    // ── 5. Render and validate the updated `crux.toml` ────────────────────
    let updated_content = match remove_dependency_from_toml(&original_content, pkg_name) {
        Ok(Some(content)) => content,
        Ok(None) => {
            // The parsed manifest contained the dependency, so this only
            // happens when the raw text is older/different than expected.
            let msg = format!(
                "dependency '{pkg_name}' was found in the parsed manifest but not in the \
                 [dependencies] table of crux.toml; refusing to rewrite it"
            );
            writeln!(stderr, "error: {msg}")?;
            return Err(CliError::General(msg));
        }
        Err(err) => {
            writeln!(stderr, "error: failed to update crux.toml: {err}")?;
            return Err(CliError::Io(err.to_string()));
        }
    };

    if let Err(err) = Manifest::from_toml_str(&updated_content) {
        writeln!(stderr, "error: generated manifest is invalid: {err}")?;
        return Err(CliError::Io(err.to_string()));
    }

    // ── 6. Write manifest then lockfile, rolling back on failure ──────────
    commit_manifest_and_lockfile(
        project_dir,
        &manifest_path,
        &original_content,
        &updated_content,
        &new_lockfile,
        stdout,
        stderr,
    )?;

    if !remaining_deps.is_empty() {
        writeln!(
            stdout,
            "\nRemaining direct dependencies: {}",
            remaining_deps
                .iter()
                .map(|d| d.name().as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }

    writeln!(
        stdout,
        "Installed files for '{pkg_name}' are left in place; run \
         'crux install --force' to refresh remaining dependencies."
    )?;

    writeln!(stdout, "\nDone. '{pkg_name}' removed successfully.")?;
    Ok(())
}
