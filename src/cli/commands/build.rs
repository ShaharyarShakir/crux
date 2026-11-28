//! Implementation of the `crux build` command.

use crate::build::package_dependencies::BuildDependencies;
use crate::build::{AppBuildEngine, AppBuildRequest, BuildProfile, BuildRequest, CMakeBuildEngine};
use crate::cli::CliError;
use crate::cli::args::LockArgs;
use crate::cli::commands::install::{PackageInstaller, run_install_selected, write_prerequisite};
use crate::cli::commands::lock::run_lock;
use crate::cli::commands::{
    DefinitionClosure, build_configuration_for, dependency_failure, require_supported_build_system,
    warn_if_not_recorded, warn_legacy_locked_checksum,
};
use crate::cli::load_active_catalog;
use crate::cli::load_catalog_for_dependency_edits;
use crate::install::current::{CurrentInstallation, read_current_record};
use crate::install::{CMakeInstallEngine, InstallRequest};
use crate::integrate::IntegrationError;
use crate::integrate::validation::{ValidatedPackageInstallation, validate_package_installation};
use crate::package::{PackageId, PackageName};
use crate::project::config::ProjectConfig;
use crate::project::manifest::MANIFEST_FILENAME;
use crate::registry::load_config;
use crate::resolution::DirectSource;
use crate::resolution::catalog::PackageCatalog;
use crate::resolution::lockfile::{
    LOCKFILE_FILENAME, LockError, LockedPackage, Lockfile, check_lockfile_sync,
};
use crate::source::{AcquisitionError, SourceAcquirer, SourceCache, compute_tree_sha256};
use std::collections::{BTreeMap, BTreeSet};
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
            let label = crate::cli::report::project_label(project_dir);
            crate::cli::report::write_build_failure(stderr, &err, &label)?;
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
    write_build_settings(stdout, &model.config.build, model.language, verbose)?;
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

    let app_label = format!("{} v{}", model.project_name(), model.config.version);
    crate::ui::status(stderr, "Compiling", &app_label)?;
    let tee = crate::cli::report::live_output_observer().map(|observer| observer.0);
    let started = std::time::Instant::now();
    match engine.execute_with_tee(&model, force, verbose, tee) {
        Ok(result) => {
            if verbose && crate::ui::verbosity() < crate::ui::Verbosity::VeryVerbose {
                echo_log(stdout, "CMake configure output", &result.configure_log_path)?;
                echo_log(stdout, "CMake build output", &result.build_log_path)?;
            }
            crate::cli::report::finished(stderr, model.profile, started)?;
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
            crate::cli::report::write_build_failure(stderr, &err, &app_label)?;
            Err(CliError::Build(Box::new(err)))
        }
    }
}

/// Prints the `[build]` customisations that apply to this build. Nothing is
/// printed when none are configured; flag lists appear with `-v`.
fn write_build_settings<W: Write>(
    stdout: &mut W,
    settings: &crate::project::BuildSettings,
    language: crate::project::ProjectLanguage,
    verbose: bool,
) -> Result<(), CliError> {
    if settings.is_default() {
        return Ok(());
    }
    let sanitizers = settings
        .sanitizers()
        .iter()
        .map(|s| s.name())
        .collect::<Vec<_>>();
    writeln!(stdout, "\nBuild settings (application only):")?;
    if !sanitizers.is_empty() {
        writeln!(stdout, "  Sanitizers:   {}", sanitizers.join(", "))?;
    }
    let compile = match language {
        crate::project::ProjectLanguage::C => &settings.c_flags,
        _ => &settings.cxx_flags,
    };
    let join = |flags: &[String]| flags.join(" ");
    if !compile.is_empty() {
        let label = if matches!(language, crate::project::ProjectLanguage::C) {
            "C flags:      "
        } else {
            "C++ flags:    "
        };
        if verbose {
            writeln!(stdout, "  {label}{}", join(compile))?;
        } else {
            writeln!(stdout, "  {label}{} custom", compile.len())?;
        }
    }
    if !settings.linker_flags.is_empty() {
        if verbose {
            writeln!(stdout, "  Linker flags: {}", join(&settings.linker_flags))?;
        } else {
            writeln!(
                stdout,
                "  Linker flags: {} custom",
                settings.linker_flags.len()
            )?;
        }
    }
    Ok(())
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

    // Packages without a usable installation, mutable path packages whose
    // source no longer matches what their current installation was built from,
    // and packages built against other dependency installations. Visiting
    // dependencies first carries a rebuild on to everything above them.
    let mut missing = Vec::new();
    let mut changed = Vec::new();
    let mut affected = BTreeSet::new();
    let ordered = sort_packages_topologically(&lockfile.packages);
    for pkg in &ordered {
        match validate_package_installation(
            project_dir,
            pkg.name.as_str(),
            &pkg.version.to_string(),
            profile,
        ) {
            Err(err) => missing.push((pkg, err)),
            Ok(installation) => {
                let mut state = path_source_state(project_dir, pkg, &installation)?;
                if matches!(state, PackageState::UpToDate)
                    && (pkg.dependencies.iter().any(|d| affected.contains(d.name()))
                        || !built_against_current(
                            project_dir,
                            &lockfile.packages,
                            pkg,
                            &installation,
                        ))
                {
                    state = PackageState::DependenciesChanged;
                }
                if matches!(state, PackageState::UpToDate) {
                    warn_if_not_recorded(stderr, &installation)?;
                    continue;
                }
                changed.push((pkg, state));
            }
        }
        affected.insert(&pkg.name);
    }

    if missing.is_empty() && changed.is_empty() {
        writeln!(
            stdout,
            "Dependencies:  {} installed [{}]",
            lockfile.packages.len(),
            profile
        )?;
        return Ok(());
    }

    // A missing installation reinstalls the whole lockfile (unchanged packages
    // are cache hits). Otherwise only edited packages and their dependents are
    // processed, so unrelated registry and Git dependencies are not touched.
    let only: Option<BTreeSet<PackageName>> = missing
        .is_empty()
        .then(|| changed.iter().map(|(pkg, _)| pkg.name.clone()).collect());
    writeln!(
        stdout,
        "\nInstalling {} required dependency package(s) [{}]...",
        only.as_ref().map_or(lockfile.packages.len(), BTreeSet::len),
        profile
    )?;
    for (pkg, err) in &missing {
        if matches!(err, IntegrationError::PackageNotInstalled { .. }) {
            writeln!(stdout, "  missing: {} {}", pkg.name, pkg.version)?;
        } else {
            writeln!(stdout, "  invalid: {} {}", pkg.name, pkg.version)?;
            let reason = match err {
                IntegrationError::CorruptInstallation { reason, .. } => reason.clone(),
                other => other.to_string(),
            };
            writeln!(
                stderr,
                "warning: cannot use the installation of '{}': {reason}; reinstalling",
                pkg.name
            )?;
        }
    }
    for (pkg, state) in &changed {
        let (label, detail) = match state {
            PackageState::Unrecorded => ("unrecorded", pkg.source.to_string()),
            PackageState::DependenciesChanged => {
                ("outdated", "its dependencies changed".to_string())
            }
            _ => ("changed", pkg.source.to_string()),
        };
        writeln!(stdout, "  {label}: {} {} ({detail})", pkg.name, pkg.version)?;
    }

    run_install_selected(
        project_dir,
        None,
        profile,
        false,
        true,
        only.as_ref(),
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

    for pkg in missing
        .iter()
        .map(|(pkg, _)| pkg)
        .chain(changed.iter().map(|(pkg, _)| pkg))
    {
        let (name, version) = (pkg.name.as_str(), pkg.version.to_string());
        validate_package_installation(project_dir, name, &version, profile).map_err(|err| {
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

/// Whether an installed dependency has to be processed again.
enum PackageState {
    /// Nothing it was built from is known to have changed.
    UpToDate,
    /// The live source differs from what the installation was built from.
    Changed,
    /// The installation predates source recording, so it cannot be compared.
    Unrecorded,
    /// Dependency installations differ from those it was built against, or
    /// those were never recorded.
    DependenciesChanged,
}

fn recorded_installation(
    installation: &ValidatedPackageInstallation,
) -> Option<CurrentInstallation> {
    let version_dir = installation.install_dir.parent()?;
    read_current_record(version_dir, installation.profile)
        .ok()
        .flatten()
}

/// Whether `installation` was built against the dependency installations that
/// are current now.
fn built_against_current(
    project_dir: &Path,
    packages: &[LockedPackage],
    locked: &LockedPackage,
    installation: &ValidatedPackageInstallation,
) -> bool {
    BuildDependencies::for_locked(project_dir, packages, locked, installation.profile).is_ok_and(
        |current| {
            recorded_installation(installation)
                .map(|record| record.dependencies)
                .unwrap_or_default()
                == current.record()
        },
    )
}

/// Compares a local path dependency's live source with the source recorded for
/// `installation`.
///
/// Registry, Git, and archive packages are pinned and always up to date here.
fn path_source_state(
    project_dir: &Path,
    locked: &LockedPackage,
    installation: &ValidatedPackageInstallation,
) -> Result<PackageState, CliError> {
    let Some(DirectSource::Path { path }) = locked.source.direct() else {
        return Ok(PackageState::UpToDate);
    };
    let recorded = recorded_installation(installation).and_then(|record| record.source_checksum);
    let Some(recorded) = recorded else {
        return Ok(PackageState::Unrecorded);
    };

    // The same digest source acquisition computes for path packages.
    let source_dir = project_dir.join(crate::resolution::normalize_direct_path(&path));
    let digest = fs::canonicalize(&source_dir)
        .map_err(AcquisitionError::Io)
        .and_then(|canonical| compute_tree_sha256(&canonical))
        .map_err(|err| {
            CliError::General(format!(
                "failed to check local path dependency '{}' at '{}' for changes: {err}",
                locked.name,
                source_dir.display()
            ))
        })?;
    Ok(if recorded == format!("sha256:{digest}") {
        PackageState::UpToDate
    } else {
        PackageState::Changed
    })
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
    let reporter = crate::cli::report::AcquisitionReporter::new();
    let acquirer = SourceAcquirer::new(cache).with_observer(reporter.clone());

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

    let build_system = require_supported_build_system(def, &id, stderr)?;

    // Prerequisites first, exactly as `crux install <package>` installs them.
    // The requested package itself is only built.
    let dependencies = if def.dependencies.is_empty() {
        BuildDependencies::default()
    } else {
        // A lockfile that cannot be read must not silently skip verification.
        let lockfile_path = project_dir.join(LOCKFILE_FILENAME);
        let lockfile = if lockfile_path.is_file() {
            Some(Lockfile::load_from_path(&lockfile_path).map_err(|err| {
                writeln!(
                    stderr,
                    "error: failed to read lockfile '{}': {err}",
                    lockfile_path.display()
                )
                .ok();
                CliError::Lock(LockError::Lockfile(err))
            })?)
        } else {
            None
        };
        let closure = DefinitionClosure::collect(&id, &catalog, lockfile.as_ref())?;
        let install_engine = CMakeInstallEngine::new(
            project_dir.join(".crux").join("installed"),
            project_dir.join(".crux").join("staging"),
        );
        let installer = PackageInstaller {
            project_dir,
            catalog: &catalog,
            acquirer: &acquirer,
            reporter: &reporter,
            build_engine: &build_engine,
            install_engine: &install_engine,
            profile,
        };

        writeln!(stdout, "Installing dependencies of {id} [{profile}]...")?;
        for dep in closure.order.iter().filter(|dep| **dep != id) {
            let locked = lockfile
                .as_ref()
                .and_then(|lf| lf.packages.iter().find(|p| p.id() == *dep));
            let result = installer.install(dep, &closure, locked, false, stderr)?;
            write_prerequisite(stdout, dep, &result)?;
        }
        BuildDependencies::resolve(project_dir, &closure.dependencies_of(&id)?, profile)
            .map_err(|err| dependency_failure(stderr, err, &id))?
    };

    writeln!(stdout, "Acquiring source for {id}...")?;
    let acquired = catalog.acquire_definition(&acquirer, project_dir, &id, force);
    reporter.flush_into(stderr)?;
    let acquired = acquired.map_err(|e| {
        writeln!(stderr, "error: failed to acquire package '{id}': {e}").ok();
        CliError::Acquisition(Box::new(e))
    })?;

    writeln!(stdout, "Building {id} [{profile}] with CMake...")?;
    let build_config = build_configuration_for(def, profile);
    let request = BuildRequest::new(id.clone(), acquired.source_dir, acquired.checksum)
        .with_configuration(build_config)
        .with_force(force);

    let started = std::time::Instant::now();
    match crate::cli::report::build_package_with_dependencies(
        &build_engine,
        build_system,
        request,
        &dependencies,
        stderr,
    ) {
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
            crate::cli::report::finished(stderr, result.profile, started)?;
            Ok(())
        }
        Err(err) => Err(err),
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
    let reporter = crate::cli::report::AcquisitionReporter::new();
    let acquirer = SourceAcquirer::new(cache).with_observer(reporter.clone());

    let builds_root = project_dir.join(".crux").join("builds");
    let build_engine = CMakeBuildEngine::new(&builds_root);
    let install_engine = CMakeInstallEngine::new(
        project_dir.join(".crux").join("installed"),
        project_dir.join(".crux").join("staging"),
    );

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
    // Packages others build against have to be installed, not only built.
    let depended_on: BTreeSet<&PackageName> = lockfile
        .packages
        .iter()
        .flat_map(|pkg| pkg.dependencies.iter().map(PackageId::name))
        .collect();
    let mut legacy_warned = std::collections::BTreeSet::new();
    let started = std::time::Instant::now();
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

        let build_system = require_supported_build_system(def, &id, stderr)?;

        let dependencies =
            BuildDependencies::for_locked(project_dir, &lockfile.packages, locked, profile)
                .map_err(|err| dependency_failure(stderr, err, &id))?;

        let acquired = catalog.acquire_locked_with_status(&acquirer, project_dir, locked, force);
        reporter.flush_into(stderr)?;
        let (acquired, status) = acquired.map_err(|e| {
            writeln!(stderr, "error: failed to acquire package '{id}': {e}").ok();
            CliError::Acquisition(Box::new(e))
        })?;
        warn_legacy_locked_checksum(stderr, &id, status, &mut legacy_warned)?;

        let build_config = build_configuration_for(def, profile);
        let source_checksum = acquired.checksum.clone();
        let request = BuildRequest::new(id.clone(), acquired.source_dir, acquired.checksum)
            .with_configuration(build_config)
            .with_force(force);

        let result = crate::cli::report::build_package_with_dependencies(
            &build_engine,
            build_system,
            request,
            &dependencies,
            stderr,
        )?;
        if depended_on.contains(&locked.name) {
            crate::ui::status(
                stderr,
                "Installing",
                &crate::cli::report::package_label(&id),
            )?;
            let install_req = InstallRequest::new(result.clone())
                .with_source_checksum(source_checksum)
                .with_force(force);
            install_engine
                .install_with_dependencies(build_system, &install_req, &dependencies)
                .map_err(|e| {
                    writeln!(stderr, "{e}").ok();
                    CliError::Install(Box::new(e))
                })?;
        }

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
    crate::cli::report::finished(stderr, profile, started)?;
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
