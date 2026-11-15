//! Implementation of the `crux check` command.

use crate::cli::CliError;
use crate::project::check::check_project;
use std::io::Write;
use std::path::Path;

pub fn run_check<W1: Write, W2: Write>(
    project_dir: &Path,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    writeln!(stdout, "Checking Crux project...")?;
    match check_project(project_dir) {
        Ok(report) => {
            writeln!(stdout, "Manifest: crux.toml")?;
            writeln!(stdout, "Package: {}", report.package_name)?;
            writeln!(stdout, "Version: {}", report.version)?;

            if !report.dependencies.is_empty() {
                writeln!(stdout, "\nDependencies:")?;
                let max_len = report
                    .dependencies
                    .iter()
                    .map(|d| d.name().as_str().len())
                    .max()
                    .unwrap_or(0);
                for dep in &report.dependencies {
                    writeln!(
                        stdout,
                        "  {:<width$} {}",
                        dep.name().as_str(),
                        dep.version_req(),
                        width = (max_len + 2).max(8)
                    )?;
                }
                writeln!(stdout)?;
            }

            writeln!(stdout, "Status: Valid")?;
            Ok(())
        }
        Err(err) => {
            writeln!(stderr, "Manifest: crux.toml")?;
            writeln!(stderr, "Status: Invalid")?;
            writeln!(stderr, "{err}")?;
            Err(CliError::Check(err))
        }
    }
}
