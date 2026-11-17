//! Implementation of the `crux toolchain` command.

use crate::build::toolchain::{PlatformInfo, detect_compiler, probe_standard_support};
use crate::cli::CliError;
use crate::cli::args::ToolchainArgs;
use crate::project::manifest::ProjectLanguage;
use std::io::Write;

pub fn run_toolchain<W1: Write, W2: Write>(
    args: &ToolchainArgs,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<(), CliError> {
    let platform = PlatformInfo::current();

    let target_languages = match &args.language {
        Some(lang_str) => {
            let lang = match lang_str.parse::<ProjectLanguage>() {
                Ok(l) => l,
                Err(e) => {
                    writeln!(stderr, "error: {e}")?;
                    return Err(CliError::General(format!("error: {e}")));
                }
            };
            vec![lang]
        }
        None => vec![ProjectLanguage::C, ProjectLanguage::Cpp],
    };

    writeln!(stdout, "Crux Toolchain Information\n")?;
    writeln!(stdout, "Platform:")?;
    writeln!(stdout, "  OS: {}", platform.os)?;
    writeln!(stdout, "  Architecture: {}\n", platform.arch)?;

    let mut had_error = false;

    for lang in target_languages {
        match lang {
            ProjectLanguage::C => {
                writeln!(stdout, "C Toolchain:")?;
                match detect_compiler(ProjectLanguage::C) {
                    Ok(info) => {
                        writeln!(stdout, "  Compiler: {}", info.family)?;
                        writeln!(stdout, "  Version: {}", info.version)?;
                        writeln!(stdout, "  Target: {}", info.target_triple)?;
                        writeln!(stdout, "  Executable: {}", info.path.display())?;

                        let c17_supported = probe_standard_support(&info, "c17").unwrap_or(false);
                        let c23_supported = probe_standard_support(&info, "c23").unwrap_or(false);

                        let c17_status = if c17_supported {
                            "supported"
                        } else {
                            "unsupported"
                        };
                        let c23_status = if c23_supported {
                            "supported"
                        } else {
                            "unsupported"
                        };
                        writeln!(
                            stdout,
                            "  Standards: C17 ({c17_status}), C23 ({c23_status})\n"
                        )?;
                    }
                    Err(err) => {
                        writeln!(stdout, "  Status: Not available ({err})\n")?;
                        if args.language.is_some() {
                            had_error = true;
                            writeln!(stderr, "{err}")?;
                        }
                    }
                }
            }
            ProjectLanguage::Cpp => {
                writeln!(stdout, "C++ Toolchain:")?;
                match detect_compiler(ProjectLanguage::Cpp) {
                    Ok(info) => {
                        writeln!(stdout, "  Compiler: {}", info.family)?;
                        writeln!(stdout, "  Version: {}", info.version)?;
                        writeln!(stdout, "  Target: {}", info.target_triple)?;
                        writeln!(stdout, "  Executable: {}", info.path.display())?;

                        let cpp20_supported =
                            probe_standard_support(&info, "c++20").unwrap_or(false);
                        let cpp23_supported =
                            probe_standard_support(&info, "c++23").unwrap_or(false);

                        let cpp20_status = if cpp20_supported {
                            "supported"
                        } else {
                            "unsupported"
                        };
                        let cpp23_status = if cpp23_supported {
                            "supported"
                        } else {
                            "unsupported"
                        };
                        writeln!(
                            stdout,
                            "  Standards: C++20 ({cpp20_status}), C++23 ({cpp23_status})\n"
                        )?;
                    }
                    Err(err) => {
                        writeln!(stdout, "  Status: Not available ({err})\n")?;
                        if args.language.is_some() {
                            had_error = true;
                            writeln!(stderr, "{err}")?;
                        }
                    }
                }
            }
            ProjectLanguage::Other => {
                writeln!(stdout, "Other Language:")?;
                writeln!(
                    stdout,
                    "  Status: Compiler detection not applicable (Other projects do not require C/C++ toolchains)\n"
                )?;
            }
        }
    }

    if had_error {
        Err(CliError::General("toolchain detection failed".to_string()))
    } else {
        Ok(())
    }
}
