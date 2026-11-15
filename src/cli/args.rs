//! Command-line argument definitions, help messages, and parsing logic.

use crate::build::BuildProfile;
use crate::cli::CliError;

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
    fetch            Fetch, verify, and extract package sources
    build            Compile package sources using the CMake build engine
    install          Build and install packages into project-local Crux installation
    integrate        Generate build system integration files (e.g. CMake)

Options:
    -h, --help       Print help information
    -V, --version    Print version information
";

/// Help and usage message for the `integrate` command group.
pub const INTEGRATE_HELP_MESSAGE: &str = "\
crux-integrate — Generate build system integration files

Usage:
    crux integrate <subcommand> [args]

Subcommands:
    cmake            Generate CMake integration files (.crux/cmake/crux-dependencies.cmake)

Options:
    -h, --help       Print help information
";

/// Help and usage message for `integrate cmake`.
pub const INTEGRATE_CMAKE_HELP_MESSAGE: &str = "\
crux-integrate-cmake — Generate CMake integration files

Usage:
    crux integrate cmake [package] [options]

Arguments:
    [package]        Optional package name or name@version to integrate (defaults to all locked dependencies)

Options:
    --profile <name> Build profile (debug or release; defaults to release)
    --locked         Verify crux.lock is synchronized and requires packages to be locked
    -h, --help       Print help information
";

/// Help and usage message for the `install` command.
pub const INSTALL_HELP_MESSAGE: &str = "\
crux-install — Build and install packages into project-local Crux installation

Usage:
    crux install [package] [options]

Arguments:
    [package]        Optional package name or name@version to install (defaults to all locked dependencies)

Options:
    --profile <name> Build profile (debug or release; defaults to release)
    --force          Force reinstall even if already installed with matching fingerprint
    --locked         Require exact match with crux.lock
    -h, --help       Print help information
";

/// Help and usage message for the `build` command.
pub const BUILD_HELP_MESSAGE: &str = "\
crux-build — Compile package sources using the CMake build engine

Usage:
    crux build [package] [options]

Arguments:
    [package]        Optional package name or name@version to build (defaults to all locked dependencies)

Options:
    --profile <type> Build profile (debug or release; defaults to release)
    --force          Force rebuild even if cache is valid
    -h, --help       Print help information
";

/// Help and usage message for the `fetch` command.
pub const FETCH_HELP_MESSAGE: &str = "\
crux-fetch — Fetch, verify, and extract package sources

Usage:
    crux fetch [package] [options]

Arguments:
    [package]        Optional package name or name@version to fetch (defaults to all locked dependencies)

Options:
    --force          Force re-download and extraction even if cache exists
    -h, --help       Print help information
";

/// Help and usage message for the `init` command.
pub const INIT_HELP_MESSAGE: &str = "\
crux-init — Initialize a new Crux package in the current directory

Usage:
    crux init [options]

Options:
    --name <NAME>    Set the package name (defaults to current directory name)
    -h, --help       Print help information
";

/// Help and usage message for the `check` command.
pub const CHECK_HELP_MESSAGE: &str = "\
crux-check — Validate the project manifest (crux.toml)

Usage:
    crux check

Options:
    -h, --help       Print help information
";

/// Help and usage message for the `lock` command.
pub const LOCK_HELP_MESSAGE: &str = "\
crux-lock — Inspect and validate the project lockfile (crux.lock)

Usage:
    crux lock [options]

Options:
    --check          Verify lockfile exists, is valid, and matches manifest
    --update         Update existing lockfile entries with latest compatible versions
    -h, --help       Print help information
";

/// Help and usage message for the `resolve` command.
pub const RESOLVE_HELP_MESSAGE: &str = "\
crux-resolve — Resolve project dependencies against in-memory test catalog (dev/test fixture only) or configured registry

Usage:
    crux resolve [options]

Options:
    -h, --help       Print help information
";

/// Help and usage message for the `registry` command.
pub const REGISTRY_HELP_MESSAGE: &str = "\
crux-registry — Manage Git package registries

Usage:
    crux registry <subcommand> [args]

Subcommands:
    list             List all configured registries and their package/version counts
    add <name> <url> Add a new Git package registry
    update <name>    Update an existing Git registry to the latest remote commit

Options:
    -h, --help       Print help information
";

/// Arguments specific to the `init` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitArgs {
    pub name: Option<String>,
}

/// Arguments specific to the `lock` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockArgs {
    pub update: bool,
}

/// Subcommands under `crux registry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryAction {
    Help,
    List,
    Add { name: String, url: String },
    Update { name: String },
}

/// Parsed command-line action to execute.
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
    FetchHelp,
    Fetch {
        package: Option<String>,
        force: bool,
    },
    BuildHelp,
    Build {
        package: Option<String>,
        profile: BuildProfile,
        force: bool,
    },
    InstallHelp,
    Install {
        package: Option<String>,
        profile: BuildProfile,
        force: bool,
        locked: bool,
    },
    IntegrateHelp,
    IntegrateCmakeHelp,
    IntegrateCmake {
        package: Option<String>,
        profile: BuildProfile,
        locked: bool,
    },
}

/// Parses CLI arguments (excluding the binary name).
pub fn parse_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    if args.is_empty() {
        return Ok(Action::Help);
    }

    match args[0].as_ref() {
        "-h" | "--help" | "help" => {
            if args.len() > 1 {
                match args[1].as_ref() {
                    "init" => Ok(Action::InitHelp),
                    "check" => Ok(Action::CheckHelp),
                    "lock" => Ok(Action::LockHelp),
                    "resolve" => Ok(Action::ResolveHelp),
                    "registry" => Ok(Action::Registry(RegistryAction::Help)),
                    "fetch" => Ok(Action::FetchHelp),
                    "build" => Ok(Action::BuildHelp),
                    "install" => Ok(Action::InstallHelp),
                    "integrate" => Ok(Action::IntegrateHelp),
                    _ => Ok(Action::Help),
                }
            } else {
                Ok(Action::Help)
            }
        }
        "-V" | "--version" | "version" => {
            if args.len() > 1 {
                Err(CliError::UnexpectedArgument(args[1].as_ref().to_string()))
            } else {
                Ok(Action::Version)
            }
        }
        "init" => parse_init_args(&args[1..]),
        "check" => parse_check_args(&args[1..]),
        "lock" => parse_lock_args(&args[1..]),
        "resolve" => parse_resolve_args(&args[1..]),
        "registry" => parse_registry_args(&args[1..]),
        "fetch" => parse_fetch_args(&args[1..]),
        "build" => parse_build_args(&args[1..]),
        "install" => parse_install_args(&args[1..]),
        "integrate" => parse_integrate_args(&args[1..]),
        other => Err(CliError::UnexpectedArgument(other.to_string())),
    }
}

pub fn parse_fetch_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut package = None;
    let mut force = false;

    for arg in args {
        let s = arg.as_ref();
        match s {
            "-h" | "--help" => return Ok(Action::FetchHelp),
            "--force" => force = true,
            opt if opt.starts_with('-') => {
                return Err(CliError::UnexpectedArgument(opt.to_string()));
            }
            pos => {
                if package.is_none() {
                    package = Some(pos.to_string());
                } else {
                    return Err(CliError::UnexpectedArgument(pos.to_string()));
                }
            }
        }
    }

    Ok(Action::Fetch { package, force })
}

pub fn parse_build_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut package = None;
    let mut profile = BuildProfile::Release;
    let mut force = false;
    let mut iter = args.iter().peekable();

    while let Some(arg) = iter.next() {
        let s = arg.as_ref();
        match s {
            "-h" | "--help" => return Ok(Action::BuildHelp),
            "--force" => force = true,
            "--profile" => {
                let val = iter
                    .next()
                    .ok_or_else(|| CliError::MissingArgumentValue("--profile".to_string()))?;
                profile = match val.as_ref().to_ascii_lowercase().as_str() {
                    "debug" => BuildProfile::Debug,
                    "release" => BuildProfile::Release,
                    other => {
                        return Err(CliError::General(format!(
                            "invalid build profile '{other}', supported: debug, release"
                        )));
                    }
                };
            }
            opt if opt.starts_with("--profile=") => {
                let val = &opt["--profile=".len()..];
                profile = match val.to_ascii_lowercase().as_str() {
                    "debug" => BuildProfile::Debug,
                    "release" => BuildProfile::Release,
                    other => {
                        return Err(CliError::General(format!(
                            "invalid build profile '{other}', supported: debug, release"
                        )));
                    }
                };
            }
            opt if opt.starts_with('-') => {
                return Err(CliError::UnexpectedArgument(opt.to_string()));
            }
            pos => {
                if package.is_none() {
                    package = Some(pos.to_string());
                } else {
                    return Err(CliError::UnexpectedArgument(pos.to_string()));
                }
            }
        }
    }

    Ok(Action::Build {
        package,
        profile,
        force,
    })
}

pub fn parse_install_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut package = None;
    let mut profile = BuildProfile::Release;
    let mut force = false;
    let mut locked = false;
    let mut iter = args.iter().peekable();

    while let Some(arg) = iter.next() {
        let s = arg.as_ref();
        match s {
            "-h" | "--help" => return Ok(Action::InstallHelp),
            "--force" => force = true,
            "--locked" => locked = true,
            "--profile" => {
                let val = iter
                    .next()
                    .ok_or_else(|| CliError::MissingArgumentValue("--profile".to_string()))?;
                profile = match val.as_ref().to_ascii_lowercase().as_str() {
                    "debug" => BuildProfile::Debug,
                    "release" => BuildProfile::Release,
                    other => {
                        return Err(CliError::General(format!(
                            "invalid build profile '{other}', supported: debug, release"
                        )));
                    }
                };
            }
            opt if opt.starts_with("--profile=") => {
                let val = &opt["--profile=".len()..];
                profile = match val.to_ascii_lowercase().as_str() {
                    "debug" => BuildProfile::Debug,
                    "release" => BuildProfile::Release,
                    other => {
                        return Err(CliError::General(format!(
                            "invalid build profile '{other}', supported: debug, release"
                        )));
                    }
                };
            }
            opt if opt.starts_with('-') => {
                return Err(CliError::UnexpectedArgument(opt.to_string()));
            }
            pos => {
                if package.is_none() {
                    package = Some(pos.to_string());
                } else {
                    return Err(CliError::UnexpectedArgument(pos.to_string()));
                }
            }
        }
    }

    Ok(Action::Install {
        package,
        profile,
        force,
        locked,
    })
}

pub fn parse_integrate_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    if args.is_empty() {
        return Ok(Action::IntegrateHelp);
    }

    match args[0].as_ref() {
        "-h" | "--help" => Ok(Action::IntegrateHelp),
        "cmake" => parse_integrate_cmake_args(&args[1..]),
        other => Err(CliError::UnexpectedArgument(other.to_string())),
    }
}

pub fn parse_integrate_cmake_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut package = None;
    let mut profile = BuildProfile::Release;
    let mut locked = false;
    let mut iter = args.iter().peekable();

    while let Some(arg) = iter.next() {
        let s = arg.as_ref();
        match s {
            "-h" | "--help" => return Ok(Action::IntegrateCmakeHelp),
            "--locked" => locked = true,
            "--profile" => {
                let val = iter
                    .next()
                    .ok_or_else(|| CliError::MissingArgumentValue("--profile".to_string()))?;
                profile = match val.as_ref().to_ascii_lowercase().as_str() {
                    "debug" => BuildProfile::Debug,
                    "release" => BuildProfile::Release,
                    other => {
                        return Err(CliError::General(format!(
                            "invalid build profile '{other}', supported: debug, release"
                        )));
                    }
                };
            }
            opt if opt.starts_with("--profile=") => {
                let val = &opt["--profile=".len()..];
                profile = match val.to_ascii_lowercase().as_str() {
                    "debug" => BuildProfile::Debug,
                    "release" => BuildProfile::Release,
                    other => {
                        return Err(CliError::General(format!(
                            "invalid build profile '{other}', supported: debug, release"
                        )));
                    }
                };
            }
            opt if opt.starts_with('-') => {
                return Err(CliError::UnexpectedArgument(opt.to_string()));
            }
            pos => {
                if package.is_none() {
                    package = Some(pos.to_string());
                } else {
                    return Err(CliError::UnexpectedArgument(pos.to_string()));
                }
            }
        }
    }

    Ok(Action::IntegrateCmake {
        package,
        profile,
        locked,
    })
}

pub fn parse_registry_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
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
            if args.len() == 2 && (args[1].as_ref() == "-h" || args[1].as_ref() == "--help") {
                return Ok(Action::Registry(RegistryAction::Help));
            }
            if args.len() < 2 {
                return Err(CliError::MissingArgumentValue("name".to_string()));
            }
            if args.len() < 3 {
                return Err(CliError::MissingArgumentValue("url".to_string()));
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
            if args.len() == 2 && (args[1].as_ref() == "-h" || args[1].as_ref() == "--help") {
                return Ok(Action::Registry(RegistryAction::Help));
            }
            if args.len() < 2 {
                return Err(CliError::MissingArgumentValue("name".to_string()));
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

pub fn parse_init_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
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

pub fn parse_check_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    match args {
        [] => Ok(Action::Check),
        [single] if single.as_ref() == "-h" || single.as_ref() == "--help" => Ok(Action::CheckHelp),
        [unexpected, ..] => Err(CliError::UnexpectedArgument(
            unexpected.as_ref().to_string(),
        )),
    }
}

pub fn parse_lock_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    match args {
        [] => Ok(Action::Lock(LockArgs { update: false })),
        [single] if single.as_ref() == "-h" || single.as_ref() == "--help" => Ok(Action::LockHelp),
        [single] if single.as_ref() == "--check" => Ok(Action::LockCheck),
        [single] if single.as_ref() == "--update" => Ok(Action::Lock(LockArgs { update: true })),
        [unexpected, ..] => Err(CliError::UnexpectedArgument(
            unexpected.as_ref().to_string(),
        )),
    }
}

pub fn parse_resolve_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    match args {
        [] => Ok(Action::Resolve),
        [single] if single.as_ref() == "-h" || single.as_ref() == "--help" => {
            Ok(Action::ResolveHelp)
        }
        [unexpected, ..] => Err(CliError::UnexpectedArgument(
            unexpected.as_ref().to_string(),
        )),
    }
}
