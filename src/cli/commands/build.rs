//! Implementation of the `crux build` command.

use crate::build::{BuildConfiguration, BuildEngine, BuildProfile, BuildRequest, CMakeBuildEngine};
use crate::cli::CliError;
use crate::cli::load_active_registry;
use crate::package::{PackageId, PackageName};
use crate::registry::{GitRegistry, load_config};
use crate::resolution::catalog::PackageCatalog;
use crate::resolution::lockfile::{LOCKFILE_FILENAME, LockError, LockedPackage, Lockfile};
use crate::source::{AcquisitionError, SourceAcquirer, SourceCache};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn run_build<W: Write, E: Write>(
    project_dir: &Path,
    package_spec: Option<&str>,
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

    let (registry, reg_name) = match load_active_registry(project_dir) {
        Ok(pair) => pair,
        Err(err) => {
            writeln!(stderr, "{err}")?;
            return Err(err);
        }
    };

    if let Some(spec) = package_spec {
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

        let pkg_name = if registry.package_names().contains(&raw_pkg_name) {
            raw_pkg_name
        } else if let Some(matching) = registry
            .package_names()
            .iter()
            .find(|p| p.as_str().eq_ignore_ascii_case(raw_pkg_name.as_str()))
        {
            matching.clone()
        } else {
            raw_pkg_name
        };

        let id = if let Some(ver_str) = ver_req {
            let version = semver::Version::parse(ver_str).map_err(|e| {
                let err = CliError::General(format!("invalid version '{ver_str}': {e}"));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            PackageId::new(pkg_name, version)
        } else {
            let entries = registry
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

        let def = registry.get_package_definition(&id).ok_or_else(|| {
            let err = CliError::General(format!(
                "package definition '{id}' not found in registry '{reg_name}'"
            ));
            writeln!(stderr, "{err}").ok();
            err
        })?;

        writeln!(stdout, "Acquiring source for {id}...")?;
        let acquired = acquirer
            .acquire_package_definition(&id, def, force)
            .map_err(|e| {
                writeln!(stderr, "error: failed to acquire package '{id}': {e}").ok();
                CliError::Acquisition(Box::new(e))
            })?;

        writeln!(stdout, "Building {id} [{profile}] with CMake...")?;
        let build_config = BuildConfiguration::with_profile(profile);
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
    } else {
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

        let (registry, reg_name) = match load_active_registry(project_dir) {
            Ok(pair) => pair,
            Err(err) => {
                writeln!(stderr, "{err}")?;
                return Err(err);
            }
        };

        writeln!(
            stdout,
            "Building {} project dependencies [{profile}]...",
            lockfile.packages.len()
        )?;

        let ordered_packages = sort_packages_topologically(&lockfile.packages, &registry);
        let mut built_count = 0;
        let mut cached_count = 0;

        for locked in &ordered_packages {
            let id = PackageId::new(locked.name.clone(), locked.version.clone());
            let def = registry.get_package_definition(&id).ok_or_else(|| {
                let err = CliError::General(format!(
                    "package definition '{id}' not found in registry '{reg_name}'"
                ));
                writeln!(stderr, "{err}").ok();
                err
            })?;

            let acquired = acquirer
                .acquire_package_definition(&id, def, force)
                .map_err(|e| {
                    writeln!(stderr, "error: failed to acquire package '{id}': {e}").ok();
                    CliError::Acquisition(Box::new(e))
                })?;

            let build_config = BuildConfiguration::with_profile(profile);
            let request = BuildRequest::new(id.clone(), acquired.source_dir, acquired.checksum)
                .with_configuration(build_config)
                .with_force(force);

            let result = build_engine.build(&request).map_err(|e| {
                writeln!(stderr, "{e}").ok();
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
}

/// Topologically sorts locked packages so dependencies are built before their dependents.
pub fn sort_packages_topologically(
    packages: &[LockedPackage],
    registry: &GitRegistry,
) -> Vec<LockedPackage> {
    let mut in_degree: BTreeMap<PackageName, usize> = BTreeMap::new();
    let mut dependents_of: BTreeMap<PackageName, Vec<PackageName>> = BTreeMap::new();
    let locked_map: BTreeMap<PackageName, &LockedPackage> =
        packages.iter().map(|lp| (lp.name.clone(), lp)).collect();

    for pkg in packages {
        in_degree.entry(pkg.name.clone()).or_insert(0);
        let id = PackageId::new(pkg.name.clone(), pkg.version.clone());
        if let Some(def) = registry.get_package_definition(&id) {
            for dep in &def.dependencies {
                if locked_map.contains_key(dep.name()) {
                    dependents_of
                        .entry(dep.name().clone())
                        .or_default()
                        .push(pkg.name.clone());
                    *in_degree.entry(pkg.name.clone()).or_insert(0) += 1;
                }
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
