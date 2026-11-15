//! Implementation of the `crux resolve` command.

use crate::cli::CliError;
use crate::cli::load_active_catalog;
use crate::project::manifest::{MANIFEST_FILENAME, Manifest};
use crate::resolution::resolver::Resolver;
use std::io::Write;
use std::path::Path;

pub fn run_resolve<W1: Write, W2: Write>(
    project_dir: &Path,
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

    let (catalog, note) = load_active_catalog(project_dir)?;

    writeln!(
        stdout,
        "Resolving dependencies for '{}' ({})...",
        manifest.package.name, manifest.package.version
    )?;
    if let Some(n) = note {
        writeln!(stdout, "{n}")?;
    }

    if manifest.dependencies.is_empty() {
        writeln!(stdout, "No dependencies declared in crux.toml.")?;
        writeln!(stdout, "Status: Success (0 packages resolved)")?;
        return Ok(());
    }

    let resolver = Resolver::new(catalog.as_ref());

    match resolver.resolve(Some(manifest.package.id()), &manifest.dependencies) {
        Ok(graph) => {
            writeln!(stdout, "\nResolved {} dependencies:", graph.package_count())?;
            for pkg in graph.packages.values() {
                writeln!(stdout, "  {} {} ({})", pkg.name, pkg.version, pkg.source)?;
            }
            writeln!(stdout, "\nStatus: Success")?;
            Ok(())
        }
        Err(err) => {
            writeln!(stderr, "\nStatus: Resolution Failed")?;
            writeln!(stderr, "error: {err}")?;
            Err(CliError::Resolution(Box::new(err)))
        }
    }
}
