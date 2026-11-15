//! Command-line interface definitions, argument parsing, error handling, and orchestration for Crux.

pub mod args;
pub mod commands;

pub use args::*;
pub use commands::*;

use crate::build::BuildError;
use crate::install::InstallError;
use crate::integrate::IntegrationError;
use crate::project::check::CheckError;
use crate::project::init::InitError;
use crate::registry::{GitRegistry, RegistryError, load_config};
use crate::resolution::catalog::{PackageCatalog, default_dev_catalog};
use crate::resolution::lockfile::LockError;
use crate::resolution::resolver::ResolutionError;
use crate::source::AcquisitionError;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Errors encountered while parsing CLI arguments or executing actions.
#[derive(Debug)]
pub enum CliError {
    UnexpectedArgument(String),
    MissingArgumentValue(String),
    Init(InitError),
    Check(CheckError),
    Lock(LockError),
    Resolution(Box<ResolutionError>),
    Registry(Box<RegistryError>),
    Acquisition(Box<AcquisitionError>),
    Build(Box<BuildError>),
    Install(Box<InstallError>),
    Integration(Box<IntegrationError>),
    Io(String),
    General(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::UnexpectedArgument(arg) => {
                write!(
                    f,
                    "error: unexpected argument '{arg}'\n\nFor more information, try '--help'."
                )
            }
            CliError::MissingArgumentValue(flag) => {
                write!(
                    f,
                    "error: flag '{flag}' requires a value\n\nFor more information, try '--help'."
                )
            }
            CliError::Init(err) => write!(f, "{err}"),
            CliError::Check(err) => write!(f, "{err}"),
            CliError::Lock(err) => write!(f, "{err}"),
            CliError::Resolution(err) => write!(f, "{err}"),
            CliError::Registry(err) => write!(f, "{err}"),
            CliError::Acquisition(err) => write!(f, "{err}"),
            CliError::Build(err) => write!(f, "{err}"),
            CliError::Install(err) => write!(f, "{err}"),
            CliError::Integration(err) => write!(f, "{err}"),
            CliError::Io(err) => write!(f, "error: I/O failure: {err}"),
            CliError::General(msg) => write!(f, "error: {msg}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<io::Error> for CliError {
    fn from(err: io::Error) -> Self {
        CliError::Io(err.to_string())
    }
}

impl From<InitError> for CliError {
    fn from(err: InitError) -> Self {
        CliError::Init(err)
    }
}

impl From<CheckError> for CliError {
    fn from(err: CheckError) -> Self {
        CliError::Check(err)
    }
}

impl From<LockError> for CliError {
    fn from(err: LockError) -> Self {
        CliError::Lock(err)
    }
}

impl From<ResolutionError> for CliError {
    fn from(err: ResolutionError) -> Self {
        CliError::Resolution(Box::new(err))
    }
}

impl From<RegistryError> for CliError {
    fn from(err: RegistryError) -> Self {
        CliError::Registry(Box::new(err))
    }
}

impl From<AcquisitionError> for CliError {
    fn from(err: AcquisitionError) -> Self {
        CliError::Acquisition(Box::new(err))
    }
}

impl From<BuildError> for CliError {
    fn from(err: BuildError) -> Self {
        CliError::Build(Box::new(err))
    }
}

impl From<InstallError> for CliError {
    fn from(err: InstallError) -> Self {
        CliError::Install(Box::new(err))
    }
}

impl From<IntegrationError> for CliError {
    fn from(err: IntegrationError) -> Self {
        CliError::Integration(Box::new(err))
    }
}

pub(crate) fn load_active_catalog(
    project_dir: &Path,
) -> Result<(Box<dyn PackageCatalog>, Option<String>), CliError> {
    let config = load_config(Some(project_dir))?;

    if let Some((reg_name, reg_cfg)) = config.get_primary_registry() {
        match GitRegistry::open(&reg_cfg.path) {
            Ok(git_reg) => {
                let note = format!("Using registry '{}' ({})", reg_name, reg_cfg.path.display());
                Ok((Box::new(git_reg), Some(note)))
            }
            Err(err) => {
                // Do not silently fall back to an in-memory catalog when Git access fails.
                Err(CliError::Registry(Box::new(err)))
            }
        }
    } else if let Ok(env_path) = std::env::var("CRUX_REGISTRY_PATH") {
        let path = PathBuf::from(&env_path);
        match GitRegistry::open(&path) {
            Ok(git_reg) => {
                let note = format!(
                    "Using registry from CRUX_REGISTRY_PATH ({})",
                    path.display()
                );
                Ok((Box::new(git_reg), Some(note)))
            }
            Err(err) => Err(CliError::Registry(Box::new(err))),
        }
    } else {
        let note = "Note: Using in-memory test catalog (development/test fixture only; no remote registry configured).".to_string();
        Ok((Box::new(default_dev_catalog()), Some(note)))
    }
}

pub(crate) fn load_active_registry(project_dir: &Path) -> Result<(GitRegistry, String), CliError> {
    let config = load_config(Some(project_dir))?;

    if let Some((reg_name, reg_cfg)) = config.get_primary_registry() {
        let git_reg =
            GitRegistry::open(&reg_cfg.path).map_err(|e| CliError::Registry(Box::new(e)))?;
        Ok((git_reg, reg_name.to_string()))
    } else if let Ok(env_path) = std::env::var("CRUX_REGISTRY_PATH") {
        let path = PathBuf::from(&env_path);
        let git_reg = GitRegistry::open(&path).map_err(|e| CliError::Registry(Box::new(e)))?;
        let name = git_reg.name().to_string();
        Ok((git_reg, name))
    } else if project_dir.join("registry").join("registry.toml").is_file() {
        let path = project_dir.join("registry");
        let git_reg = GitRegistry::open(&path).map_err(|e| CliError::Registry(Box::new(e)))?;
        let name = git_reg.name().to_string();
        Ok((git_reg, name))
    } else {
        Err(CliError::General(
            "no package registry configured; use 'crux registry add <name> <git-url>' to add one"
                .to_string(),
        ))
    }
}

/// Executes the CLI with given arguments and streams.
pub fn run<I, T, W, E>(args: I, mut stdout: W, mut stderr: E) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
    W: Write,
    E: Write,
{
    let args_vec: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();

    // Skip the program binary name if present
    let effective_args = if args_vec.is_empty() {
        &[][..]
    } else {
        &args_vec[1..]
    };

    match parse_args(effective_args) {
        Ok(Action::Help) => {
            stdout.write_all(HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::Version) => {
            writeln!(stdout, "crux {VERSION}")?;
            Ok(())
        }
        Ok(Action::InitHelp) => {
            stdout.write_all(INIT_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::CheckHelp) => {
            stdout.write_all(CHECK_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::LockHelp) => {
            stdout.write_all(LOCK_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::ResolveHelp) => {
            stdout.write_all(RESOLVE_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::Init(init_args)) => {
            let current_dir = std::env::current_dir()?;
            run_init(&current_dir, &init_args, &mut stdout, &mut stderr)
        }
        Ok(Action::Check) => {
            let current_dir = std::env::current_dir()?;
            run_check(&current_dir, &mut stdout, &mut stderr)
        }
        Ok(Action::Lock(lock_args)) => {
            let current_dir = std::env::current_dir()?;
            run_lock(&current_dir, &lock_args, &mut stdout, &mut stderr)
        }
        Ok(Action::LockCheck) => {
            let current_dir = std::env::current_dir()?;
            run_lock_check(&current_dir, &mut stdout, &mut stderr)
        }
        Ok(Action::Resolve) => {
            let current_dir = std::env::current_dir()?;
            run_resolve(&current_dir, &mut stdout, &mut stderr)
        }
        Ok(Action::Registry(RegistryAction::Help)) => {
            stdout.write_all(REGISTRY_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::Registry(RegistryAction::List)) => {
            let current_dir = std::env::current_dir().ok();
            run_registry_list(current_dir.as_deref(), &mut stdout)
        }
        Ok(Action::Registry(RegistryAction::Add { name, url })) => {
            let current_dir = std::env::current_dir().ok();
            run_registry_add(
                current_dir.as_deref(),
                &name,
                &url,
                &mut stdout,
                &mut stderr,
            )
        }
        Ok(Action::Registry(RegistryAction::Update { name })) => {
            let current_dir = std::env::current_dir().ok();
            run_registry_update(current_dir.as_deref(), &name, &mut stdout, &mut stderr)
        }
        Ok(Action::FetchHelp) => {
            stdout.write_all(FETCH_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::Fetch { package, force }) => {
            let current_dir = std::env::current_dir()?;
            run_fetch(
                &current_dir,
                package.as_deref(),
                force,
                &mut stdout,
                &mut stderr,
            )
        }
        Ok(Action::BuildHelp) => {
            stdout.write_all(BUILD_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::Build {
            package,
            profile,
            force,
        }) => {
            let current_dir = std::env::current_dir()?;
            run_build(
                &current_dir,
                package.as_deref(),
                profile,
                force,
                &mut stdout,
                &mut stderr,
            )
        }
        Ok(Action::InstallHelp) => {
            stdout.write_all(INSTALL_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::Install {
            package,
            profile,
            force,
            locked,
        }) => {
            let current_dir = std::env::current_dir()?;
            run_install(
                &current_dir,
                package.as_deref(),
                profile,
                force,
                locked,
                &mut stdout,
                &mut stderr,
            )
        }
        Ok(Action::IntegrateHelp) => {
            stdout.write_all(INTEGRATE_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::IntegrateCmakeHelp) => {
            stdout.write_all(INTEGRATE_CMAKE_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::IntegrateCmake {
            package,
            profile,
            locked,
        }) => {
            let current_dir = std::env::current_dir()?;
            run_integrate_cmake(
                &current_dir,
                package.as_deref(),
                profile,
                locked,
                &mut stdout,
                &mut stderr,
            )
        }
        Err(err) => {
            writeln!(stderr, "{err}")?;
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::BuildProfile;

    #[test]
    fn test_parse_empty_args_defaults_to_help() {
        assert_eq!(parse_args::<&str>(&[]).unwrap(), Action::Help);
    }

    #[test]
    fn test_parse_check_plain() {
        assert_eq!(parse_args(&["check"]).unwrap(), Action::Check);
    }

    #[test]
    fn test_parse_check_help() {
        assert_eq!(parse_args(&["check", "-h"]).unwrap(), Action::CheckHelp);
        assert_eq!(parse_args(&["check", "--help"]).unwrap(), Action::CheckHelp);
    }

    #[test]
    fn test_parse_check_unexpected_arg() {
        assert!(matches!(
            parse_args(&["check", "--extra"]),
            Err(CliError::UnexpectedArgument(_))
        ));
    }

    #[test]
    fn test_run_check_help_output() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let res = run(["crux", "check", "--help"], &mut out, &mut err);
        assert!(res.is_ok());
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("crux-check — Validate the project manifest"));
        assert!(err.is_empty());
    }

    #[test]
    fn test_parse_lock_plain() {
        assert_eq!(
            parse_args(&["lock"]).unwrap(),
            Action::Lock(LockArgs { update: false })
        );
    }

    #[test]
    fn test_parse_lock_update() {
        assert_eq!(
            parse_args(&["lock", "--update"]).unwrap(),
            Action::Lock(LockArgs { update: true })
        );
    }

    #[test]
    fn test_parse_lock_help() {
        assert_eq!(parse_args(&["lock", "-h"]).unwrap(), Action::LockHelp);
        assert_eq!(parse_args(&["lock", "--help"]).unwrap(), Action::LockHelp);
    }

    #[test]
    fn test_parse_lock_check() {
        assert_eq!(parse_args(&["lock", "--check"]).unwrap(), Action::LockCheck);
    }

    #[test]
    fn test_parse_lock_unexpected_arg() {
        assert!(matches!(
            parse_args(&["lock", "--bogus"]),
            Err(CliError::UnexpectedArgument(_))
        ));
    }

    #[test]
    fn test_run_lock_help_output() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let res = run(["crux", "lock", "--help"], &mut out, &mut err);
        assert!(res.is_ok());
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("crux-lock — Inspect and validate the project lockfile"));
        assert!(output.contains("--check"));
        assert!(output.contains("--update"));
        assert!(err.is_empty());
    }

    #[test]
    fn test_parse_resolve_plain() {
        assert_eq!(parse_args(&["resolve"]).unwrap(), Action::Resolve);
    }

    #[test]
    fn test_parse_resolve_help() {
        assert_eq!(parse_args(&["resolve", "-h"]).unwrap(), Action::ResolveHelp);
        assert_eq!(
            parse_args(&["resolve", "--help"]).unwrap(),
            Action::ResolveHelp
        );
    }

    #[test]
    fn test_parse_resolve_unexpected_arg() {
        assert!(matches!(
            parse_args(&["resolve", "--bogus"]),
            Err(CliError::UnexpectedArgument(_))
        ));
    }

    #[test]
    fn test_run_resolve_help_output() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let res = run(["crux", "resolve", "--help"], &mut out, &mut err);
        assert!(res.is_ok());
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("crux-resolve — Resolve project dependencies"));
        assert!(err.is_empty());
    }

    #[test]
    fn test_parse_registry_help() {
        assert_eq!(
            parse_args(&["registry"]).unwrap(),
            Action::Registry(RegistryAction::Help)
        );
        assert_eq!(
            parse_args(&["registry", "-h"]).unwrap(),
            Action::Registry(RegistryAction::Help)
        );
        assert_eq!(
            parse_args(&["registry", "--help"]).unwrap(),
            Action::Registry(RegistryAction::Help)
        );
    }

    #[test]
    fn test_parse_registry_list() {
        assert_eq!(
            parse_args(&["registry", "list"]).unwrap(),
            Action::Registry(RegistryAction::List)
        );
    }

    #[test]
    fn test_parse_registry_add() {
        assert_eq!(
            parse_args(&["registry", "add", "default", "https://example.com/repo.git"]).unwrap(),
            Action::Registry(RegistryAction::Add {
                name: "default".to_string(),
                url: "https://example.com/repo.git".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_registry_add_missing_args() {
        assert!(matches!(
            parse_args(&["registry", "add"]),
            Err(CliError::MissingArgumentValue(_))
        ));
        assert!(matches!(
            parse_args(&["registry", "add", "default"]),
            Err(CliError::MissingArgumentValue(_))
        ));
    }

    #[test]
    fn test_parse_registry_update() {
        assert_eq!(
            parse_args(&["registry", "update", "default"]).unwrap(),
            Action::Registry(RegistryAction::Update {
                name: "default".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_registry_update_missing_args() {
        assert!(matches!(
            parse_args(&["registry", "update"]),
            Err(CliError::MissingArgumentValue(_))
        ));
    }

    #[test]
    fn test_parse_build_args() {
        assert_eq!(
            parse_args(&["build"]).unwrap(),
            Action::Build {
                package: None,
                profile: BuildProfile::Release,
                force: false,
            }
        );

        assert_eq!(
            parse_args(&["build", "fmt", "--profile", "debug", "--force"]).unwrap(),
            Action::Build {
                package: Some("fmt".to_string()),
                profile: BuildProfile::Debug,
                force: true,
            }
        );

        assert_eq!(
            parse_args(&["build", "fmt@11.1.1", "--profile=release"]).unwrap(),
            Action::Build {
                package: Some("fmt@11.1.1".to_string()),
                profile: BuildProfile::Release,
                force: false,
            }
        );

        assert_eq!(parse_args(&["build", "-h"]).unwrap(), Action::BuildHelp);
    }

    #[test]
    fn test_parse_build_invalid_args() {
        assert!(matches!(
            parse_args(&["build", "--profile"]),
            Err(CliError::MissingArgumentValue(_))
        ));

        assert!(matches!(
            parse_args(&["build", "--profile", "invalid"]),
            Err(CliError::General(_))
        ));

        assert!(matches!(
            parse_args(&["build", "fmt", "extra"]),
            Err(CliError::UnexpectedArgument(_))
        ));
    }

    #[test]
    fn test_parse_install_args() {
        assert_eq!(
            parse_args(&["install"]).unwrap(),
            Action::Install {
                package: None,
                profile: BuildProfile::Release,
                force: false,
                locked: false,
            }
        );

        assert_eq!(
            parse_args(&[
                "install",
                "fmt",
                "--profile",
                "debug",
                "--force",
                "--locked"
            ])
            .unwrap(),
            Action::Install {
                package: Some("fmt".to_string()),
                profile: BuildProfile::Debug,
                force: true,
                locked: true,
            }
        );

        assert_eq!(
            parse_args(&["install", "fmt@11.1.1", "--profile=release"]).unwrap(),
            Action::Install {
                package: Some("fmt@11.1.1".to_string()),
                profile: BuildProfile::Release,
                force: false,
                locked: false,
            }
        );

        assert_eq!(parse_args(&["install", "-h"]).unwrap(), Action::InstallHelp);
    }

    #[test]
    fn test_parse_install_invalid_args() {
        assert!(matches!(
            parse_args(&["install", "--profile"]),
            Err(CliError::MissingArgumentValue(_))
        ));

        assert!(matches!(
            parse_args(&["install", "--profile", "invalid"]),
            Err(CliError::General(_))
        ));

        assert!(matches!(
            parse_args(&["install", "fmt", "extra"]),
            Err(CliError::UnexpectedArgument(_))
        ));
    }

    #[test]
    fn test_parse_integrate_args() {
        assert_eq!(parse_args(&["integrate"]).unwrap(), Action::IntegrateHelp);
        assert_eq!(
            parse_args(&["integrate", "-h"]).unwrap(),
            Action::IntegrateHelp
        );
        assert_eq!(
            parse_args(&["integrate", "--help"]).unwrap(),
            Action::IntegrateHelp
        );

        assert_eq!(
            parse_args(&["integrate", "cmake"]).unwrap(),
            Action::IntegrateCmake {
                package: None,
                profile: BuildProfile::Release,
                locked: false,
            }
        );

        assert_eq!(
            parse_args(&[
                "integrate",
                "cmake",
                "fmt@11.1.1",
                "--profile",
                "debug",
                "--locked"
            ])
            .unwrap(),
            Action::IntegrateCmake {
                package: Some("fmt@11.1.1".to_string()),
                profile: BuildProfile::Debug,
                locked: true,
            }
        );

        assert_eq!(
            parse_args(&["integrate", "cmake", "-h"]).unwrap(),
            Action::IntegrateCmakeHelp
        );
    }

    #[test]
    fn test_parse_integrate_invalid_args() {
        assert!(matches!(
            parse_args(&["integrate", "unsupported"]),
            Err(CliError::UnexpectedArgument(_))
        ));

        assert!(matches!(
            parse_args(&["integrate", "cmake", "--profile"]),
            Err(CliError::MissingArgumentValue(_))
        ));

        assert!(matches!(
            parse_args(&["integrate", "cmake", "--profile", "invalid"]),
            Err(CliError::General(_))
        ));

        assert!(matches!(
            parse_args(&["integrate", "cmake", "fmt", "extra"]),
            Err(CliError::UnexpectedArgument(_))
        ));
    }
}
