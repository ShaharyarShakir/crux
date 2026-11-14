//! Command-line interface definitions and argument parsing for Crux.

use crate::catalog::{PackageCatalog, default_dev_catalog};
use crate::check::{CheckError, check_project};
use crate::init::{InitError, init_project};
use crate::lock::{LOCKFILE_FILENAME, LockError, Lockfile, LockfileError, check_lockfile_sync};
use crate::manifest::{MANIFEST_FILENAME, Manifest};
use crate::registry::{
    GitRegistry, RegistryError, add_registry, list_registries, load_config, update_registry,
};
use crate::resolver::{ResolutionError, Resolver};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Package version obtained at compile time from Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Top-level help and usage message.
pub const HELP_MESSAGE: &str = "\
Crux — A source-first C/C++ package manager

Usage:
    crux [options]
    crux <command> [args]

Commands:
    init             Initialize a new Crux package in the current directory
    check            Validate the project manifest (crux.toml)
    lock             Generate, synchronize, or inspect the project lockfile (crux.lock)
    resolve          Resolve project dependencies
    registry         Manage Git package registries (add, list, update)

Options:
    -h, --help       Print help information
    -V, --version    Print version information
";

/// Subcommand help message for `crux init`.
pub const INIT_HELP_MESSAGE: &str = "\
crux-init — Initialize a new Crux package in the current directory

Usage:
    crux init [options]

Options:
    --name <NAME>    Set the package name (defaults to current directory name)
    -h, --help       Print help information
";

/// Subcommand help message for `crux check`.
pub const CHECK_HELP_MESSAGE: &str = "\
crux-check — Validate the project manifest (crux.toml)

Usage:
    crux check [options]

Options:
    -h, --help       Print help information
";

/// Subcommand help message for `crux lock`.
pub const LOCK_HELP_MESSAGE: &str = "\
crux-lock — Inspect and validate the project lockfile (crux.lock)

Usage:
    crux lock [options]

Options:
    --check          Validate and check synchronization of crux.lock
    --update         Update all dependencies to latest compatible versions
    -h, --help       Print help information
";

/// Subcommand help message for `crux resolve`.
pub const RESOLVE_HELP_MESSAGE: &str = "\
crux-resolve — Resolve project dependencies against in-memory test catalog (dev/test fixture only) or configured registry

Usage:
    crux resolve [options]

Options:
    -h, --help       Print help information
";

/// Subcommand help message for `crux registry`.
pub const REGISTRY_HELP_MESSAGE: &str = "\
crux-registry — Manage Git package registries

Usage:
    crux registry <command> [args]
    crux registry [options]

Commands:
    add <name> <git-url>    Add and clone a new Git package registry
    list                    List all configured package registries
    update <name>           Update an existing package registry from upstream

Options:
    -h, --help              Print help information
";

/// Arguments specific to `crux init`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitArgs {
    pub name: Option<String>,
}

/// Arguments specific to `crux lock`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LockArgs {
    pub update: bool,
}

/// Subcommands for `crux registry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryAction {
    Help,
    Add { name: String, url: String },
    List,
    Update { name: String },
}

/// Supported top-level actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Help,
    Version,
    InitHelp,
    Init(InitArgs),
    CheckHelp,
    Check,
    LockHelp,
    Lock(LockArgs),
    LockCheck,
    ResolveHelp,
    Resolve,
    Registry(RegistryAction),
}

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
    Io(String),
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
            CliError::Io(err) => write!(f, "error: I/O failure: {err}"),
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

/// Parses CLI arguments (excluding the binary name).
pub fn parse_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    if args.is_empty() {
        return Ok(Action::Help);
    }

    match args[0].as_ref() {
        "--help" | "-h" => {
            if args.len() == 1 {
                Ok(Action::Help)
            } else {
                Err(CliError::UnexpectedArgument(args[1].as_ref().to_string()))
            }
        }
        "--version" | "-V" => {
            if args.len() == 1 {
                Ok(Action::Version)
            } else {
                Err(CliError::UnexpectedArgument(args[1].as_ref().to_string()))
            }
        }
        "init" => parse_init_args(&args[1..]),
        "check" => parse_check_args(&args[1..]),
        "lock" => parse_lock_args(&args[1..]),
        "resolve" => parse_resolve_args(&args[1..]),
        "registry" => parse_registry_args(&args[1..]),
        other => Err(CliError::UnexpectedArgument(other.to_string())),
    }
}

fn parse_registry_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    if args.is_empty() {
        return Ok(Action::Registry(RegistryAction::Help));
    }

    match args[0].as_ref() {
        "-h" | "--help" => Ok(Action::Registry(RegistryAction::Help)),
        "list" => {
            if args.len() > 1 {
                if args[1].as_ref() == "-h" || args[1].as_ref() == "--help" {
                    return Ok(Action::Registry(RegistryAction::Help));
                }
                return Err(CliError::UnexpectedArgument(args[1].as_ref().to_string()));
            }
            Ok(Action::Registry(RegistryAction::List))
        }
        "add" => {
            if args.len() == 1 {
                return Err(CliError::MissingArgumentValue(
                    "<NAME> and <GIT-URL>".to_string(),
                ));
            }
            if args[1].as_ref() == "-h" || args[1].as_ref() == "--help" {
                return Ok(Action::Registry(RegistryAction::Help));
            }
            if args.len() == 2 {
                return Err(CliError::MissingArgumentValue("<GIT-URL>".to_string()));
            }
            if args.len() > 3 {
                return Err(CliError::UnexpectedArgument(args[3].as_ref().to_string()));
            }
            Ok(Action::Registry(RegistryAction::Add {
                name: args[1].as_ref().to_string(),
                url: args[2].as_ref().to_string(),
            }))
        }
        "update" => {
            if args.len() == 1 {
                return Err(CliError::MissingArgumentValue("<NAME>".to_string()));
            }
            if args[1].as_ref() == "-h" || args[1].as_ref() == "--help" {
                return Ok(Action::Registry(RegistryAction::Help));
            }
            if args.len() > 2 {
                return Err(CliError::UnexpectedArgument(args[2].as_ref().to_string()));
            }
            Ok(Action::Registry(RegistryAction::Update {
                name: args[1].as_ref().to_string(),
            }))
        }
        other => Err(CliError::UnexpectedArgument(other.to_string())),
    }
}

fn load_active_catalog(
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

fn parse_init_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut name = None;
    let mut i = 0;

    while i < args.len() {
        let arg = args[i].as_ref();
        match arg {
            "-h" | "--help" => return Ok(Action::InitHelp),
            "--name" => {
                if i + 1 < args.len() {
                    let val = args[i + 1].as_ref();
                    if val.starts_with('-') {
                        return Err(CliError::MissingArgumentValue("--name".to_string()));
                    }
                    name = Some(val.to_string());
                    i += 1;
                } else {
                    return Err(CliError::MissingArgumentValue("--name".to_string()));
                }
            }
            opt if opt.starts_with("--name=") => {
                let val = &opt["--name=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("--name".to_string()));
                }
                name = Some(val.to_string());
            }
            unexpected => return Err(CliError::UnexpectedArgument(unexpected.to_string())),
        }
        i += 1;
    }

    Ok(Action::Init(InitArgs { name }))
}

fn parse_check_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    match args {
        [] => Ok(Action::Check),
        [single] if single.as_ref() == "-h" || single.as_ref() == "--help" => Ok(Action::CheckHelp),
        [other, ..] => Err(CliError::UnexpectedArgument(other.as_ref().to_string())),
    }
}

fn parse_lock_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    match args {
        [] => Ok(Action::Lock(LockArgs { update: false })),
        [single] if single.as_ref() == "-h" || single.as_ref() == "--help" => Ok(Action::LockHelp),
        [single] if single.as_ref() == "--check" => Ok(Action::LockCheck),
        [single] if single.as_ref() == "--update" => Ok(Action::Lock(LockArgs { update: true })),
        [other, ..] => Err(CliError::UnexpectedArgument(other.as_ref().to_string())),
    }
}

fn parse_resolve_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    match args {
        [] => Ok(Action::Resolve),
        [single] if single.as_ref() == "-h" || single.as_ref() == "--help" => {
            Ok(Action::ResolveHelp)
        }
        [other, ..] => Err(CliError::UnexpectedArgument(other.as_ref().to_string())),
    }
}

/// Dispatches CLI execution with custom stdout and stderr writers.
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
            match init_project(&current_dir, init_args.name.as_deref()) {
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
        Ok(Action::Check) => {
            let current_dir = std::env::current_dir()?;
            writeln!(stdout, "Checking Crux project...")?;
            match check_project(&current_dir) {
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
        Ok(Action::Lock(lock_args)) => {
            let current_dir = std::env::current_dir()?;
            let manifest_path = current_dir.join(MANIFEST_FILENAME);
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

            let (catalog, note) = load_active_catalog(&current_dir)?;

            writeln!(
                stdout,
                "Resolving dependencies for '{}' ({})...",
                manifest.package.name, manifest.package.version
            )?;
            if let Some(n) = note {
                writeln!(stdout, "{n}")?;
            }

            let lockfile_path = current_dir.join(LOCKFILE_FILENAME);
            let existing_lockfile = if lockfile_path.is_file() {
                match Lockfile::load_from_path(&lockfile_path) {
                    Ok(lf) => Some(lf),
                    Err(err) => {
                        writeln!(stderr, "error reading existing lockfile: {err}")?;
                        return Err(CliError::Lock(LockError::Lockfile(err)));
                    }
                }
            } else {
                None
            };

            let mut preferences = BTreeMap::new();
            if !lock_args.update
                && let Some(ref lf) = existing_lockfile
            {
                for pkg in &lf.packages {
                    preferences.insert(pkg.name.clone(), pkg.version.clone());
                }
            }

            let resolver = Resolver::new(catalog.as_ref());

            let graph = match resolver.resolve_with_preferences(
                Some(manifest.package.id()),
                &manifest.dependencies,
                &preferences,
            ) {
                Ok(g) => g,
                Err(err) => {
                    writeln!(stderr, "\nStatus: Resolution Failed")?;
                    writeln!(stderr, "error: {err}")?;
                    return Err(CliError::Resolution(Box::new(err)));
                }
            };

            let new_lockfile = match graph.to_lockfile_with_existing(existing_lockfile.as_ref()) {
                Ok(lf) => lf,
                Err(err) => {
                    writeln!(stderr, "error constructing lockfile: {err}")?;
                    return Err(CliError::Lock(LockError::Lockfile(
                        LockfileError::Validation(err),
                    )));
                }
            };

            if let Err(err) = new_lockfile.write_atomic(&current_dir) {
                writeln!(stderr, "error: failed to write lockfile: {err}")?;
                return Err(CliError::Lock(LockError::Lockfile(err)));
            }

            if existing_lockfile.is_some() {
                if lock_args.update {
                    writeln!(
                        stdout,
                        "\nUpdated crux.lock ({} packages locked)",
                        new_lockfile.package_count()
                    )?;
                } else {
                    writeln!(
                        stdout,
                        "\nSynchronized crux.lock ({} packages locked)",
                        new_lockfile.package_count()
                    )?;
                }
            } else {
                writeln!(
                    stdout,
                    "\nGenerated crux.lock ({} packages locked)",
                    new_lockfile.package_count()
                )?;
            }

            for pkg in &new_lockfile.packages {
                writeln!(stdout, "  {} {} ({})", pkg.name, pkg.version, pkg.source)?;
            }
            writeln!(stdout, "\nStatus: Success")?;
            Ok(())
        }
        Ok(Action::LockCheck) => {
            let current_dir = std::env::current_dir()?;
            let (catalog, _) = load_active_catalog(&current_dir)?;
            writeln!(stdout, "Checking Crux lockfile...")?;
            match check_lockfile_sync(&current_dir, catalog.as_ref()) {
                Ok(report) => {
                    writeln!(stdout, "Lockfile: crux.lock")?;
                    writeln!(stdout, "Format version: {}", report.lockfile_version)?;
                    writeln!(stdout, "Packages: {}", report.package_count)?;
                    if report.is_synchronized {
                        writeln!(stdout, "Status: Valid (synchronized)")?;
                    } else {
                        writeln!(stdout, "Status: Valid")?;
                    }
                    Ok(())
                }
                Err(err) => {
                    writeln!(stderr, "Lockfile: crux.lock")?;
                    writeln!(stderr, "Status: Invalid")?;
                    writeln!(stderr, "{err}")?;
                    Err(CliError::Lock(err))
                }
            }
        }
        Ok(Action::Resolve) => {
            let current_dir = std::env::current_dir()?;
            let manifest_path = current_dir.join(MANIFEST_FILENAME);
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

            let (catalog, note) = load_active_catalog(&current_dir)?;

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
        Ok(Action::Registry(RegistryAction::Help)) => {
            stdout.write_all(REGISTRY_HELP_MESSAGE.as_bytes())?;
            Ok(())
        }
        Ok(Action::Registry(RegistryAction::List)) => {
            let current_dir = std::env::current_dir().ok();
            let list = list_registries(current_dir.as_deref())?;
            if list.is_empty() {
                writeln!(stdout, "No package registries configured.")?;
            } else {
                writeln!(stdout, "Configured registries ({}):", list.len())?;
                for item in list {
                    writeln!(stdout, "  {}", item.name)?;
                    if let Some(ref url) = item.config.url {
                        writeln!(stdout, "    URL:      {}", url)?;
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
        Ok(Action::Registry(RegistryAction::Add { name, url })) => {
            writeln!(stdout, "Adding registry '{name}' from '{url}'...")?;
            let current_dir = std::env::current_dir().ok();
            match add_registry(&name, &url, current_dir.as_deref()) {
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
        Ok(Action::Registry(RegistryAction::Update { name })) => {
            writeln!(stdout, "Updating registry '{name}'...")?;
            let current_dir = std::env::current_dir().ok();
            match update_registry(&name, current_dir.as_deref()) {
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
        Err(err) => {
            writeln!(stderr, "{err}")?;
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_run_registry_help_output() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let res = run(["crux", "registry", "--help"], &mut out, &mut err);
        assert!(res.is_ok());
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("crux-registry — Manage Git package registries"));
        assert!(output.contains("add <name> <git-url>"));
        assert!(output.contains("list"));
        assert!(output.contains("update <name>"));
        assert!(err.is_empty());
    }
}
