//! Implementation of the `crux init` command.

use crate::cli::CliError;
use crate::cli::args::InitArgs;
use crate::project::init::{InitOptions, init_project_with_options};
use crate::project::manifest::ProjectLanguage;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

pub fn run_init<W1: Write, W2: Write>(
    project_dir: &Path,
    args: &InitArgs,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    // Determine target directory
    let target_dir: PathBuf = match &args.path {
        Some(p) => {
            let path = Path::new(p);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                project_dir.join(path)
            }
        }
        None => project_dir.to_path_buf(),
    };

    // Determine project language
    let language: ProjectLanguage = match &args.language {
        Some(lang_str) => match lang_str.parse::<ProjectLanguage>() {
            Ok(lang) => lang,
            Err(e) => {
                writeln!(stderr, "error: {e}")?;
                return Err(CliError::General(format!("error: {e}")));
            }
        },
        None => {
            if std::io::stdin().is_terminal() {
                // Interactive selection prompt
                writeln!(stdout, "Select project language:")?;
                writeln!(stdout, "  1) C")?;
                writeln!(stdout, "  2) C++ (default)")?;
                writeln!(stdout, "  3) Other")?;
                write!(stdout, "Enter choice [1-3, default: 2]: ")?;
                stdout.flush()?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                match input.trim().to_ascii_lowercase().as_str() {
                    "" | "2" | "cpp" | "c++" => ProjectLanguage::Cpp,
                    "1" | "c" => ProjectLanguage::C,
                    "3" | "other" => ProjectLanguage::Other,
                    other => {
                        let err_msg = format!(
                            "invalid language choice '{other}'; must be 1 (C), 2 (C++), or 3 (Other)"
                        );
                        writeln!(stderr, "error: {err_msg}")?;
                        return Err(CliError::General(err_msg));
                    }
                }
            } else {
                // Non-interactive environment: check if input is piped
                let mut input = String::new();
                if std::io::stdin().read_line(&mut input).is_ok() && !input.trim().is_empty() {
                    match input.trim().to_ascii_lowercase().as_str() {
                        "1" | "c" => ProjectLanguage::C,
                        "2" | "cpp" | "c++" => ProjectLanguage::Cpp,
                        "3" | "other" => ProjectLanguage::Other,
                        other => {
                            let err_msg = format!(
                                "invalid language choice '{other}'; must be 1 (C), 2 (C++), or 3 (Other)"
                            );
                            writeln!(stderr, "error: {err_msg}")?;
                            return Err(CliError::General(err_msg));
                        }
                    }
                } else {
                    // Stdin is EOF / empty; default to C++
                    ProjectLanguage::Cpp
                }
            }
        }
    };

    let options = InitOptions {
        custom_name: args.name.as_deref(),
        language: Some(language),
        standard: args.standard.as_deref(),
    };

    match init_project_with_options(&target_dir, &options) {
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
