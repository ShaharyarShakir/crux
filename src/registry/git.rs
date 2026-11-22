//! Git repository inspection and safe execution operations.

use super::RegistryError;
use super::metadata::{
    CURRENT_REGISTRY_FORMAT_VERSION, PACKAGE_DEFINITION_FILENAME, PackageDefinition,
    PackageSourceDefinition, REGISTRY_MANIFEST_FILENAME, RegistryManifest,
    parse_package_definition,
};
use crate::catalog::{CatalogEntry, CatalogError, PackageCatalog};
use crate::core::process::run_with_timeout;
use crate::lock::PackageSource;
use crate::package::{PackageId, PackageName};
use crate::resolution::GitRef;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Remote name used for every source-acquisition clone.
const CLONE_REMOTE_NAME: &str = "origin";

/// Git transports Crux permits, as a `GIT_ALLOW_PROTOCOL` value. `ssh` also
/// covers SCP-style `user@host:path`; `file` covers local paths.
pub const ALLOWED_GIT_PROTOCOLS: &str = "https:ssh:file";

/// Upper bound for registry clone/pull operations (network, possibly large).
pub const REGISTRY_NETWORK_TIMEOUT: Duration = Duration::from_secs(600);

/// Upper bound for local, read-only Git queries such as `rev-parse`.
pub const LOCAL_GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Returns the `rev-parse --verify` expression that resolves `requested` to a
/// commit inside a fresh `clone --no-checkout`.
///
/// Such a clone only creates a local branch for the remote's default branch;
/// every other branch exists solely as `refs/remotes/origin/<name>`. Branches
/// and tags are therefore resolved through fully qualified refs, which also
/// prevents a tag and a branch with the same name from being confused.
fn ref_resolution_expression(requested: &GitRef) -> String {
    match requested {
        GitRef::Head => "HEAD^{commit}".to_string(),
        GitRef::Branch(value) => format!("refs/remotes/{CLONE_REMOTE_NAME}/{value}^{{commit}}"),
        GitRef::Tag(value) => format!("refs/tags/{value}^{{commit}}"),
        GitRef::Rev(value) | GitRef::Commit(value) => format!("{value}^{{commit}}"),
    }
}

/// A local Git-backed Crux package registry.
#[derive(Debug, Clone)]
pub struct GitRegistry {
    name: String,
    path: PathBuf,
    manifest: RegistryManifest,
    packages: BTreeMap<PackageName, BTreeMap<semver::Version, PackageDefinition>>,
    catalog_entries: BTreeMap<PackageName, BTreeMap<semver::Version, CatalogEntry>>,
}

impl GitRegistry {
    /// Opens a registry described by a [`RegistryConfig`], honoring an optional
    /// registry subdirectory inside the configured Git checkout.
    pub fn open_configured(config: &super::config::RegistryConfig) -> Result<Self, RegistryError> {
        config.validate()?;
        Self::open_with_subdir(&config.path, config.subdir.as_deref())
    }

    /// Opens a registry stored in `path`, optionally below a relative subdirectory.
    ///
    /// The Git repository check is performed against the resolved registry root,
    /// which is also inside the enclosing Git work tree for monorepo layouts such
    /// as the official Crux repository (`<repo>/registry`).
    pub fn open_with_subdir(
        path: impl AsRef<Path>,
        subdir: Option<&Path>,
    ) -> Result<Self, RegistryError> {
        let base = path.as_ref();
        let registry_root = match subdir {
            Some(subdir) => {
                if subdir.as_os_str().is_empty() {
                    return Err(RegistryError::Validation(
                        "registry subdirectory cannot be empty".to_string(),
                    ));
                }
                for component in subdir.components() {
                    match component {
                        std::path::Component::Normal(_) | std::path::Component::CurDir => {}
                        other => {
                            return Err(RegistryError::SecurityViolation(format!(
                                "registry subdirectory '{}' must be relative and cannot contain root, prefix, or parent components (found {other:?})",
                                subdir.display()
                            )));
                        }
                    }
                }
                base.join(subdir)
            }
            None => base.to_path_buf(),
        };

        Self::open(registry_root)
    }

    /// Opens and validates a local Git registry repository at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let registry_path = path.as_ref();
        if !registry_path.is_dir() {
            return Err(RegistryError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "registry directory '{}' does not exist",
                    registry_path.display()
                ),
            )));
        }

        // Canonicalize the registry path to resolve symlinks and ensure safe boundary checks
        let canonical_root = fs::canonicalize(registry_path)?;

        // Verify it is a valid Git repository
        if !GitOps::is_git_repository(&canonical_root) {
            return Err(RegistryError::NotAGitRepository(canonical_root));
        }

        // Load and validate registry.toml
        let manifest_path = canonical_root.join(REGISTRY_MANIFEST_FILENAME);
        if !manifest_path.is_file() {
            return Err(RegistryError::MissingRegistryManifest(manifest_path));
        }

        let manifest_str = fs::read_to_string(&manifest_path)?;
        let manifest: RegistryManifest = toml::from_str(&manifest_str).map_err(|e| {
            RegistryError::Toml(format!(
                "failed to parse '{}': {e}",
                manifest_path.display()
            ))
        })?;

        if manifest.format_version != CURRENT_REGISTRY_FORMAT_VERSION {
            return Err(RegistryError::UnsupportedFormatVersion(
                manifest.format_version,
            ));
        }

        if manifest.name.trim().is_empty() {
            return Err(RegistryError::Validation(
                "registry name in registry.toml cannot be empty".to_string(),
            ));
        }

        let mut packages = BTreeMap::new();
        let mut catalog_entries = BTreeMap::new();

        let packages_dir = canonical_root.join("packages");
        if fs::symlink_metadata(&packages_dir).is_ok() {
            check_path_safety(&packages_dir, &canonical_root)?;
        }
        if packages_dir.is_dir() {
            let mut pkg_entries = Vec::new();
            for entry in fs::read_dir(&packages_dir)? {
                pkg_entries.push(entry?);
            }
            // Deterministic lexicographical ordering
            pkg_entries.sort_by_key(|e| e.file_name());

            for pkg_entry in pkg_entries {
                let pkg_name_str = pkg_entry.file_name().to_string_lossy().to_string();
                if pkg_name_str.starts_with('.') {
                    continue;
                }

                let pkg_path = pkg_entry.path();
                check_path_safety(&pkg_path, &canonical_root)?;

                if !pkg_path.is_dir() {
                    return Err(RegistryError::Validation(format!(
                        "expected package directory in 'packages/', found file '{}'",
                        pkg_path.display()
                    )));
                }

                let package_name = PackageName::new(&pkg_name_str).map_err(|e| {
                    RegistryError::InvalidPackageName(format!(
                        "invalid package name '{pkg_name_str}': {e}"
                    ))
                })?;

                let mut ver_entries = Vec::new();
                for entry in fs::read_dir(&pkg_path)? {
                    ver_entries.push(entry?);
                }
                ver_entries.sort_by_key(|e| e.file_name());

                for ver_entry in ver_entries {
                    let ver_str = ver_entry.file_name().to_string_lossy().to_string();
                    if ver_str.starts_with('.') {
                        continue;
                    }

                    let ver_path = ver_entry.path();
                    check_path_safety(&ver_path, &canonical_root)?;

                    if !ver_path.is_dir() {
                        return Err(RegistryError::Validation(format!(
                            "expected version directory in '{}', found file '{}'",
                            pkg_path.display(),
                            ver_path.display()
                        )));
                    }

                    let version = semver::Version::parse(&ver_str).map_err(|e| {
                        RegistryError::InvalidVersion(format!(
                            "invalid semantic version directory '{ver_str}': {e}"
                        ))
                    })?;

                    let def_path = ver_path.join(PACKAGE_DEFINITION_FILENAME);
                    if !def_path.is_file() {
                        return Err(RegistryError::MissingPackageManifest(def_path));
                    }

                    check_path_safety(&def_path, &canonical_root)?;

                    let def_str = fs::read_to_string(&def_path)?;
                    let def = parse_package_definition(&def_str, &def_path, None)?;

                    // Identity consistency validation
                    if def.metadata.name.as_str() != pkg_name_str
                        || def.metadata.version.to_string() != ver_str
                    {
                        return Err(RegistryError::IdentityMismatch {
                            expected_name: pkg_name_str.clone(),
                            found_name: def.metadata.name.as_str().to_string(),
                            expected_version: ver_str.clone(),
                            found_version: def.metadata.version.to_string(),
                        });
                    }

                    let source_def = def.source.clone();
                    let dependencies = def.dependencies.clone();

                    let pkg_id = PackageId::new(package_name.clone(), version.clone());

                    let versions_map = packages
                        .entry(package_name.clone())
                        .or_insert_with(BTreeMap::new);
                    if versions_map.contains_key(&version) {
                        return Err(RegistryError::DuplicatePackage(pkg_id));
                    }
                    versions_map.insert(version.clone(), def);

                    let mut entry = CatalogEntry::new(package_name.clone(), version.clone())
                        .with_source(PackageSource::registry(&manifest.name))
                        .with_dependencies(dependencies);
                    if let PackageSourceDefinition::Archive { ref checksum, .. } = source_def {
                        entry = entry.with_checksum(checksum.clone());
                    }

                    let catalog_map = catalog_entries
                        .entry(package_name.clone())
                        .or_insert_with(BTreeMap::new);
                    catalog_map.insert(version, entry);
                }
            }
        }

        Ok(Self {
            name: manifest.name.clone(),
            path: canonical_root,
            manifest,
            packages,
            catalog_entries,
        })
    }

    /// Returns the name of the registry.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the physical filesystem path to the registry root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the registry manifest.
    pub fn manifest(&self) -> &RegistryManifest {
        &self.manifest
    }

    /// Returns the total number of distinct packages in the registry.
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Returns the total number of distinct package versions in the registry.
    pub fn total_version_count(&self) -> usize {
        self.packages.values().map(|v| v.len()).sum()
    }

    /// Returns a sorted list of all package names in the registry.
    pub fn package_names(&self) -> Vec<PackageName> {
        self.packages.keys().cloned().collect()
    }

    /// Returns a specific package definition if present.
    pub fn get_package_definition(&self, id: &PackageId) -> Option<&PackageDefinition> {
        self.packages.get(id.name())?.get(id.version())
    }
}

impl PackageCatalog for GitRegistry {
    fn get_package_versions(
        &self,
        package: &PackageName,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        match self.catalog_entries.get(package) {
            Some(versions) => Ok(versions.values().cloned().collect()),
            None => Ok(Vec::new()),
        }
    }
}

/// Validates that `path` is not a symbolic link pointing outside `canonical_root`.
pub(crate) fn check_path_safety(path: &Path, canonical_root: &Path) -> Result<(), RegistryError> {
    let symlink_meta = fs::symlink_metadata(path)?;
    if symlink_meta.file_type().is_symlink() {
        let target = fs::canonicalize(path).map_err(|e| {
            RegistryError::SecurityViolation(format!(
                "invalid symbolic link '{}': {e}",
                path.display()
            ))
        })?;
        if !target.starts_with(canonical_root) {
            return Err(RegistryError::SecurityViolation(format!(
                "symbolic link '{}' resolves to '{}', which is outside registry root '{}'",
                path.display(),
                target.display(),
                canonical_root.display()
            )));
        }
    }
    Ok(())
}

/// Safe Git operations using direct executable invocation without shell interpolation.
pub struct GitOps;

impl GitOps {
    /// Checks whether the directory is inside a Git work tree.
    pub fn is_git_repository(path: &Path) -> bool {
        let mut command = Self::policy_command();
        command
            .arg("-C")
            .arg(path)
            .arg("rev-parse")
            .arg("--is-inside-work-tree")
            .env("GIT_TERMINAL_PROMPT", "0");
        match run_with_timeout(&mut command, LOCAL_GIT_TIMEOUT) {
            Ok(out) => out.success() && String::from_utf8_lossy(&out.stdout).trim() == "true",
            Err(_) => false,
        }
    }

    /// Clones a Git repository from `url` into `target_path`.
    ///
    /// Used by `crux registry add`, an explicit user action: Git may still
    /// prompt for credentials on the terminal. See [`GitOps::clone_registry`].
    pub fn clone_repository(url: &str, target_path: &Path) -> Result<(), RegistryError> {
        Self::clone_registry(url, target_path, true)
    }

    /// Clones a registry repository with the registry URL policy applied.
    ///
    /// The URL is validated ([`crate::resolution::validate_registry_url`]),
    /// passed after `--` so it can never be parsed as a Git option, restricted
    /// to the allowed transports, and bounded by [`REGISTRY_NETWORK_TIMEOUT`].
    /// `interactive = false` additionally disables terminal credential prompts
    /// (used for the implicit official-registry bootstrap).
    pub fn clone_registry(
        url: &str,
        target_path: &Path,
        interactive: bool,
    ) -> Result<(), RegistryError> {
        let url = crate::resolution::validate_registry_url(url)
            .map_err(|err| RegistryError::Validation(err.to_string()))?;
        let mut command = Self::policy_command();
        if !interactive {
            command.env("GIT_TERMINAL_PROMPT", "0");
        }
        command.arg("clone").arg("--").arg(&url).arg(target_path);
        let output = run_with_timeout(&mut command, REGISTRY_NETWORK_TIMEOUT)
            .map_err(|e| RegistryError::Git(format!("failed to execute git clone: {e}")))?;
        let shown_url = crate::resolution::redact_url_credentials(&url);
        if output.timed_out {
            return Err(RegistryError::Git(format!(
                "cloning '{shown_url}' timed out after {} seconds",
                REGISTRY_NETWORK_TIMEOUT.as_secs()
            )));
        }
        if !output.status.success() {
            let stderr =
                crate::resolution::redact_url_credentials(&String::from_utf8_lossy(&output.stderr));
            return Err(RegistryError::Git(format!(
                "failed to clone repository from '{shown_url}': {stderr}"
            )));
        }

        Ok(())
    }

    /// Clones a repository, resolves the requested ref to a full commit, and checks it out detached.
    ///
    /// Git is invoked with explicit arguments, a null stdin, and a hard timeout.
    /// URL and ref values are validated before reaching this function.
    pub fn clone_and_pin_commit(
        url: &str,
        target_path: &Path,
        requested: &GitRef,
        timeout: Duration,
    ) -> Result<String, RegistryError> {
        // The remote name is pinned explicitly so a user-level
        // `clone.defaultRemoteName` setting cannot change where remote-tracking
        // branches are created.
        Self::run_git(
            timeout,
            &[
                "clone",
                "--no-checkout",
                "--origin",
                CLONE_REMOTE_NAME,
                "--",
                url,
                target_path.to_str().unwrap_or(""),
            ],
        )?;

        let expression = ref_resolution_expression(requested);

        let commit = Self::run_git(
            timeout,
            &[
                "-C",
                target_path.to_str().unwrap_or(""),
                "rev-parse",
                "--verify",
                &expression,
            ],
        )?
        .trim()
        .to_string();

        if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(RegistryError::Git(format!(
                "Git ref '{requested:?}' resolved to invalid commit '{commit}'"
            )));
        }

        Self::run_git(
            timeout,
            &[
                "-C",
                target_path.to_str().unwrap_or(""),
                "checkout",
                "--detach",
                &commit,
            ],
        )?;

        Ok(commit.to_ascii_lowercase())
    }

    /// Verifies that a full commit exists in an already cloned repository.
    pub fn verify_commit(
        path: &Path,
        commit: &str,
        timeout: Duration,
    ) -> Result<(), RegistryError> {
        let expression = format!("{commit}^{{commit}}");
        Self::run_git(
            timeout,
            &[
                "-C",
                path.to_str().unwrap_or(""),
                "rev-parse",
                "--verify",
                &expression,
            ],
        )
        .map(|_| ())
    }

    /// Builds a non-interactive `git` command for source acquisition.
    ///
    /// `GIT_TERMINAL_PROMPT=0` prevents Git from opening `/dev/tty` to ask for
    /// credentials (a null stdin alone does not stop that), so an
    /// unauthenticated private URL fails fast instead of hanging until the
    /// timeout. Configured credential helpers keep working.
    pub(crate) fn noninteractive_command() -> Command {
        let mut command = Self::policy_command();
        command.env("GIT_TERMINAL_PROMPT", "0");
        command
    }

    /// Builds a `git` command restricted to the transports Crux supports.
    ///
    /// `GIT_ALLOW_PROTOCOL` is Git's own transport allow-list: it applies to
    /// the initial URL, to HTTP redirects (Git restricts libcurl's redirect
    /// protocols accordingly), and to any nested fetch. Plain `http://`,
    /// `git://`, and remote-helper transports such as `ext::` are refused by
    /// Git itself even if a URL slipped past validation.
    pub(crate) fn policy_command() -> Command {
        let mut command = Command::new("git");
        command.env("GIT_ALLOW_PROTOCOL", ALLOWED_GIT_PROTOCOLS);
        command
    }

    /// Runs Git with a hard timeout while continuously draining stdout and
    /// stderr, so large output can never fill a pipe and stall the child.
    pub(crate) fn run_git(timeout: Duration, args: &[&str]) -> Result<String, RegistryError> {
        let mut command = Self::noninteractive_command();
        command.args(args);
        let output = run_with_timeout(&mut command, timeout)
            .map_err(|e| RegistryError::Git(format!("failed to execute git: {e}")))?;

        if output.timed_out {
            return Err(RegistryError::Git(format!(
                "git command timed out after {} seconds",
                timeout.as_secs()
            )));
        }
        if !output.status.success() {
            let stderr =
                crate::resolution::redact_url_credentials(&String::from_utf8_lossy(&output.stderr));
            return Err(RegistryError::Git(format!(
                "git {} failed: {stderr}",
                args.first().copied().unwrap_or("command")
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Updates an existing repository by performing a fast-forward pull.
    ///
    /// Bounded by [`REGISTRY_NETWORK_TIMEOUT`] and restricted to the allowed
    /// transports. Used by `crux registry update`, so terminal credential
    /// prompts remain possible.
    pub fn update_repository(path: &Path) -> Result<String, RegistryError> {
        let mut command = Self::policy_command();
        command.arg("-C").arg(path).arg("pull").arg("--ff-only");
        let output = run_with_timeout(&mut command, REGISTRY_NETWORK_TIMEOUT)
            .map_err(|e| RegistryError::Git(format!("failed to execute git pull: {e}")))?;

        if output.timed_out {
            return Err(RegistryError::Git(format!(
                "updating registry repository at '{}' timed out after {} seconds",
                path.display(),
                REGISTRY_NETWORK_TIMEOUT.as_secs()
            )));
        }
        if !output.status.success() {
            let stderr =
                crate::resolution::redact_url_credentials(&String::from_utf8_lossy(&output.stderr));
            return Err(RegistryError::Git(format!(
                "failed to update registry repository at '{}': {stderr}",
                path.display()
            )));
        }

        Self::head_commit(path)
    }

    /// Returns the HEAD commit hash.
    pub fn head_commit(path: &Path) -> Result<String, RegistryError> {
        let mut command = Self::policy_command();
        command
            .arg("-C")
            .arg(path)
            .arg("rev-parse")
            .arg("HEAD")
            .env("GIT_TERMINAL_PROMPT", "0");
        let output = run_with_timeout(&mut command, LOCAL_GIT_TIMEOUT)
            .map_err(|e| RegistryError::Git(format!("failed to get HEAD commit: {e}")))?;

        if !output.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RegistryError::Git(format!(
                "failed to query HEAD commit at '{}': {stderr}",
                path.display()
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crux_gitops_{prefix}_{}_{}",
            std::process::id(),
            crate::core::filesystem::generate_nonce()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Crux Test",
                "-c",
                "user.email=crux@example.invalid",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn ref_expressions_use_fully_qualified_refs() {
        assert_eq!(
            ref_resolution_expression(&GitRef::Branch("dev".into())),
            "refs/remotes/origin/dev^{commit}"
        );
        assert_eq!(
            ref_resolution_expression(&GitRef::Tag("v1.0.0".into())),
            "refs/tags/v1.0.0^{commit}"
        );
        assert_eq!(ref_resolution_expression(&GitRef::Head), "HEAD^{commit}");
    }

    #[test]
    fn noninteractive_command_disables_terminal_prompts() {
        let command = GitOps::noninteractive_command();
        let prompt = command
            .get_envs()
            .find(|(key, _)| *key == "GIT_TERMINAL_PROMPT")
            .and_then(|(_, value)| value);
        assert_eq!(prompt, Some(std::ffi::OsStr::new("0")));
    }

    /// Regression: stdout was only read after the child exited, so output
    /// larger than the pipe buffer (~64 KiB) deadlocked until the timeout.
    #[test]
    fn run_git_drains_large_output_without_stalling() {
        let dir = temp_dir("large_output");
        git(&dir, &["init", "-q"]);
        let payload = "0123456789abcdef".repeat(64 * 1024); // 1 MiB
        fs::write(dir.join("big.txt"), &payload).unwrap();
        let blob = git(&dir, &["hash-object", "-w", "big.txt"]);

        let started = std::time::Instant::now();
        let output = GitOps::run_git(
            Duration::from_secs(20),
            &["-C", dir.to_str().unwrap(), "cat-file", "-p", &blob],
        )
        .expect("large git output must be drained");
        assert_eq!(output.len(), payload.len());
        assert!(started.elapsed() < Duration::from_secs(20));

        let _ = fs::remove_dir_all(dir);
    }

    /// Regression: non-default branches only exist as remote-tracking refs in a
    /// `--no-checkout` clone and previously failed to resolve.
    #[test]
    fn clone_and_pin_commit_resolves_non_default_branch() {
        let dir = temp_dir("branch");
        let upstream = dir.join("upstream");
        fs::create_dir_all(&upstream).unwrap();
        git(&upstream, &["init", "-q", "-b", "main"]);
        fs::write(upstream.join("a.txt"), "main").unwrap();
        git(&upstream, &["add", "."]);
        git(&upstream, &["commit", "-q", "-m", "main"]);
        let main_commit = git(&upstream, &["rev-parse", "HEAD"]);
        git(&upstream, &["checkout", "-q", "-b", "dev"]);
        fs::write(upstream.join("a.txt"), "dev").unwrap();
        git(&upstream, &["commit", "-q", "-am", "dev"]);
        let dev_commit = git(&upstream, &["rev-parse", "HEAD"]);
        git(&upstream, &["checkout", "-q", "main"]);
        git(&upstream, &["tag", "dev-tag", &main_commit]);

        let url = upstream.to_str().unwrap();
        let resolved = GitOps::clone_and_pin_commit(
            url,
            &dir.join("clone_dev"),
            &GitRef::Branch("dev".into()),
            Duration::from_secs(30),
        )
        .expect("non-default branch must resolve");
        assert_eq!(resolved, dev_commit);
        assert_eq!(
            fs::read_to_string(dir.join("clone_dev/a.txt")).unwrap(),
            "dev"
        );

        let resolved_main = GitOps::clone_and_pin_commit(
            url,
            &dir.join("clone_main"),
            &GitRef::Branch("main".into()),
            Duration::from_secs(30),
        )
        .unwrap();
        assert_eq!(resolved_main, main_commit);

        let resolved_tag = GitOps::clone_and_pin_commit(
            url,
            &dir.join("clone_tag"),
            &GitRef::Tag("dev-tag".into()),
            Duration::from_secs(30),
        )
        .unwrap();
        assert_eq!(resolved_tag, main_commit);

        // A branch name must not resolve through a tag (and vice versa).
        assert!(
            GitOps::clone_and_pin_commit(
                url,
                &dir.join("clone_wrong_kind"),
                &GitRef::Branch("dev-tag".into()),
                Duration::from_secs(30),
            )
            .is_err()
        );

        let _ = fs::remove_dir_all(dir);
    }

    /// Regression: `git clone <url> <path>` without `--` let a URL such as
    /// `--upload-pack=<cmd>` be parsed as a Git option (confirmed: Git then
    /// treated the target directory as the repository).
    #[test]
    fn registry_clone_rejects_option_injection_before_running_git() {
        let dir = temp_dir("option_injection");
        let marker = dir.join("PWNED");
        let url = format!("--upload-pack=touch {};false", marker.display());
        let err = GitOps::clone_registry(&url, &dir.join("target"), false).unwrap_err();
        assert!(matches!(err, RegistryError::Validation(_)), "{err}");
        assert!(!marker.exists());
        assert!(!dir.join("target").exists());
        let _ = fs::remove_dir_all(dir);
    }

    /// Defense in depth: even a URL that bypassed validation is refused by Git
    /// itself through `GIT_ALLOW_PROTOCOL` (plain HTTP, `git://`, `ext::`).
    #[test]
    fn git_transport_allow_list_is_enforced_by_git() {
        let dir = temp_dir("transport_policy");
        for (index, url) in [
            "http://127.0.0.1:9/repo.git",
            "git://127.0.0.1:9/repo.git",
            "ext::sh -c true",
        ]
        .iter()
        .enumerate()
        {
            let target = dir.join(format!("t{index}"));
            let err = GitOps::run_git(
                Duration::from_secs(20),
                &["clone", "--", url, target.to_str().unwrap()],
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("not allowed"), "{url}: {err}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn git_commands_carry_the_transport_allow_list() {
        for command in [GitOps::policy_command(), GitOps::noninteractive_command()] {
            let allow = command
                .get_envs()
                .find(|(key, _)| *key == "GIT_ALLOW_PROTOCOL")
                .and_then(|(_, value)| value);
            assert_eq!(allow, Some(std::ffi::OsStr::new(ALLOWED_GIT_PROTOCOLS)));
        }
    }

    #[test]
    fn registry_clone_errors_redact_credentials() {
        let dir = temp_dir("redaction");
        // Valid by registry policy (credentials allowed) but unreachable.
        let url = "https://user:supersecret@127.0.0.1:9/registry.git";
        let err = GitOps::clone_registry(url, &dir.join("target"), false)
            .unwrap_err()
            .to_string();
        assert!(!err.contains("supersecret"), "{err}");
        assert!(err.contains("***@127.0.0.1"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_clone_still_supports_local_repositories() {
        let dir = temp_dir("local_registry");
        let upstream = dir.join("upstream");
        fs::create_dir_all(&upstream).unwrap();
        git(&upstream, &["init", "-q", "-b", "main"]);
        fs::write(upstream.join("registry.toml"), "x").unwrap();
        git(&upstream, &["add", "."]);
        git(&upstream, &["commit", "-q", "-m", "init"]);
        GitOps::clone_registry(upstream.to_str().unwrap(), &dir.join("clone"), false).unwrap();
        assert!(dir.join("clone/registry.toml").is_file());
        assert!(GitOps::is_git_repository(&dir.join("clone")));
        assert_eq!(
            GitOps::head_commit(&dir.join("clone")).unwrap(),
            git(&upstream, &["rev-parse", "HEAD"])
        );
        let _ = fs::remove_dir_all(dir);
    }
}
