//! Implementation of the `crux import` command.

use crate::cli::CliError;
use crate::cli::args::ImportArgs;
use crate::project::import::{ImportOptions, format_import_report, import_project};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn run_import<W1: Write, W2: Write>(
    project_dir: &Path,
    args: &ImportArgs,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    let target_dir: PathBuf = match &args.path {
        Some(path) => {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                project_dir.join(path)
            }
        }
        None => project_dir.to_path_buf(),
    };

    let options = ImportOptions {
        dry_run: args.dry_run,
        force: args.force,
    };

    match import_project(&target_dir, &options) {
        Ok(summary) => {
            let report = format_import_report(&summary);
            stdout.write_all(report.as_bytes())?;
            Ok(())
        }
        Err(err) => {
            writeln!(stderr, "{err}")?;
            Err(CliError::Import(err))
        }
    }
}
