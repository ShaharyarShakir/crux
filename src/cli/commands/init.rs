//! Implementation of the `crux init` command.

use crate::cli::CliError;
use crate::cli::args::InitArgs;
use crate::project::init::init_project;
use std::io::Write;
use std::path::Path;

pub fn run_init<W1: Write, W2: Write>(
    project_dir: &Path,
    args: &InitArgs,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    match init_project(project_dir, args.name.as_deref()) {
        Ok(summary) => {
            writeln!(
                stdout,
                "Initialized Crux package '{}' in {}",
                summary.name,
                summary.manifest_path.display()
            )?;
            Ok(())
        }
        Err(err) => {
            writeln!(stderr, "{err}")?;
            Err(CliError::Init(err))
        }
    }
}
