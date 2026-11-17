//! Implementation of `crux registry` subcommands (list, add, update).

use crate::cli::CliError;
use crate::registry::{add_registry, list_registries, update_registry};
use std::io::Write;
use std::path::Path;

pub fn run_registry_list<W: Write>(
    project_dir: Option<&Path>,
    stdout: &mut W,
) -> Result<(), CliError> {
    let list = list_registries(project_dir)?;
    if list.is_empty() {
        writeln!(stdout, "No package registries configured.")?;
    } else {
        writeln!(stdout, "Configured registries ({}):", list.len())?;
        for item in list {
            writeln!(stdout, "  {}", item.name)?;
            if let Some(ref url) = item.config.url {
                writeln!(stdout, "    URL:      {}", url)?;
            }
            if let Some(ref subdir) = item.config.subdir {
                writeln!(stdout, "    Subdir:   {}", subdir.display())?;
            }
            writeln!(stdout, "    Path:     {}", item.config.path.display())?;
            match item.registry {
                Ok(reg) => {
                    writeln!(stdout, "    Packages: {}", reg.package_count())?;
                    writeln!(stdout, "    Versions: {}", reg.total_version_count())?;
                    writeln!(stdout, "    Status:   Valid")?;
                }
                Err(err) => {
                    writeln!(stdout, "    Status:   Error ({err})")?;
                }
            }
        }
    }
    Ok(())
}

pub fn run_registry_add<W1: Write, W2: Write>(
    project_dir: Option<&Path>,
    name: &str,
    url: &str,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    writeln!(stdout, "Adding registry '{name}' from '{url}'...")?;
    match add_registry(name, url, project_dir) {
        Ok(reg) => {
            writeln!(stdout, "\nSuccessfully added registry '{name}'")?;
            writeln!(stdout, "  Path:     {}", reg.path().display())?;
            writeln!(stdout, "  Packages: {}", reg.package_count())?;
            writeln!(stdout, "  Versions: {}", reg.total_version_count())?;
            Ok(())
        }
        Err(err) => {
            writeln!(stderr, "error: failed to add registry '{name}': {err}")?;
            Err(CliError::Registry(Box::new(err)))
        }
    }
}

pub fn run_registry_update<W1: Write, W2: Write>(
    project_dir: Option<&Path>,
    name: &str,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    writeln!(stdout, "Updating registry '{name}'...")?;
    match update_registry(name, project_dir) {
        Ok((reg, head)) => {
            writeln!(stdout, "\nSuccessfully updated registry '{name}'")?;
            writeln!(stdout, "  Head:     {head}")?;
            writeln!(stdout, "  Packages: {}", reg.package_count())?;
            writeln!(stdout, "  Versions: {}", reg.total_version_count())?;
            Ok(())
        }
        Err(err) => {
            writeln!(stderr, "error: failed to update registry '{name}': {err}")?;
            Err(CliError::Registry(Box::new(err)))
        }
    }
}
