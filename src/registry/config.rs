//! Registry configuration file models and persistent state management.

use super::RegistryError;
use super::git::{GitOps, GitRegistry};
use super::metadata::{PACKAGE_DEFINITION_FILENAME, REGISTRY_MANIFEST_FILENAME};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Canonical Git URL of the official Crux registry repository.
///
/// The registry itself lives in the `registry/` subdirectory of this
/// repository, so cloning the repository once and opening the subdirectory
/// gives access to all official package definitions.
pub const OFFICIAL_REGISTRY_URL: &str = "https://github.com/ShaharyarShakir/crux.git";

/// Name used for the automatically configured official registry.
pub const OFFICIAL_REGISTRY_NAME: &str = "default";

/// Subdirectory of the official repository that contains the registry.
pub const OFFICIAL_REGISTRY_SUBDIR: &str = "registry";

/// Directory name used for the shared official registry checkout.
pub const OFFICIAL_REGISTRY_DIR_NAME: &str = "official";

/// Persistent registry configuration for Crux (`.crux/config.toml` or `~/.crux/config.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CruxConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<PathBuf>,
    #[serde(default)]
    pub registries: BTreeMap<String, RegistryConfig>,
}

/// Configuration entry for a single package registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Path to the Git checkout that contains the registry.
    pub path: PathBuf,
    /// Optional registry root relative to `path` (for monorepos such as the
    /// official Crux repository, where the registry lives in `registry/`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdir: Option<PathBuf>,
}

impl RegistryConfig {
    /// Returns the effective registry root directory (`path` joined with `subdir`).
    pub fn registry_root(&self) -> PathBuf {
        match &self.subdir {
            Some(subdir) => self.path.join(subdir),
            None => self.path.clone(),
        }
    }

    /// Validates that the configured subdirectory is relative and cannot escape `path`.
    pub fn validate(&self) -> Result<(), RegistryError> {
        if let Some(subdir) = &self.subdir {
            if subdir.as_os_str().is_empty() {
                return Err(RegistryError::Validation(
                    "registry subdirectory cannot be empty".to_string(),
                ));
            }
            for component in subdir.components() {
                match component {
                    std::path::Component::Normal(_) => {}
                    std::path::Component::CurDir => {}
                    other => {
                        return Err(RegistryError::SecurityViolation(format!(
                            "registry subdirectory '{}' must be a relative path without '..', root, or prefix components (found {other:?})",
                            subdir.display()
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

impl CruxConfig {
    /// Returns the primary registry to use (defaults to "default", or the first configured one).
    pub fn get_primary_registry(&self) -> Option<(&str, &RegistryConfig)> {
        if let Some(cfg) = self.registries.get("default") {
            return Some(("default", cfg));
        }
        self.registries.iter().next().map(|(k, v)| (k.as_str(), v))
    }
}

/// Resolves the path to the configuration file.
pub fn config_file_path(project_dir: Option<&Path>) -> PathBuf {
    if let Ok(env_path) = std::env::var("CRUX_CONFIG_PATH") {
        return PathBuf::from(env_path);
    }

    if let Some(p) = project_dir {
        let local_config = p.join(".crux").join("config.toml");
        if local_config.is_file() || p.join(crate::manifest::MANIFEST_FILENAME).is_file() {
            return local_config;
        }
    }

    if let Ok(crux_home) = std::env::var("CRUX_HOME") {
        return PathBuf::from(crux_home).join("config.toml");
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".crux").join("config.toml");
    }

    PathBuf::from(".crux").join("config.toml")
}

/// Resolves the base directory where cloned registries are stored.
pub fn registries_dir_path(project_dir: Option<&Path>) -> PathBuf {
    if let Ok(env_dir) = std::env::var("CRUX_REGISTRIES_DIR") {
        return PathBuf::from(env_dir);
    }

    if let Ok(crux_home) = std::env::var("CRUX_HOME") {
        return PathBuf::from(crux_home).join("registries");
    }

    if let Some(p) = project_dir {
        let manifest_candidate = p.join(crate::manifest::MANIFEST_FILENAME);
        if manifest_candidate.is_file() {
            return p.join(".crux").join("registries");
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".crux").join("registries");
    }

    PathBuf::from(".crux").join("registries")
}

/// Resolves the user-level directory used to cache the official registry checkout.
///
/// The official registry is shared across projects. `CRUX_REGISTRIES_DIR` and
/// `CRUX_HOME` are honored so tests and sandboxed environments remain isolated.
pub fn official_registry_cache_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var("CRUX_REGISTRIES_DIR") {
        let path = PathBuf::from(env_dir.trim());
        if !path.as_os_str().is_empty() {
            return path;
        }
    }

    if let Ok(crux_home) = std::env::var("CRUX_HOME") {
        let path = PathBuf::from(crux_home.trim());
        if !path.as_os_str().is_empty() {
            return path.join("registries");
        }
    }

    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        let path = PathBuf::from(xdg.trim());
        if !path.as_os_str().is_empty() {
            return path.join("crux").join("registries");
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let path = PathBuf::from(home.trim());
        if !path.as_os_str().is_empty() {
            return path.join(".crux").join("registries");
        }
    }

    std::env::temp_dir().join("crux-registries")
}

/// Returns the Git URL to clone for the official registry.
///
/// `CRUX_OFFICIAL_REGISTRY_URL` may be used to point at a mirror (for example
/// in air-gapped or offline test environments).
pub fn official_registry_url() -> String {
    std::env::var("CRUX_OFFICIAL_REGISTRY_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OFFICIAL_REGISTRY_URL.to_string())
}

/// Returns the repository checkout that ships with the Crux source tree, if it exists.
///
/// This makes development builds and the test suite work offline while still
/// using the exact same `registry/` content that the published repository
/// exposes. Released binaries normally will not have this directory, so they
/// transparently fall back to cloning the official repository.
fn bundled_official_registry_root() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry_root = manifest_dir.join(OFFICIAL_REGISTRY_SUBDIR);
    if registry_root.join(REGISTRY_MANIFEST_FILENAME).is_file()
        && GitOps::is_git_repository(&manifest_dir)
    {
        Some(manifest_dir)
    } else {
        None
    }
}

/// Ensures that at least one registry is configured, bootstrapping the official
/// Crux registry when no registry exists.
///
/// Custom registry configuration is never overwritten. An existing official
/// checkout is reused without contacting the network, which makes subsequent
/// runs (and fully offline runs) fast and reliable.
pub fn ensure_official_registry(
    project_dir: Option<&Path>,
) -> Result<(String, RegistryConfig), RegistryError> {
    let mut config = load_config(project_dir)?;

    if let Some((name, cfg)) = config.get_primary_registry() {
        cfg.validate()?;
        return Ok((name.to_string(), cfg.clone()));
    }

    let subdir = PathBuf::from(OFFICIAL_REGISTRY_SUBDIR);
    let cached_checkout = official_registry_cache_dir().join(OFFICIAL_REGISTRY_DIR_NAME);

    let checkout = if cached_checkout
        .join(&subdir)
        .join(REGISTRY_MANIFEST_FILENAME)
        .is_file()
        && GitOps::is_git_repository(&cached_checkout)
    {
        cached_checkout
    } else if let Some(bundled) = bundled_official_registry_root() {
        bundled
    } else {
        if cached_checkout.exists() && !GitOps::is_git_repository(&cached_checkout) {
            return Err(RegistryError::Validation(format!(
                "official registry cache '{}' exists but is not a valid Git repository; remove it and retry",
                cached_checkout.display()
            )));
        }

        if let Some(parent) = cached_checkout.parent() {
            fs::create_dir_all(parent)?;
        }

        let url = official_registry_url();
        GitOps::clone_repository(&url, &cached_checkout).map_err(|err| {
            RegistryError::Git(format!(
                "failed to bootstrap the official Crux registry from '{url}': {err}\n\
                 If you are offline, configure an existing checkout with \
                 'crux registry add <name> <git-url>' or set CRUX_OFFICIAL_REGISTRY_URL."
            ))
        })?;
        cached_checkout
    };

    // Validate the checkout before persisting it so a broken clone is never
    // recorded in configuration.
    GitRegistry::open_with_subdir(&checkout, Some(&subdir))?;

    let registry_config = RegistryConfig {
        url: Some(official_registry_url()),
        path: checkout,
        subdir: Some(subdir),
    };

    config
        .registries
        .insert(OFFICIAL_REGISTRY_NAME.to_string(), registry_config.clone());
    save_config(&config, &config_file_path(project_dir))?;

    Ok((OFFICIAL_REGISTRY_NAME.to_string(), registry_config))
}

/// Loads Crux configuration from disk. Returns default empty config if file doesn't exist.
pub fn load_config(project_dir: Option<&Path>) -> Result<CruxConfig, RegistryError> {
    let cfg_path = config_file_path(project_dir);
    if !cfg_path.is_file() {
        return Ok(CruxConfig::default());
    }

    let content = fs::read_to_string(&cfg_path)?;
    let config: CruxConfig = toml::from_str(&content).map_err(|e| {
        RegistryError::Toml(format!(
            "failed to parse configuration file '{}': {e}",
            cfg_path.display()
        ))
    })?;

    Ok(config)
}

/// Atomically saves Crux configuration to disk.
pub fn save_config(config: &CruxConfig, path: &Path) -> Result<(), RegistryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let toml_str = toml::to_string_pretty(config)
        .map_err(|e| RegistryError::Toml(format!("failed to serialize configuration: {e}")))?;

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let temp_name = format!(".config.tmp.{pid}.{nanos}");
    let temp_path = parent.join(temp_name);

    {
        use std::io::Write;
        let mut f = fs::File::create(&temp_path)?;
        f.write_all(toml_str.as_bytes())?;
        f.flush()?;
        f.sync_data()?;
    }

    fs::rename(&temp_path, path)?;
    Ok(())
}

/// Adds and clones a new registry.
pub fn add_registry(
    name: &str,
    git_url: &str,
    project_dir: Option<&Path>,
) -> Result<GitRegistry, RegistryError> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(RegistryError::Validation(
            "registry name cannot be empty".to_string(),
        ));
    }
    if !trimmed_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(RegistryError::Validation(format!(
            "invalid registry name '{trimmed_name}': must contain only alphanumeric characters, '-', or '_'"
        )));
    }

    let mut config = load_config(project_dir)?;
    if config.registries.contains_key(trimmed_name) {
        return Err(RegistryError::AlreadyExists(trimmed_name.to_string()));
    }

    let target_dir = registries_dir_path(project_dir).join(trimmed_name);
    if target_dir.exists() {
        if !GitOps::is_git_repository(&target_dir) {
            return Err(RegistryError::Validation(format!(
                "target registry directory '{}' exists but is not a valid Git repository",
                target_dir.display()
            )));
        }
    } else {
        if let Some(parent) = target_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        GitOps::clone_repository(git_url, &target_dir)?;
    }

    // Validate the cloned repository by opening it
    let registry = match GitRegistry::open(&target_dir) {
        Ok(reg) => reg,
        Err(err) => {
            let _ = fs::remove_dir_all(&target_dir);
            return Err(err);
        }
    };

    // Save to configuration
    config.registries.insert(
        trimmed_name.to_string(),
        RegistryConfig {
            url: Some(git_url.to_string()),
            path: target_dir,
            subdir: None,
        },
    );
    save_config(&config, &config_file_path(project_dir))?;

    Ok(registry)
}

/// Status report for a configured package registry.
#[derive(Debug)]
pub struct RegistryStatus {
    pub name: String,
    pub config: RegistryConfig,
    pub registry: Result<GitRegistry, RegistryError>,
}

/// Lists all configured registries.
pub fn list_registries(project_dir: Option<&Path>) -> Result<Vec<RegistryStatus>, RegistryError> {
    let config = load_config(project_dir)?;
    let mut list = Vec::new();
    for (name, reg_cfg) in config.registries {
        let reg_result = reg_cfg
            .validate()
            .and_then(|()| GitRegistry::open_configured(&reg_cfg));
        list.push(RegistryStatus {
            name,
            config: reg_cfg,
            registry: reg_result,
        });
    }
    Ok(list)
}

/// Updates an existing registry from upstream.
pub fn update_registry(
    name: &str,
    project_dir: Option<&Path>,
) -> Result<(GitRegistry, String), RegistryError> {
    let config = load_config(project_dir)?;
    let reg_cfg = config
        .registries
        .get(name)
        .ok_or_else(|| RegistryError::NotFound(name.to_string()))?;
    reg_cfg.validate()?;

    let head = GitOps::update_repository(&reg_cfg.path)?;
    let registry = GitRegistry::open_configured(reg_cfg)?;

    Ok((registry, head))
}

/// Seeds a directory with verified initial package definitions (fmt, zlib, catch2) and registry.toml.
pub fn seed_registry_files(dir: &Path) -> Result<(), io::Error> {
    fs::create_dir_all(dir)?;

    fs::write(
        dir.join(REGISTRY_MANIFEST_FILENAME),
        "format_version = 1\nname = \"default\"\ndescription = \"Official Crux package registry\"\n",
    )?;

    let pkgs = [
        (
            "fmt",
            "11.0.2",
            r#"[package]
name = "fmt"
version = "11.0.2"
description = "A modern formatting library"
license = "MIT"
homepage = "https://fmt.dev"
repository = "https://github.com/fmtlib/fmt"

[source]
type = "archive"
url = "https://github.com/fmtlib/fmt/releases/download/11.0.2/fmt-11.0.2.zip"
checksum = "sha256:40fc58bebcf38c759e11a7bd8fdc163507d2423ef5058bba7f26280c5b9c5465"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "fmt",
            "11.1.1",
            r#"[package]
name = "fmt"
version = "11.1.1"
description = "A modern formatting library"
license = "MIT"
homepage = "https://fmt.dev"
repository = "https://github.com/fmtlib/fmt"

[source]
type = "archive"
url = "https://github.com/fmtlib/fmt/releases/download/11.1.1/fmt-11.1.1.zip"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "zlib",
            "1.3.0",
            r#"[package]
name = "zlib"
version = "1.3.0"
description = "A Massively Spiffy Yet Delicately Unobtrusive Compression Library"
license = "Zlib"
homepage = "https://zlib.net"
repository = "https://github.com/madler/zlib"

[source]
type = "archive"
url = "https://github.com/madler/zlib/releases/download/v1.3/zlib-1.3.tar.gz"
checksum = "sha256:ff0ba4c292013dbc27530b3a81e1f9a813cd39de01ca5e0f8bf355702efa593e"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "zlib",
            "1.3.1",
            r#"[package]
name = "zlib"
version = "1.3.1"
description = "A Massively Spiffy Yet Delicately Unobtrusive Compression Library"
license = "Zlib"
homepage = "https://zlib.net"
repository = "https://github.com/madler/zlib"

[source]
type = "archive"
url = "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz"
checksum = "sha256:9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "catch2",
            "3.7.1",
            r#"[package]
name = "catch2"
version = "3.7.1"
description = "A modern, C++-native, test framework for unit-tests, TDD and BDD"
license = "BSL-1.0"
homepage = "https://github.com/catchorg/Catch2"
repository = "https://github.com/catchorg/Catch2"

[source]
type = "archive"
url = "https://github.com/catchorg/Catch2/archive/refs/tags/v3.7.1.tar.gz"
checksum = "sha256:1542950f7d0376938841218a41cb5b1fc138f6262603e07d0062226012fbd577"

[build]
system = "cmake"

[dependencies]
"#,
        ),
        (
            "catch2",
            "3.8.0",
            r#"[package]
name = "catch2"
version = "3.8.0"
description = "A modern, C++-native, test framework for unit-tests, TDD and BDD"
license = "BSL-1.0"
homepage = "https://github.com/catchorg/Catch2"
repository = "https://github.com/catchorg/Catch2"

[source]
type = "archive"
url = "https://github.com/catchorg/Catch2/archive/refs/tags/v3.8.0.tar.gz"
checksum = "sha256:1ab2de20460d4641553addfdfe6acd4109d871d5531f8f519a52ea4926303087"

[build]
system = "cmake"

[dependencies]
"#,
        ),
    ];

    for (name, ver, content) in pkgs {
        let pkg_dir = dir.join("packages").join(name).join(ver);
        fs::create_dir_all(&pkg_dir)?;
        fs::write(pkg_dir.join(PACKAGE_DEFINITION_FILENAME), content)?;
    }

    Ok(())
}

/// Initializes a directory as a Git repository and commits the seeded registry definitions.
pub fn init_test_git_registry(dir: &Path) -> Result<GitRegistry, RegistryError> {
    seed_registry_files(dir)?;

    let run_git = |args: &[&str]| -> Result<(), RegistryError> {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(|e| RegistryError::Git(format!("git command failed: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(RegistryError::Git(format!(
                "git {} failed: {err}",
                args.join(" ")
            )));
        }
        Ok(())
    };

    run_git(&["init", "-b", "main"])?;
    run_git(&["config", "user.name", "Crux Test"])?;
    run_git(&["config", "user.email", "test@crux.invalid"])?;
    run_git(&["add", "."])?;
    run_git(&["commit", "-m", "Initial test registry commit"])?;

    GitRegistry::open(dir)
}
