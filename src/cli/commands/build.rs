//! Implementation of the `crux build` command.

use crate::build::{
    AppBuildEngine, AppBuildRequest, BuildEngine, BuildError, BuildProfile, BuildRequest,
    CMakeBuildEngine,
};
use crate::cli::CliError;
use crate::cli::args::LockArgs;
use crate::cli::commands::install::run_install;
use crate::cli::commands::lock::run_lock;
use crate::cli::commands::{build_configuration_for, require_supported_build_system};
use crate::cli::load_active_catalog;
use crate::cli::load_catalog_for_dependency_edits;
use crate::integrate::validation::validate_package_installation;
use crate::package::{PackageId, PackageName};
use crate::project::config::ProjectConfig;
use crate::project::manifest::MANIFEST_FILENAME;
use crate::registry::load_config;
use crate::resolution::catalog::PackageCatalog;
use crate::resolution::lockfile::{
    LOCKFILE_FILENAME, LockError, LockedPackage, Lockfile, check_lockfile_sync,
};
use crate::source::{AcquisitionError, SourceAcquirer, SourceCache};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Top-level dispatcher for `crux build`.
///
/// Dispatch is deterministic and preserves v0.1.0 package builds:
/// 1. If a `package_spec` is provided (e.g. `crux build fmt`), builds that package.
/// 2. Otherwise, if `crux.toml` contains a `[project]` section, builds the local application.
/// 3. Otherwise, if `crux.toml` contains a legacy `[package]` section, builds locked dependencies.
/// 4. Otherwise, if `crux.lock` exists, builds locked dependencies.
/// 5. If neither a manifest nor a lockfile exists, returns an error.
pub fn run_build<W: Write, E: Write>(
    project_dir: &Path,
    package_spec: Option<&str>,
    profile: Option<BuildProfile>,
    force: bool,
    verbose: bool,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    if let Some(spec) = package_spec {
        let prof = profile.unwrap_or(BuildProfile::Release);
        return run_package_build(project_dir, spec, prof, force, stdout, stderr);
    }

    let manifest_path = project_dir.join(MANIFEST_FILENAME);
    if manifest_path.is_file() {
        let config = match ProjectConfig::load_from_dir(project_dir) {
            Ok(config) => config,
            Err(err) => {
                writeln!(stderr, "error: {err}")?;
                return Err(CliError::General(format!("error: {err}")));
            }
        };

        if config.is_project {
            let prof = profile.unwrap_or(BuildProfile::Debug);
            return run_app_build(project_dir, prof, force, verbose, stdout, stderr);
        }

        // Legacy `[package]` manifests retain v0.1.0 behavior: build locked dependencies.
        let prof = profile.unwrap_or(BuildProfile::Release);
        return run_lockfile_dependencies_build(project_dir, prof, force, stdout, stderr);
    }

    if project_dir.join(LOCKFILE_FILENAME).is_file() {
        let prof = profile.unwrap_or(BuildProfile::Release);
        return run_lockfile_dependencies_build(project_dir, prof, force, stdout, stderr);
    }

    let err = format!(
        "manifest not found at '{}': run 'crux init' to initialize a new project",
        manifest_path.display()
    );
    writeln!(stderr, "error: {err}")?;
    Err(CliError::General(format!("error: {err}")))
}

/// Builds the local Crux application project using the internal CMake engine.
fn run_app_build<W: Write, E: Write>(
    project_dir: &Path,
    profile: BuildProfile,
    force: bool,
    verbose: bool,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    writeln!(stdout, "Crux Application Build\n")?;

    // Resolve, lock, and install declared dependencies before configuring CMake.
    ensure_app_dependencies(project_dir, profile, stdout, stderr)?;

    let engine = AppBuildEngine::new();
    let request = AppBuildRequest::new(project_dir)
        .with_profile(profile)
        .with_force(force)
        .with_verbose(verbose);

    let model = match engine.prepare(&request) {
        Ok(model) => model,
        Err(err) => {
            writeln!(stderr, "error: {err}")?;
            return Err(CliError::Build(Box::new(err)));
        }
    };

    writeln!(stdout, "Project:       {}", model.project_name())?;
    writeln!(stdout, "Language:      {}", model.language.display_name())?;
    writeln!(
        stdout,
        "Standard:      {}",
        model.standard.to_ascii_uppercase()
    )?;
    writeln!(
        stdout,
        "Configuration: {}",
        model.profile.as_cmake_build_type()
    )?;
    writeln!(stdout, "\nToolchain:")?;
    writeln!(
        stdout,
        "  Compiler:    {} {}",
        model.toolchain.compiler.family, model.toolchain.compiler.version
    )?;
    writeln!(stdout, "  CMake:       {}", model.cmake.version)?;
    writeln!(stdout, "  Generator:   {}", model.cmake.generator_display())?;
    if verbose {
        writeln!(
            stdout,
            "  Compiler Exe: {}",
            model.toolchain.compiler.path.display()
        )?;
        writeln!(stdout, "  CMake Exe:    {}", model.cmake.path.display())?;
    }
    writeln!(stdout, "\nSource files:  {}", model.source_count())?;
    if verbose {
        for source in &model.sources.files {
            let relative = source.strip_prefix(&model.project_root).unwrap_or(source);
            writeln!(stdout, "  - {}", relative.display())?;
        }
    }
    writeln!(stdout, "Dependencies:  {}", model.dependency_count())?;
    if verbose {
        for dep in &model.dependencies.dependencies {
            writeln!(
                stdout,
                "  - {} {} ({}:{})",
                dep.name, dep.version, dep.fingerprint, profile
            )?;
        }
    }
    writeln!(stdout, "\nGenerating build configuration...")?;
    writeln!(stdout, "Configuring...")?;
    writeln!(stdout, "Building...")?;

    match engine.execute(&model, force, verbose) {
        Ok(result) => {
            if verbose {
                echo_log(stdout, "CMake configure output", &result.configure_log_path)?;
                echo_log(stdout, "CMake build output", &result.build_log_path)?;
            }
            writeln!(stdout, "\nBuild successful!\n")?;
            writeln!(stdout, "Executable:")?;
            let relative = result
                .executable_path
                .strip_prefix(&model.project_root)
                .unwrap_or(&result.executable_path);
            writeln!(stdout, "  {}", relative.display())?;
            writeln!(
                stdout,
                "\nFinished in {:.2}s",
                result.duration.as_secs_f64()
            )?;
            Ok(())
        }
        Err(err) => {
            if verbose {
                if let Some(log_path) = error_log_path(&err) {
                    echo_log(stderr, "CMake output", log_path)?;
                }
            }
            writeln!(stderr, "\nBuild failed!\n{err}")?;
            Err(CliError::Build(Box::new(err)))
        }
    }
}

/// Ensures all declared application dependencies are locked and installed for
/// the selected profile before CMake configuration runs.
///
/// This is the single shared dependency-synchronization step used by both
/// `crux build` and `crux run`. It is a no-op for projects without
/// dependencies, so existing application builds are unaffected.
pub(crate) fn ensure_app_dependencies<W: Write, E: Write>(
    project_dir: &Path,
    profile: BuildProfile,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    let config = ProjectConfig::load_from_dir(project_dir)
        .map_err(|err| CliError::General(err.to_string()))?;

    if !config.is_project || config.dependencies.is_empty() {
        return Ok(());
    }

    let lockfile_path = project_dir.join(LOCKFILE_FILENAME);
    let needs_lock = if !lockfile_path.is_file() {
        true
    } else {
        let (catalog, _) =
            load_catalog_for_dependency_edits(project_dir, &config.dependencies, stderr)?;
        check_lockfile_sync(project_dir, &catalog).is_err()
    };

    if needs_lock {
        writeln!(
            stdout,
            "Lockfile is missing or out of date; resolving dependencies..."
        )?;
        run_lock(project_dir, &LockArgs { update: false }, stdout, stderr)?;
        writeln!(stdout)?;
    }

    let lockfile = Lockfile::load_from_path(&lockfile_path).map_err(|err| {
        let msg = format!(
            "failed to read lockfile '{}': {err}",
            lockfile_path.display()
        );
        writeln!(stderr, "error: {msg}").ok();
        CliError::Lock(LockError::Lockfile(err))
    })?;

    if lockfile.packages.is_empty() {
        return Ok(());
    }

    let mut missing = Vec::new();
    for pkg in &lockfile.packages {
        if let Err(err) = validate_package_installation(
            project_dir,
            pkg.name.as_str(),
            &pkg.version.to_string(),
            profile,
        ) {
            missing.push((
                pkg.name.as_str().to_string(),
                pkg.version.to_string(),
                err.to_string(),
            ));
        }
    }

    if missing.is_empty() {
        writeln!(
            stdout,
            "Dependencies:  {} installed [{}]",
            lockfile.packages.len(),
            profile
        )?;
        return Ok(());
    }

    writeln!(
        stdout,
        "\nInstalling {} required dependency package(s) [{}]...",
        lockfile.packages.len(),
        profile
    )?;
    for (name, version, _) in &missing {
        writeln!(stdout, "  missing: {name} {version}")?;
    }

    run_install(
        project_dir,
        None,
        profile,
        false,
        true,
        &mut *stdout,
        &mut *stderr,
    )
    .inspect_err(|_err| {
        writeln!(
            stderr,
            "error: failed to install required dependencies for the {profile} profile.\n\
             hint: run 'crux install --profile {profile}' to inspect acquisition/build output, \
             or 'crux lock --update' if the lockfile is stale."
        )
        .ok();
    })?;

    for (name, version, _) in &missing {
        validate_package_installation(project_dir, name, version, profile).map_err(|err| {
            let msg = format!(
                "dependency '{name}' ({version}) is still unavailable after installation: {err}\n\
                 hint: run 'crux install --profile {profile} --force' to rebuild it"
            );
            writeln!(stderr, "error: {msg}").ok();
            CliError::Integration(Box::new(err))
        })?;
    }

    Ok(())
}

fn error_log_path(error: &BuildError) -> Option<&Path> {
    match error {
        BuildError::ConfigureFailed { log_path, .. } | BuildError::BuildFailed { log_path, .. } => {
            Some(log_path)
        }
        _ => None,
    }
}

fn echo_log<W: Write>(writer: &mut W, label: &str, path: &Path) -> Result<(), CliError> {
    match fs::read(path) {
        Ok(content) => {
            writeln!(writer, "\n--- {label} ({}) ---", path.display())?;
            writer.write_all(&content)?;
            if !content.ends_with(b"\n") {
                writeln!(writer)?;
            }
            writeln!(writer, "--- end {label} ---")?;
            Ok(())
        }
        Err(error) => {
            writeln!(
                writer,
                "warning: failed to read {label} log '{}': {error}",
                path.display()
            )?;
            Ok(())
        }
    }
}

/// Builds a specific package from the configured package registry.
fn run_package_build<W: Write, E: Write>(
    project_dir: &Path,
    spec: &str,
    profile: BuildProfile,
    force: bool,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    let config = match load_config(Some(project_dir)) {
        Ok(cfg) => cfg,
        Err(err) => {
            let cli_err = CliError::Registry(Box::new(err));
            writeln!(stderr, "{cli_err}").ok();
            return Err(cli_err);
        }
    };
    let cache = SourceCache::from_config(&config).map_err(AcquisitionError::Io)?;
    let acquirer = SourceAcquirer::new(cache);

    let builds_root = project_dir.join(".crux").join("builds");
    let build_engine = CMakeBuildEngine::new(&builds_root);

    let (catalog, reg_note) = match load_active_catalog(project_dir, stderr) {
        Ok(pair) => pair,
        Err(err) => {
            writeln!(stderr, "{err}")?;
            return Err(err);
        }
    };
    let reg_name = catalog.registry_name().to_string();
    if let Some(note) = reg_note {
        writeln!(stdout, "{note}")?;
    }

    let (name_str, ver_req) = if let Some((n, v)) = spec.split_once('@') {
        (n, Some(v))
    } else {
        (spec, None)
    };

    let raw_pkg_name = PackageName::new(name_str).map_err(|e| {
        let err = CliError::General(format!("invalid package name '{name_str}': {e}"));
        writeln!(stderr, "{err}").ok();
        err
    })?;

    let pkg_name = catalog.resolve_name(&raw_pkg_name);

    let id = if let Some(ver_str) = ver_req {
        let version = semver::Version::parse(ver_str).map_err(|e| {
            let err = CliError::General(format!("invalid version '{ver_str}': {e}"));
            writeln!(stderr, "{err}").ok();
            err
        })?;
        PackageId::new(pkg_name, version)
    } else {
        let entries = catalog
            .get_package_versions(&pkg_name)
            .map_err(|e| CliError::General(format!("error querying package versions: {e}")))?;
        if entries.is_empty() {
            let err = CliError::General(format!(
                "package '{pkg_name}' not found in registry '{reg_name}'"
            ));
            writeln!(stderr, "{err}")?;
            return Err(err);
        }
        let highest = entries.last().unwrap().version.clone();
        PackageId::new(pkg_name, highest)
    };

    let def = catalog.definition(&id).ok_or_else(|| {
        let err = CliError::General(format!(
            "package definition '{id}' not found in active catalog ('{reg_name}')"
        ));
        writeln!(stderr, "{err}").ok();
        err
    })?;

    require_supported_build_system(def, &id, stderr)?;

    writeln!(stdout, "Acquiring source for {id}...")?;
    let acquired = catalog
        .acquire_definition(&acquirer, project_dir, &id, force)
        .map_err(|e| {
            writeln!(stderr, "error: failed to acquire package '{id}': {e}").ok();
            CliError::Acquisition(Box::new(e))
        })?;

    writeln!(stdout, "Building {id} [{profile}] with CMake...")?;
    let build_config = build_configuration_for(def, profile);
    let request = BuildRequest::new(id.clone(), acquired.source_dir, acquired.checksum)
        .with_configuration(build_config)
        .with_force(force);

    match build_engine.build(&request) {
        Ok(result) => {
            let status_str = if result.reused_cache {
                "reused cached build"
            } else {
                "compiled successfully"
            };
            writeln!(stdout, "\nSuccessfully built {id} ({status_str}):")?;
            writeln!(stdout, "  Profile:     {}", result.profile)?;
            writeln!(stdout, "  Fingerprint: {}", result.fingerprint)?;
            writeln!(stdout, "  Build Dir:   {}", result.build_dir.display())?;
            writeln!(stdout, "  Logs:        {}", result.logs_dir.display())?;
            writeln!(stdout, "  Artifacts ({}):", result.artifacts.len())?;
            for artifact in &result.artifacts {
                writeln!(
                    stdout,
                    "    - {} ({}, {} bytes)",
                    artifact.name, artifact.artifact_type, artifact.size_bytes
                )?;
            }
            Ok(())
        }
        Err(err) => {
            writeln!(stderr, "{err}")?;
            Err(CliError::Build(Box::new(err)))
        }
    }
}

/// Builds all locked project dependencies from `crux.lock`.
fn run_lockfile_dependencies_build<W: Write, E: Write>(
    project_dir: &Path,
    profile: BuildProfile,
    force: bool,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    let lockfile_path = project_dir.join(LOCKFILE_FILENAME);
    if !lockfile_path.is_file() {
        let err = CliError::General(format!(
            "lockfile not found at '{}': run 'crux lock' first to generate it",
            lockfile_path.display()
        ));
        writeln!(stderr, "{err}")?;
        return Err(err);
    }

    let lock_content = fs::read_to_string(&lockfile_path).map_err(|e| {
        let err = CliError::General(format!(
            "failed to read lockfile '{}': {e}",
            lockfile_path.display()
        ));
        writeln!(stderr, "{err}").ok();
        err
    })?;
    let lockfile = Lockfile::from_toml_str(&lock_content).map_err(|e| {
        let err = CliError::Lock(LockError::Lockfile(e));
        writeln!(stderr, "{err}").ok();
        err
    })?;

    if lockfile.packages.is_empty() {
        writeln!(stdout, "No dependencies to build in lockfile.")?;
        return Ok(());
    }

    let config = match load_config(Some(project_dir)) {
        Ok(cfg) => cfg,
        Err(err) => {
            let cli_err = CliError::Registry(Box::new(err));
            writeln!(stderr, "{cli_err}").ok();
            return Err(cli_err);
        }
    };
    let cache = SourceCache::from_config(&config).map_err(AcquisitionError::Io)?;
    let acquirer = SourceAcquirer::new(cache);

    let builds_root = project_dir.join(".crux").join("builds");
    let build_engine = CMakeBuildEngine::new(&builds_root);

    let (catalog, reg_note) = match load_active_catalog(project_dir, stderr) {
        Ok(pair) => pair,
        Err(err) => {
            writeln!(stderr, "{err}")?;
            return Err(err);
        }
    };
    let reg_name = catalog.registry_name().to_string();
    if let Some(note) = reg_note {
        writeln!(stdout, "{note}")?;
    }

    writeln!(
        stdout,
        "Building {} project dependencies [{profile}]...",
        lockfile.packages.len()
    )?;

    let ordered_packages = sort_packages_topologically(&lockfile.packages);
    let mut built_count = 0;
    let mut cached_count = 0;

    for locked in &ordered_packages {
        let id = PackageId::new(locked.name.clone(), locked.version.clone());
        let def = catalog.definition(&id).ok_or_else(|| {
            let err = CliError::General(format!(
                "package definition '{id}' not found in active catalog ('{reg_name}')"
            ));
            writeln!(stderr, "{err}").ok();
            err
        })?;

        require_supported_build_system(def, &id, stderr)?;

        let acquired = catalog
            .acquire_locked(&acquirer, project_dir, locked, force)
            .map_err(|e| {
                writeln!(stderr, "error: failed to acquire package '{id}': {e}").ok();
                CliError::Acquisition(Box::new(e))
            })?;

        let build_config = build_configuration_for(def, profile);
        let request = BuildRequest::new(id.clone(), acquired.source_dir, acquired.checksum)
            .with_configuration(build_config)
            .with_force(force);

        let result = build_engine.build(&request).map_err(|e| {
            writeln!(stderr, "error: failed to build package '{id}': {e}").ok();
            CliError::Build(Box::new(e))
        })?;

        if result.reused_cache {
            cached_count += 1;
            writeln!(
                stdout,
                "  [cached] {} (fingerprint: {})",
                id, result.fingerprint
            )?;
        } else {
            built_count += 1;
            writeln!(
                stdout,
                "  [built]  {} ({} artifacts in {:.2}s)",
                id,
                result.artifacts.len(),
                result.duration.as_secs_f64()
            )?;
        }
    }

    writeln!(
        stdout,
        "\nBuild completed: {built_count} compiled, {cached_count} reused from cache."
    )?;
    Ok(())
}

/// Topologically sorts locked packages so dependencies are built before their dependents.
pub fn sort_packages_topologically(packages: &[LockedPackage]) -> Vec<LockedPackage> {
    let mut in_degree: BTreeMap<PackageName, usize> = BTreeMap::new();
    let mut dependents_of: BTreeMap<PackageName, Vec<PackageName>> = BTreeMap::new();
    let locked_map: BTreeMap<PackageName, &LockedPackage> =
        packages.iter().map(|lp| (lp.name.clone(), lp)).collect();

    for pkg in packages {
        in_degree.entry(pkg.name.clone()).or_insert(0);
        for dep_id in &pkg.dependencies {
            if locked_map.contains_key(dep_id.name()) {
                dependents_of
                    .entry(dep_id.name().clone())
                    .or_default()
                    .push(pkg.name.clone());
                *in_degree.entry(pkg.name.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut queue: std::collections::VecDeque<PackageName> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(name, _)| name.clone())
        .collect();

    let mut sorted_names = Vec::new();
    while let Some(name) = queue.pop_front() {
        sorted_names.push(name.clone());
        if let Some(dependents) = dependents_of.get(&name) {
            for dep in dependents {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }
    }

    let mut result = Vec::new();
    for name in sorted_names {
        if let Some(&lp) = locked_map.get(&name) {
            result.push(lp.clone());
        }
    }
    for lp in packages {
        if !result.iter().any(|r| r.name == lp.name) {
            result.push(lp.clone());
        }
    }
    result
}
