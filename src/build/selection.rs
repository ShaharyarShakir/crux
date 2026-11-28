//! Compiler and ambient-flag selection for package builds.
//!
//! The selection is resolved once per request and used for the fingerprint, the
//! build invocation and the build metadata, so the three cannot disagree. No
//! shell is involved in resolving or running a compiler.
//!
//! How compilers are named on a build system's command line, and what a
//! configured build tree says it used, belong to the build system adapters in
//! [`super::systems`].

use super::BuildError;
use super::metadata::{BuildConfiguration, SelectedCompiler, SelectedToolchain};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Environment variables build systems initialize compiler and linker flags from.
const FLAG_VARIABLES: [&str; 3] = ["CFLAGS", "CXXFLAGS", "LDFLAGS"];

/// Inherited paths that decide what `find_package`, `find_library` and
/// pkg-config discover.
pub(crate) const SEARCH_PATH_VARIABLES: [&str; 2] = ["CMAKE_PREFIX_PATH", "PKG_CONFIG_PATH"];

/// Replaces the process environment for compiler selection, when set.
pub(crate) type AmbientEnv = BTreeMap<OsString, OsString>;

struct Language {
    env_var: &'static str,
    default: &'static str,
    /// Tried in this order, only when the default is unavailable.
    fallbacks: [&'static str; 2],
}

const C: Language = Language {
    env_var: "CC",
    default: "gcc",
    fallbacks: ["cc", "clang"],
};

const CXX: Language = Language {
    env_var: "CXX",
    default: "g++",
    fallbacks: ["c++", "clang++"],
};

/// What the build system's own options say about the toolchain. They take
/// precedence over everything Crux would otherwise select.
pub(crate) struct CompilerDeclarations<'a> {
    /// The option naming the C compiler and its value, when the options set it.
    pub c: Option<(&'static str, &'a str)>,
    /// As `c`, for C++.
    pub cxx: Option<(&'static str, &'a str)>,
    /// Environment variable naming a toolchain file.
    pub toolchain_file_variable: &'static str,
    /// Whether the options themselves name a toolchain file.
    pub declares_toolchain_file: bool,
}

/// The compiler chosen for one language.
#[derive(Debug, Default)]
pub(crate) struct CompilerChoice {
    pub compiler: Option<SelectedCompiler>,
    /// First line of the compiler's `--version` output.
    pub version: Option<String>,
    /// Whether the fingerprint must name this compiler explicitly, because the
    /// pre-existing fingerprint bytes would not identify it.
    identify: bool,
    /// Whether Crux must name the compiler to the build system itself.
    pub pass: bool,
}

/// Everything about the toolchain environment that one build depends on.
#[derive(Debug)]
pub(crate) struct ToolchainSelection {
    pub c: CompilerChoice,
    pub cxx: CompilerChoice,
    /// `CFLAGS`, `CXXFLAGS` and `LDFLAGS` that come from the ambient
    /// environment (`extra_env` values are fingerprinted separately).
    flags: BTreeMap<&'static str, Option<OsString>>,
    /// The effective [`SEARCH_PATH_VARIABLES`] before Crux adds anything, and
    /// whether each is already fingerprinted as part of `extra_env`.
    search_paths: BTreeMap<&'static str, (Option<OsString>, bool)>,
    /// A toolchain file may choose the compilers itself.
    toolchain_file: bool,
    toolchain_file_variable: &'static str,
    ambient: Option<AmbientEnv>,
}

impl ToolchainSelection {
    pub(crate) fn resolve(
        config: &BuildConfiguration,
        engine_c: Option<&Path>,
        engine_cxx: Option<&Path>,
        ambient: Option<&AmbientEnv>,
        declared: &CompilerDeclarations<'_>,
    ) -> Result<Self, BuildError> {
        let lookup = |key: &str| -> Option<OsString> {
            if let Some(value) = config.extra_env.get(key) {
                return Some(value.into());
            }
            match ambient {
                Some(env) => env.get(OsStr::new(key)).cloned(),
                None => std::env::var_os(key),
            }
        };
        let path_var = lookup("PATH");
        let toolchain_file = declared.declares_toolchain_file
            || lookup(declared.toolchain_file_variable).is_some_and(|v| !v.is_empty());

        let choose = |language: &Language,
                      definition: Option<(&'static str, &str)>,
                      explicit: Option<&PathBuf>,
                      engine: Option<&Path>| {
            choose_compiler(
                language,
                definition,
                explicit.map(PathBuf::as_path),
                engine,
                lookup(language.env_var),
                path_var.as_deref(),
                toolchain_file,
            )
        };
        let c = choose(&C, declared.c, config.c_compiler.as_ref(), engine_c)?;
        let cxx = choose(&CXX, declared.cxx, config.cxx_compiler.as_ref(), engine_cxx)?;

        let flags = FLAG_VARIABLES
            .into_iter()
            .filter(|key| !config.extra_env.contains_key(*key))
            .map(|key| (key, lookup(key)))
            .collect();

        let search_paths = SEARCH_PATH_VARIABLES
            .into_iter()
            .map(|key| (key, (lookup(key), config.extra_env.contains_key(key))))
            .collect();

        Ok(Self {
            c,
            cxx,
            flags,
            search_paths,
            toolchain_file,
            toolchain_file_variable: declared.toolchain_file_variable,
            ambient: ambient.cloned(),
        })
    }

    /// The value of a search path variable the build inherits.
    pub(crate) fn search_path(&self, key: &str) -> Option<&OsString> {
        self.search_paths.get(key)?.0.as_ref()
    }

    /// Bytes appended to the build fingerprint. Empty for a clean environment
    /// using the default compilers, which keeps existing fingerprints.
    pub(crate) fn fingerprint_fields(&self) -> Vec<(&'static str, &[u8])> {
        let mut fields = Vec::new();
        for (path_label, arg_label, choice) in [
            ("c.path", "c.arg", &self.c),
            ("cxx.path", "cxx.arg", &self.cxx),
        ] {
            if let Some(compiler) = choice.compiler.as_ref().filter(|_| choice.identify) {
                fields.push((path_label, compiler.path.as_os_str().as_encoded_bytes()));
                for arg in &compiler.args {
                    fields.push((arg_label, arg.as_bytes()));
                }
            }
        }
        for (key, value) in &self.flags {
            if let Some(value) = value {
                fields.push((key, value.as_encoded_bytes()));
            }
        }
        // Unset adds nothing; an empty value adds an empty field.
        for (key, (value, in_extra_env)) in &self.search_paths {
            if let Some(value) = value.as_ref().filter(|_| !in_extra_env) {
                fields.push((key, value.as_encoded_bytes()));
            }
        }
        fields
    }

    /// Gives `command` the same flag values that were fingerprinted.
    pub(crate) fn apply_environment(&self, command: &mut Command) {
        fn set(command: &mut Command, key: &str, value: Option<&OsString>) {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        for (key, value) in &self.flags {
            set(command, key, value.as_ref());
        }
        for (key, (value, _)) in &self.search_paths {
            set(command, key, value.as_ref());
        }
        if let Some(ambient) = &self.ambient {
            for key in ["CC", "CXX", self.toolchain_file_variable] {
                set(command, key, ambient.get(OsStr::new(key)));
            }
            if let Some(path) = ambient.get(OsStr::new("PATH")) {
                set(command, "PATH", Some(path));
            }
        }
    }

    /// What to record in the build metadata.
    pub(crate) fn record(&self) -> SelectedToolchain {
        let used = |choice: &CompilerChoice| {
            choice
                .compiler
                .clone()
                .filter(|_| choice.pass || !self.toolchain_file)
        };
        SelectedToolchain {
            c_compiler: used(&self.c),
            cxx_compiler: used(&self.cxx),
            ambient_flags: self
                .flags
                .iter()
                .filter_map(|(key, value)| {
                    let value = value.as_ref()?.to_string_lossy().into_owned();
                    Some((key.to_string(), value))
                })
                .collect(),
        }
    }

    /// Whether a recorded selection used other compilers than this one.
    pub(crate) fn differs_from(&self, recorded: &SelectedToolchain) -> bool {
        let current = self.record();
        [
            (&recorded.c_compiler, &current.c_compiler),
            (&recorded.cxx_compiler, &current.cxx_compiler),
        ]
        .into_iter()
        .any(|pair| match pair {
            (Some(old), Some(new)) => !same_compiler(old, new),
            _ => false,
        })
    }

    /// Whether the flag variable `key` is captured here and unset.
    pub(crate) fn flag_is_unset(&self, key: &str) -> bool {
        matches!(self.flags.get(key), Some(None))
    }

    /// Whether a toolchain file may choose the compilers.
    pub(crate) fn uses_toolchain_file(&self) -> bool {
        self.toolchain_file
    }
}

fn choose_compiler(
    language: &Language,
    definition: Option<(&'static str, &str)>,
    explicit: Option<&Path>,
    engine: Option<&Path>,
    env_value: Option<OsString>,
    path_var: Option<&OsStr>,
    toolchain_file: bool,
) -> Result<CompilerChoice, BuildError> {
    let default = find_program(Path::new(language.default), path_var)
        .and_then(|path| inspect(path, Vec::new()));
    let choice =
        |(compiler, version): (SelectedCompiler, String), verbatim: Option<&OsStr>, pass: bool| {
            let is_default = default.as_ref().is_some_and(|(d, _)| *d == compiler);
            let already_hashed = compiler.args.is_empty()
                && verbatim.is_some_and(|raw| raw == compiler.path.as_os_str());
            CompilerChoice {
                identify: !is_default && !already_hashed,
                compiler: Some(compiler),
                version: Some(version),
                pass,
            }
        };
    let required = |program: &Path, args: Vec<String>, origin: String| {
        find_program(program, path_var)
            .and_then(|path| inspect(path, args))
            .ok_or_else(|| {
                BuildError::ToolchainDetectionFailed(format!(
                    "compiler specified by {origin} was not found, is not executable, \
                     or failed to run '--version'"
                ))
            })
    };

    // 1. A definition in the build system's options wins inside it, so it wins here.
    if let Some((option, value)) = definition {
        let mut parts = value.split(';').map(str::to_string);
        let program = parts.next().unwrap_or_default();
        let origin = format!("CMake argument {option}='{value}'");
        let found = required(Path::new(&program), parts.collect(), origin)?;
        return Ok(choice(found, Some(OsStr::new(&program)), false));
    }

    // 2. Explicit Crux override.
    if let Some(path) = explicit.or(engine) {
        let origin = format!("the Crux compiler override '{}'", path.display());
        let found = required(path, Vec::new(), origin)?;
        return Ok(choice(found, explicit.map(Path::as_os_str), true));
    }

    // 3. CC / CXX. The whole value may be a path with spaces; otherwise it is a
    //    program followed by arguments, split on whitespace as CMake accepts.
    if let Some(raw) = env_value {
        let text = raw.to_string_lossy();
        let text = text.trim();
        if !text.is_empty() {
            let origin = format!("{}='{text}'", language.env_var);
            let (program, args) = if find_program(Path::new(&raw), path_var).is_some() {
                (PathBuf::from(&raw), Vec::new())
            } else {
                let mut words = text.split_whitespace();
                let program = PathBuf::from(words.next().unwrap_or_default());
                (program, words.map(str::to_string).collect())
            };
            let found = required(&program, args, origin)?;
            return Ok(choice(found, None, !toolchain_file));
        }
    }

    // 4. GCC, then the fallbacks. With none, CMake is left to search on its own.
    let found = default.clone().or_else(|| {
        language.fallbacks.iter().find_map(|name| {
            find_program(Path::new(name), path_var).and_then(|path| inspect(path, Vec::new()))
        })
    });
    Ok(match found {
        Some(found) => choice(found, None, !toolchain_file),
        None => CompilerChoice::default(),
    })
}

/// Resolves `program` to an absolute executable path, without following
/// symlinks (a wrapper such as ccache dispatches on the name it is run by).
fn find_program(program: &Path, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let candidate = if program.components().count() > 1 || program.is_absolute() {
        Some(program.to_path_buf()).filter(|p| is_executable(p))
    } else if program.as_os_str().is_empty() {
        None
    } else {
        std::env::split_paths(path_var?)
            .map(|dir| dir.join(program))
            .find(|p| is_executable(p))
    };
    std::path::absolute(candidate?).ok()
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn inspect(path: PathBuf, args: Vec<String>) -> Option<(SelectedCompiler, String)> {
    let output = Command::new(&path)
        .args(&args)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    Some((SelectedCompiler { path, args }, version))
}

/// Whether two selections run the same compiler: the same path and arguments,
/// or aliases of one binary (such as `cc` and `gcc`) reporting the same
/// version after the program name.
pub(crate) fn same_compiler(a: &SelectedCompiler, b: &SelectedCompiler) -> bool {
    if a.args != b.args {
        return false;
    }
    if a.path == b.path {
        return true;
    }
    let release = |c: &SelectedCompiler| {
        let (_, version) = inspect(c.path.clone(), c.args.clone())?;
        let release = version.split_once(' ').map(|(_, rest)| rest.to_string());
        Some((fs::canonicalize(&c.path).ok()?, release.unwrap_or(version)))
    };
    matches!((release(a), release(b)), (Some(a), Some(b)) if a == b)
}
