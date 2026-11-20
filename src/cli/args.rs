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
    import           Safely import an existing C/C++ project
    check            Validate the project manifest (crux.toml)
    add              Add a dependency to the project
    remove           Remove a dependency from the project
    lock             Generate, synchronize, or inspect the project lockfile (crux.lock)
    resolve          Resolve project dependencies
    registry         Manage Git package registries (add, list, update)
    fetch            Fetch, verify, and extract package sources
    build            Compile package sources using the CMake build engine
    run              Build and execute the local Crux application
    install          Build and install packages into project-local Crux installation
    integrate        Generate build system integration files (e.g. CMake)
    toolchain        Display detected compiler and toolchain information

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
crux-build — Build the local Crux application or a package with CMake

Usage:
    crux build [package] [options]

Arguments:
    [package]        Optional package name or name@version to build
                     (omit to build the local application)

Options:
    --debug          Build with debug configuration (default for applications)
    --release        Build with release configuration
    --profile <type> Build profile (debug or release; defaults to release for packages, debug for applications)
    -v, --verbose    Show verbose compiler and CMake output
    --force          Force rebuild even if cache is valid
    -h, --help       Print help information

Dispatch:
    With a package argument, 'crux build' compiles that package from the configured
    registry. Without a package argument, a [project] manifest builds the local
    application from sources under src/. Legacy [package] manifests retain v0.1.0
    behavior and build locked dependencies.
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
crux-init — Initialize a new Crux package or project

Usage:
    crux init [path] [options]

Arguments:
    [path]               Target directory for the project (defaults to current directory)

Options:
    -n, --name <NAME>    Set the project name (defaults to directory name)
    -l, --language <LANG> Project language: c, cpp (or c++), other
    -s, --standard <STD> Language standard (e.g. c17 for C, c++20 for C++)
    -h, --help           Print help information
";

/// Help and usage message for the `import` command.
pub const IMPORT_HELP_MESSAGE: &str = "\
crux-import — Safely import an existing C/C++ project

Usage:
    crux import [options]

Options:
    -p, --path <DIR>  Project directory to import (defaults to current directory)
    --dry-run         Show the detected project and proposed crux.toml without writing files
    --force           Accept low-confidence layouts and refresh Crux-managed import metadata;
                      an existing crux.toml is never overwritten
    -h, --help        Print help information

Behavior:
    Import is read-only with respect to existing source, header, CMake, and Make
    files. It never executes project build scripts. Makefile projects are
    detected, but Make is not executed or translated by Crux.
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
crux-resolve — Resolve project dependencies against the active package registry

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

/// Help and usage message for the `run` command.
pub const RUN_HELP_MESSAGE: &str = "\
crux-run — Build and run a Crux application, or compile and run ad-hoc C/C++ sources

Usage:
    crux run [options] [-- <args>...]
    crux run --bin <source> [options] [-- <args>...]
    crux run --sources <source>... [options] [-- <args>...]

Options:
    --bin <source>   Run one C/C++ source file without a crux.toml manifest
    --sources <list> Run one or more sources (repeat the flag, comma-separated, or space-separated)
    -I, --include-dir <dir>
                     Add an include directory for ad-hoc runs (repeatable)
    --debug          Build and run the debug configuration (default)
    --release        Build and run the release configuration
    -v, --verbose    Show verbose compiler and CMake output
    -h, --help       Print help information

Arguments:
    <args>...        Arguments forwarded to the application (after `--`)

Ad-hoc mode:
    `--bin hello.c` is shorthand for `--sources hello.c`. Source extensions are
    .c (C) or .cpp/.cc/.cxx (C++); header files may be listed to copy them into
    the scratch source tree. `--sources a.cpp b.cpp` is accepted; a space-separated
    list ends at the next option or `--`. Include directories may be given as `-I dir`,
    `-Idir`, `--include-dir dir`, or `--include-dir=dir`. Ad-hoc artifacts are
    isolated under `.crux/run/<name>-<hash>/`; no manifest or registry is used.
";

/// Help and usage message for the `add` command.
pub const ADD_HELP_MESSAGE: &str = "\
crux-add — Add a dependency to the project

Usage:
    crux add <package>[@<constraint>]

Arguments:
    <package>        Package name to add (e.g. fmt)
    @<constraint>    Optional SemVer constraint (e.g. @^11.0). Defaults to '*' (any version).

Options:
    -h, --help       Print help information

Examples:
    crux add fmt
    crux add fmt@^11.0
    crux add zlib@>=1.2
";

/// Help and usage message for the `remove` command.
pub const REMOVE_HELP_MESSAGE: &str = "\
crux-remove — Remove a dependency from the project

Usage:
    crux remove <package>

Arguments:
    <package>        Name of the direct dependency to remove

Options:
    -h, --help       Print help information

Examples:
    crux remove fmt
";

/// Help and usage message for the `toolchain` command.
pub const TOOLCHAIN_HELP_MESSAGE: &str = "\
crux-toolchain — Display detected compiler and toolchain information

Usage:
    crux toolchain [options]

Options:
    -l, --language <LANG> Filter toolchain inspection by language (c or cpp)
    -h, --help           Print help information
";

/// Arguments specific to the `init` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitArgs {
    pub path: Option<String>,
    pub name: Option<String>,
    pub language: Option<String>,
    pub standard: Option<String>,
}

/// Arguments specific to the `import` command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportArgs {
    pub path: Option<String>,
    pub dry_run: bool,
    pub force: bool,
}

/// Arguments specific to the `run` command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunArgs {
    pub profile: Option<BuildProfile>,
    pub verbose: bool,
    /// Single ad-hoc source file (`--bin`), without a `crux.toml` manifest.
    pub bin: Option<String>,
    /// Additional ad-hoc source/header files (`--sources`), repeatable, comma-separated, or space-separated.
    pub sources: Vec<String>,
    /// Include directories for ad-hoc runs, repeatable.
    pub include_dirs: Vec<String>,
    pub app_args: Vec<String>,
}

impl RunArgs {
    /// Returns true when explicit sources select ad-hoc run mode.
    pub fn is_adhoc(&self) -> bool {
        self.bin.is_some() || !self.sources.is_empty()
    }
}

/// Arguments specific to the `toolchain` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainArgs {
    pub language: Option<String>,
}

/// Arguments specific to the `lock` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockArgs {
    pub update: bool,
}

/// Arguments specific to the `add` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddArgs {
    /// Package spec in the form "name" or "name@constraint".
    pub spec: String,
}

/// Arguments specific to the `remove` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveArgs {
    /// Name of the package to remove.
    pub name: String,
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
    // New commands for dependency management
    AddHelp,
    Add(AddArgs),
    RemoveHelp,
    Remove(RemoveArgs),
    Help,
    Version,
    InitHelp,
    Init(InitArgs),
    ImportHelp,
    Import(ImportArgs),
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
        profile: Option<BuildProfile>,
        force: bool,
        verbose: bool,
    },
    RunHelp,
    Run(RunArgs),
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
    ToolchainHelp,
    Toolchain(ToolchainArgs),
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
                    "import" => Ok(Action::ImportHelp),
                    "check" => Ok(Action::CheckHelp),
                    "add" => Ok(Action::AddHelp),
                    "remove" => Ok(Action::RemoveHelp),
                    "lock" => Ok(Action::LockHelp),
                    "resolve" => Ok(Action::ResolveHelp),
                    "registry" => Ok(Action::Registry(RegistryAction::Help)),
                    "fetch" => Ok(Action::FetchHelp),
                    "build" => Ok(Action::BuildHelp),
                    "install" => Ok(Action::InstallHelp),
                    "integrate" => Ok(Action::IntegrateHelp),
                    "run" => Ok(Action::RunHelp),
                    "toolchain" => Ok(Action::ToolchainHelp),
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
        "add" => parse_add_args(&args[1..]),
        "remove" => parse_remove_args(&args[1..]),
        "init" => parse_init_args(&args[1..]),
        "import" => parse_import_args(&args[1..]),
        "check" => parse_check_args(&args[1..]),
        "lock" => parse_lock_args(&args[1..]),
        "resolve" => parse_resolve_args(&args[1..]),
        "registry" => parse_registry_args(&args[1..]),
        "fetch" => parse_fetch_args(&args[1..]),
        "build" => parse_build_args(&args[1..]),
        "install" => parse_install_args(&args[1..]),
        "integrate" => parse_integrate_args(&args[1..]),
        "run" => parse_run_args(&args[1..]),
        "toolchain" => parse_toolchain_args(&args[1..]),
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
    let mut profile = None;
    let mut force = false;
    let mut verbose = false;
    let mut iter = args.iter().peekable();

    while let Some(arg) = iter.next() {
        let s = arg.as_ref();
        match s {
            "-h" | "--help" => return Ok(Action::BuildHelp),
            "--force" => force = true,
            "-v" | "--verbose" => verbose = true,
            "--debug" => {
                if let Some(BuildProfile::Release) = profile {
                    return Err(CliError::General(
                        "cannot specify both --debug and --release".to_string(),
                    ));
                }
                profile = Some(BuildProfile::Debug);
            }
            "--release" => {
                if let Some(BuildProfile::Debug) = profile {
                    return Err(CliError::General(
                        "cannot specify both --debug and --release".to_string(),
                    ));
                }
                profile = Some(BuildProfile::Release);
            }
            "--profile" => {
                let val = iter
                    .next()
                    .ok_or_else(|| CliError::MissingArgumentValue("--profile".to_string()))?;
                profile = match val.as_ref().to_ascii_lowercase().as_str() {
                    "debug" => Some(BuildProfile::Debug),
                    "release" => Some(BuildProfile::Release),
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
                    "debug" => Some(BuildProfile::Debug),
                    "release" => Some(BuildProfile::Release),
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
        verbose,
    })
}

fn push_sources(target: &mut Vec<String>, value: &str) -> Result<(), CliError> {
    for item in value.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            return Err(CliError::MissingArgumentValue("--sources".to_string()));
        }
        target.push(trimmed.to_string());
    }
    Ok(())
}

fn push_include_dir(target: &mut Vec<String>, value: &str) -> Result<(), CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::MissingArgumentValue("--include-dir".to_string()));
    }
    target.push(trimmed.to_string());
    Ok(())
}

pub fn parse_run_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut profile = None;
    let mut verbose = false;
    let mut bin = None;
    let mut sources = Vec::new();
    let mut include_dirs = Vec::new();
    let mut app_args = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let s = args[i].as_ref();
        if s == "--" {
            for arg in &args[i + 1..] {
                app_args.push(arg.as_ref().to_string());
            }
            break;
        }

        match s {
            "-h" | "--help" => return Ok(Action::RunHelp),
            "-v" | "--verbose" => verbose = true,
            "--debug" => {
                if let Some(BuildProfile::Release) = profile {
                    return Err(CliError::General(
                        "cannot specify both --debug and --release".to_string(),
                    ));
                }
                profile = Some(BuildProfile::Debug);
            }
            "--release" => {
                if let Some(BuildProfile::Debug) = profile {
                    return Err(CliError::General(
                        "cannot specify both --debug and --release".to_string(),
                    ));
                }
                profile = Some(BuildProfile::Release);
            }
            "--bin" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| CliError::MissingArgumentValue("--bin".to_string()))?
                    .as_ref();
                bin = Some(value.to_string());
                i += 1;
            }
            opt if opt.starts_with("--bin=") => {
                bin = Some(opt["--bin=".len()..].to_string());
            }
            "--sources" => {
                let mut found_source = false;
                while i + 1 < args.len() {
                    let candidate = args[i + 1].as_ref();
                    if candidate == "--" || candidate.starts_with('-') {
                        break;
                    }
                    push_sources(&mut sources, candidate)?;
                    found_source = true;
                    i += 1;
                }
                if !found_source {
                    return Err(CliError::MissingArgumentValue("--sources".to_string()));
                }
            }
            opt if opt.starts_with("--sources=") => {
                push_sources(&mut sources, &opt["--sources=".len()..])?;
            }
            "-I" | "--include-dir" | "--include" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| CliError::MissingArgumentValue(s.to_string()))?
                    .as_ref();
                push_include_dir(&mut include_dirs, value)?;
                i += 1;
            }
            opt if opt.starts_with("--include-dir=") => {
                push_include_dir(&mut include_dirs, &opt["--include-dir=".len()..])?;
            }
            opt if opt.starts_with("--include=") => {
                push_include_dir(&mut include_dirs, &opt["--include=".len()..])?;
            }
            opt if opt.starts_with("-I") && opt.len() > 2 => {
                push_include_dir(&mut include_dirs, &opt[2..])?;
            }
            opt if opt.starts_with('-') => {
                return Err(CliError::UnexpectedArgument(opt.to_string()));
            }
            pos => {
                return Err(CliError::UnexpectedArgument(pos.to_string()));
            }
        }
        i += 1;
    }

    if bin.as_deref().is_some_and(str::is_empty) {
        return Err(CliError::MissingArgumentValue("--bin".to_string()));
    }

    Ok(Action::Run(RunArgs {
        profile,
        verbose,
        bin,
        sources,
        include_dirs,
        app_args,
    }))
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
    let mut path = None;
    let mut name = None;
    let mut language = None;
    let mut standard = None;
    let mut i = 0;

    while i < args.len() {
        let arg = args[i].as_ref();
        match arg {
            "-h" | "--help" => return Ok(Action::InitHelp),
            "-n" | "--name" => {
                if i + 1 < args.len() {
                    let val = args[i + 1].as_ref();
                    if val.starts_with('-') {
                        return Err(CliError::MissingArgumentValue(arg.to_string()));
                    }
                    name = Some(val.to_string());
                    i += 1;
                } else {
                    return Err(CliError::MissingArgumentValue(arg.to_string()));
                }
            }
            opt if opt.starts_with("--name=") => {
                let val = &opt["--name=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("--name".to_string()));
                }
                name = Some(val.to_string());
            }
            opt if opt.starts_with("-n=") => {
                let val = &opt["-n=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("-n".to_string()));
                }
                name = Some(val.to_string());
            }
            "-l" | "--language" => {
                if i + 1 < args.len() {
                    let val = args[i + 1].as_ref();
                    if val.starts_with('-') {
                        return Err(CliError::MissingArgumentValue(arg.to_string()));
                    }
                    language = Some(val.to_string());
                    i += 1;
                } else {
                    return Err(CliError::MissingArgumentValue(arg.to_string()));
                }
            }
            opt if opt.starts_with("--language=") => {
                let val = &opt["--language=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("--language".to_string()));
                }
                language = Some(val.to_string());
            }
            opt if opt.starts_with("-l=") => {
                let val = &opt["-l=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("-l".to_string()));
                }
                language = Some(val.to_string());
            }
            "-s" | "--standard" => {
                if i + 1 < args.len() {
                    let val = args[i + 1].as_ref();
                    if val.starts_with('-') {
                        return Err(CliError::MissingArgumentValue(arg.to_string()));
                    }
                    standard = Some(val.to_string());
                    i += 1;
                } else {
                    return Err(CliError::MissingArgumentValue(arg.to_string()));
                }
            }
            opt if opt.starts_with("--standard=") => {
                let val = &opt["--standard=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("--standard".to_string()));
                }
                standard = Some(val.to_string());
            }
            opt if opt.starts_with("-s=") => {
                let val = &opt["-s=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("-s".to_string()));
                }
                standard = Some(val.to_string());
            }
            pos if !pos.starts_with('-') => {
                if path.is_some() {
                    return Err(CliError::UnexpectedArgument(pos.to_string()));
                }
                path = Some(pos.to_string());
            }
            unexpected => return Err(CliError::UnexpectedArgument(unexpected.to_string())),
        }
        i += 1;
    }

    Ok(Action::Init(InitArgs {
        path,
        name,
        language,
        standard,
    }))
}

pub fn parse_import_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut path = None;
    let mut dry_run = false;
    let mut force = false;
    let mut i = 0;

    while i < args.len() {
        let arg = args[i].as_ref();
        match arg {
            "-h" | "--help" => return Ok(Action::ImportHelp),
            "-p" | "--path" => {
                if i + 1 >= args.len() {
                    return Err(CliError::MissingArgumentValue(arg.to_string()));
                }
                let value = args[i + 1].as_ref();
                if value.is_empty() || value.starts_with('-') {
                    return Err(CliError::MissingArgumentValue(arg.to_string()));
                }
                path = Some(value.to_string());
                i += 1;
            }
            opt if opt.starts_with("--path=") => {
                let value = &opt["--path=".len()..];
                if value.is_empty() {
                    return Err(CliError::MissingArgumentValue("--path".to_string()));
                }
                path = Some(value.to_string());
            }
            opt if opt.starts_with("-p=") => {
                let value = &opt["-p=".len()..];
                if value.is_empty() {
                    return Err(CliError::MissingArgumentValue("-p".to_string()));
                }
                path = Some(value.to_string());
            }
            "--dry-run" => dry_run = true,
            "--force" => force = true,
            unexpected => return Err(CliError::UnexpectedArgument(unexpected.to_string())),
        }
        i += 1;
    }

    Ok(Action::Import(ImportArgs {
        path,
        dry_run,
        force,
    }))
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

pub fn parse_toolchain_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut language = None;
    let mut i = 0;

    while i < args.len() {
        let arg = args[i].as_ref();
        match arg {
            "-h" | "--help" => return Ok(Action::ToolchainHelp),
            "-l" | "--language" => {
                if i + 1 < args.len() {
                    let val = args[i + 1].as_ref();
                    if val.starts_with('-') {
                        return Err(CliError::MissingArgumentValue(arg.to_string()));
                    }
                    language = Some(val.to_string());
                    i += 1;
                } else {
                    return Err(CliError::MissingArgumentValue(arg.to_string()));
                }
            }
            opt if opt.starts_with("--language=") => {
                let val = &opt["--language=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("--language".to_string()));
                }
                language = Some(val.to_string());
            }
            opt if opt.starts_with("-l=") => {
                let val = &opt["-l=".len()..];
                if val.is_empty() {
                    return Err(CliError::MissingArgumentValue("-l".to_string()));
                }
                language = Some(val.to_string());
            }
            unexpected => return Err(CliError::UnexpectedArgument(unexpected.to_string())),
        }
        i += 1;
    }

    Ok(Action::Toolchain(ToolchainArgs { language }))
}

pub fn parse_add_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut spec: Option<String> = None;

    for arg in args {
        let s = arg.as_ref();
        match s {
            "-h" | "--help" => return Ok(Action::AddHelp),
            opt if opt.starts_with('-') => {
                return Err(CliError::UnexpectedArgument(opt.to_string()));
            }
            pos => {
                if spec.is_none() {
                    spec = Some(pos.to_string());
                } else {
                    return Err(CliError::UnexpectedArgument(format!(
                        "unexpected argument '{pos}' — 'crux add' takes a single package spec"
                    )));
                }
            }
        }
    }

    match spec {
        Some(s) => Ok(Action::Add(AddArgs { spec: s })),
        None => Err(CliError::UnexpectedArgument(
            "missing package spec; usage: crux add <package>[@<constraint>]".to_string(),
        )),
    }
}

pub fn parse_remove_args<T: AsRef<str>>(args: &[T]) -> Result<Action, CliError> {
    let mut name: Option<String> = None;

    for arg in args {
        let s = arg.as_ref();
        match s {
            "-h" | "--help" => return Ok(Action::RemoveHelp),
            opt if opt.starts_with('-') => {
                return Err(CliError::UnexpectedArgument(opt.to_string()));
            }
            pos => {
                if name.is_none() {
                    name = Some(pos.to_string());
                } else {
                    return Err(CliError::UnexpectedArgument(format!(
                        "unexpected argument '{pos}' — 'crux remove' takes a single package name"
                    )));
                }
            }
        }
    }

    match name {
        Some(n) => Ok(Action::Remove(RemoveArgs { name: n })),
        None => Err(CliError::UnexpectedArgument(
            "missing package name; usage: crux remove <package>".to_string(),
        )),
    }
}
