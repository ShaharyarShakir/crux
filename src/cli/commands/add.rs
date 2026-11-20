//! Implementation of the `crux add <package>[@<constraint>]` command.
//!
//! # Behaviour
//!
//! 1. Load and validate `crux.toml`.
//! 2. Parse the package spec into a name and optional SemVer constraint.
//!    When no constraint is supplied the default `"*"` (any version) is used,
//!    consistent with how the existing resolver interprets a wildcard requirement.
//! 3. Reject duplicate direct dependencies.
//! 4. Resolve the full dependency graph (including transitive deps) using the
//!    existing deterministic `Resolver`, reusing existing lockfile preferences
//!    so pinned versions are not needlessly updated.
//! 5. Build the updated manifest content without disturbing unrelated
//!    formatting or comments (`[dependencies]` only).
//! 6. Write `crux.toml` and `crux.lock` transactionally: the lockfile is
//!    constructed in memory first, and the original manifest is restored if the
//!    lockfile write fails.
//! 7. Run the existing install pipeline so the new dependency is immediately
//!    available on disk (Debug profile, matching the default application build).

use crate::build::BuildProfile;
use crate::cli::CliError;
use crate::cli::args::AddArgs;
use crate::cli::commands::dependency_edit::commit_manifest_and_lockfile;
use crate::cli::commands::install::run_install;
use crate::cli::load_catalog_for_dependency_edits;
use crate::package::Dependency;
use crate::project::manifest::{MANIFEST_FILENAME, Manifest, upsert_dependency_in_toml};
use crate::resolution::lockfile::{LOCKFILE_FILENAME, Lockfile};
use crate::resolution::resolver::Resolver;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Default version constraint applied when the user omits one.
/// `"*"` matches any version, identical to the resolver's wildcard behaviour.
pub const DEFAULT_VERSION_CONSTRAINT: &str = "*";

/// Parses a package spec of the form `name` or `name@constraint` into
/// `(name_str, constraint_str)`.
pub(crate) fn parse_package_spec(spec: &str) -> (&str, &str) {
    if let Some(at) = spec.find('@') {
        (&spec[..at], &spec[at + 1..])
    } else {
        (spec, DEFAULT_VERSION_CONSTRAINT)
    }
}

pub fn run_add<W1: Write, W2: Write>(
    project_dir: &Path,
    args: &AddArgs,
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

    // ── 2. Parse package spec ─────────────────────────────────────────────
    let (name_str, constraint_str) = parse_package_spec(&args.spec);
    let constraint_display = if constraint_str == DEFAULT_VERSION_CONSTRAINT {
        "*  (any version — override with crux add name@constraint)".to_string()
    } else {
        constraint_str.to_string()
    };

    writeln!(stdout, "Adding dependency: {name_str} {constraint_display}")?;

    let new_dep = match Dependency::try_new(name_str, constraint_str) {
        Ok(d) => d,
        Err(err) => {
            writeln!(stderr, "error: invalid package spec '{}': {err}", args.spec)?;
            return Err(CliError::General(format!(
                "invalid package spec '{}': {err}",
                args.spec
            )));
        }
    };

    // ── 3. Reject duplicates ──────────────────────────────────────────────
    if manifest
        .dependencies
        .iter()
        .any(|d| d.name() == new_dep.name())
    {
        let msg = format!(
            "dependency '{}' is already declared in crux.toml; \
             use 'crux remove {}' first if you want to change the constraint",
            name_str, name_str
        );
        writeln!(stderr, "error: {msg}")?;
        return Err(CliError::General(msg));
    }

    // ── 4. Resolve the full graph ─────────────────────────────────────────
    let (catalog, reg_note) = load_catalog_for_dependency_edits(project_dir, stderr)?;
    if let Some(note) = reg_note {
        writeln!(stdout, "{note}")?;
    }

    // Build tentative dependency list (existing + new)
    let mut tentative_deps = manifest.dependencies.clone();
    tentative_deps.push(new_dep.clone());
    tentative_deps.sort();

    // Reuse existing lockfile preferences to preserve pinned versions
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

    writeln!(stdout, "\nResolving dependencies...")?;
    let resolver = Resolver::new(catalog.as_ref());
    let graph = match resolver.resolve_with_preferences(
        Some(manifest.package.id()),
        &tentative_deps,
        &preferences,
    ) {
        Ok(g) => g,
        Err(err) => {
            writeln!(stderr, "\nResolution failed: {err}")?;
            return Err(CliError::Resolution(Box::new(err)));
        }
    };

    // ── 5. Build the updated lockfile in memory before touching the disk ─
    let new_lockfile = match graph.to_lockfile_with_existing(existing_lockfile.as_ref()) {
        Ok(lf) => lf,
        Err(err) => {
            writeln!(stderr, "error constructing lockfile: {err}")?;
            return Err(CliError::General(format!(
                "lockfile construction failed: {err}"
            )));
        }
    };

    // ── 6. Render and validate the updated `crux.toml` ────────────────────
    // Only the `[dependencies]` table is touched so comments, ordering, and
    // unrelated formatting survive the edit.
    let updated_content = match upsert_dependency_in_toml(&original_content, &new_dep) {
        Ok(content) => content,
        Err(err) => {
            writeln!(stderr, "error: failed to update crux.toml: {err}")?;
            return Err(CliError::Io(err.to_string()));
        }
    };

    if let Err(err) = Manifest::from_toml_str(&updated_content) {
        writeln!(stderr, "error: generated manifest is invalid: {err}")?;
        return Err(CliError::Io(err.to_string()));
    }

    // ── 7. Write manifest then lockfile, rolling back on failure ──────────
    commit_manifest_and_lockfile(
        project_dir,
        &manifest_path,
        &original_content,
        &updated_content,
        &new_lockfile,
        stdout,
        stderr,
    )?;

    // ── 8. Install dependencies (Debug, matching the default app build) ───
    writeln!(stdout, "\nInstalling dependencies...")?;
    run_install(
        project_dir,
        None, // install all locked packages
        BuildProfile::Debug,
        false, // no --force
        false, // no --locked (we just wrote the lock)
        &mut *stdout,
        &mut *stderr,
    )?;

    writeln!(stdout, "\nDone. '{name_str}' added successfully.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_package_spec_name_only() {
        let (name, constraint) = parse_package_spec("fmt");
        assert_eq!(name, "fmt");
        assert_eq!(constraint, DEFAULT_VERSION_CONSTRAINT);
    }

    #[test]
    fn test_parse_package_spec_with_constraint() {
        let (name, constraint) = parse_package_spec("fmt@^11.0");
        assert_eq!(name, "fmt");
        assert_eq!(constraint, "^11.0");
    }

    #[test]
    fn test_parse_package_spec_with_range() {
        let (name, constraint) = parse_package_spec("zlib@>=1.2, <2.0");
        assert_eq!(name, "zlib");
        assert_eq!(constraint, ">=1.2, <2.0");
    }

    #[test]
    fn test_parse_package_spec_tilde() {
        let (name, constraint) = parse_package_spec("openssl@~3.4");
        assert_eq!(name, "openssl");
        assert_eq!(constraint, "~3.4");
    }
}
