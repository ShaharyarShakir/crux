//! Implementation of the `crux install` command.

use crate::build::{BuildEngine, BuildProfile, BuildRequest, CMakeBuildEngine};
use crate::cli::CliError;
use crate::cli::commands::build::sort_packages_topologically;
use crate::cli::commands::{build_configuration_for, require_supported_build_system};
use crate::cli::load_active_catalog;
use crate::install::{CMakeInstallEngine, InstallEngine, InstallRequest};
use crate::package::{PackageId, PackageName};
use crate::project::manifest::MANIFEST_FILENAME;
use crate::registry::load_config;
use crate::resolution::catalog::PackageCatalog;
use crate::resolution::lockfile::{LOCKFILE_FILENAME, LockError, Lockfile, check_lockfile_sync};
use crate::source::{AcquisitionError, ActiveCatalog, SourceAcquirer, SourceCache};
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn run_install<W1: Write, W2: Write>(
    project_dir: &Path,
    package_spec: Option<&str>,
    profile: BuildProfile,
    force: bool,
    locked: bool,
    mut stdout: W1,
    mut stderr: W2,
) -> Result<(), CliError> {
    let lockfile_path = project_dir.join(LOCKFILE_FILENAME);

    // If --locked was specified, verify lockfile presence immediately
    if locked && !lockfile_path.is_file() {
        let err = CliError::General(format!(
            "--locked was specified, but lockfile not found at '{}': run 'crux lock' first",
            lockfile_path.display()
        ));
        writeln!(stderr, "{err}")?;
        return Err(err);
    }

    let (catalog, _catalog_note) = match load_active_catalog(project_dir, &mut stderr) {
        Ok(pair) => pair,
        Err(err) => {
            writeln!(stderr, "{err}")?;
            return Err(err);
        }
    };
    let reg_name = catalog.registry_name().to_string();

    // If --locked was specified, validate synchronization with crux.toml if manifest exists
    if locked {
        let manifest_path = project_dir.join(MANIFEST_FILENAME);
        if manifest_path.is_file()
            && let Err(err) = check_lockfile_sync(project_dir, &catalog)
        {
            let cli_err = CliError::Lock(err);
            writeln!(stderr, "{cli_err}")?;
            return Err(cli_err);
        }
    }

    let builds_root = project_dir.join(".crux").join("builds");
    let install_root = project_dir.join(".crux").join("installed");
    let staging_root = project_dir.join(".crux").join("staging");

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

    let build_engine = CMakeBuildEngine::new(&builds_root);
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

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

        let pkg_name = catalog.resolve_name(&raw_pkg_name);

        // If locked, check if package exists in lockfile
        let existing_lockfile = if lockfile_path.is_file() {
            fs::read_to_string(&lockfile_path)
                .ok()
                .and_then(|c| Lockfile::from_toml_str(&c).ok())
        } else {
            None
        };

        if locked {
            if let Some(ref lf) = existing_lockfile {
                if !lf.packages.iter().any(|p| p.name == pkg_name) {
                    let err = CliError::General(format!(
                        "--locked was specified, but package '{pkg_name}' is not in crux.lock"
                    ));
                    writeln!(stderr, "{err}")?;
                    return Err(err);
                }
            } else {
                let err = CliError::General(format!(
                    "--locked was specified, but lockfile not found at '{}'",
                    lockfile_path.display()
                ));
                writeln!(stderr, "{err}")?;
                return Err(err);
            }
        }

        let target_id = if let Some(ver_str) = ver_req {
            let version = semver::Version::parse(ver_str).map_err(|e| {
                let err = CliError::General(format!("invalid version '{ver_str}': {e}"));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            PackageId::new(pkg_name.clone(), version)
        } else if let Some(ref lf) = existing_lockfile
            && let Some(locked_pkg) = lf.packages.iter().find(|p| p.name == pkg_name)
        {
            PackageId::new(pkg_name.clone(), locked_pkg.version.clone())
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
            PackageId::new(pkg_name.clone(), highest)
        };

        // Collect dependency closure for target_id
        let mut packages_to_install = Vec::new();
        let mut visited = std::collections::BTreeSet::new();

        fn collect_deps(
            pkg_id: &PackageId,
            catalog: &ActiveCatalog,
            lockfile: Option<&Lockfile>,
            visited: &mut std::collections::BTreeSet<PackageId>,
            closure: &mut Vec<PackageId>,
        ) -> Result<(), CliError> {
            if !visited.insert(pkg_id.clone()) {
                return Ok(());
            }

            if let Some(def) = catalog.definition(pkg_id) {
                for dep in &def.dependencies {
                    let dep_id = if let Some(lf) = lockfile
                        && let Some(lp) = lf.packages.iter().find(|p| p.name == *dep.name())
                    {
                        PackageId::new(lp.name.clone(), lp.version.clone())
                    } else {
                        let versions = catalog.get_package_versions(dep.name()).map_err(|e| {
                            CliError::General(format!(
                                "failed querying dependency '{}': {e}",
                                dep.name()
                            ))
                        })?;
                        let matching = versions
                            .iter()
                            .rev()
                            .find(|v| dep.version_req().matches(&v.version))
                            .ok_or_else(|| {
                                CliError::General(format!(
                                    "no version of dependency '{}' matches requirement '{}'",
                                    dep.name(),
                                    dep.version_req()
                                ))
                            })?;
                        PackageId::new(dep.name().clone(), matching.version.clone())
                    };

                    collect_deps(&dep_id, catalog, lockfile, visited, closure)?;
                }
            }

            closure.push(pkg_id.clone());
            Ok(())
        }

        collect_deps(
            &target_id,
            &catalog,
            existing_lockfile.as_ref(),
            &mut visited,
            &mut packages_to_install,
        )?;

        // If multiple packages, print overview
        if packages_to_install.len() > 1 {
            writeln!(
                stdout,
                "Installing {target_id} and {} dependencies [{profile}]...",
                packages_to_install.len() - 1
            )?;
        } else {
            writeln!(stdout, "Installing {target_id} [{profile}]...")?;
        }

        for pkg in &packages_to_install {
            let is_target = pkg == &target_id;
            let pkg_force = if is_target { force } else { false };

            let def = catalog.definition(pkg).ok_or_else(|| {
                let err = CliError::General(format!(
                    "package definition '{pkg}' not found in active catalog ('{reg_name}')"
                ));
                writeln!(stderr, "{err}").ok();
                err
            })?;

            require_supported_build_system(def, pkg, &mut stderr)?;

            let acquired = catalog
                .acquire_definition(&acquirer, project_dir, pkg, false)
                .map_err(|e| {
                    writeln!(stderr, "error: failed to acquire package '{pkg}': {e}").ok();
                    CliError::Acquisition(Box::new(e))
                })?;

            let build_config = build_configuration_for(def, profile);
            let build_req = BuildRequest::new(pkg.clone(), acquired.source_dir, acquired.checksum)
                .with_configuration(build_config)
                .with_force(false);

            let build_result = build_engine.build(&build_req).map_err(|e| {
                writeln!(stderr, "{e}").ok();
                CliError::Build(Box::new(e))
            })?;

            let install_req = InstallRequest::new(build_result).with_force(pkg_force);
            let install_result = install_engine.install(&install_req).map_err(|e| {
                writeln!(stderr, "{e}").ok();
                CliError::Install(Box::new(e))
            })?;

            if is_target {
                let status_str = if install_result.reused_install {
                    "reused cached installation"
                } else {
                    "installed successfully"
                };
                writeln!(stdout, "\nSuccessfully installed {pkg} ({status_str}):")?;
                writeln!(stdout, "  Profile:      {}", profile)?;
                writeln!(stdout, "  Fingerprint:  {}", install_result.fingerprint)?;
                writeln!(
                    stdout,
                    "  Install Dir:  {}",
                    install_result.install_dir.display()
                )?;
                writeln!(
                    stdout,
                    "  Metadata:     {}",
                    install_result.metadata_path.display()
                )?;
                writeln!(
                    stdout,
                    "  Total Size:   {} bytes",
                    install_result.total_size_bytes
                )?;
                writeln!(
                    stdout,
                    "  Files ({}):",
                    install_result.installed_files.len()
                )?;
                for file in &install_result.installed_files {
                    writeln!(
                        stdout,
                        "    - {} ({}, {} bytes)",
                        file.relative_path.display(),
                        file.file_type,
                        file.size_bytes
                    )?;
                }
            } else if install_result.reused_install {
                writeln!(stdout, "  [cached]    {pkg}")?;
            } else {
                writeln!(
                    stdout,
                    "  [installed] {pkg} ({} files, {} bytes)",
                    install_result.installed_files.len(),
                    install_result.total_size_bytes
                )?;
            }
        }

        Ok(())
    } else {
        // Install dependencies from lockfile
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
            writeln!(stdout, "No dependencies to install in lockfile.")?;
            return Ok(());
        }

        writeln!(
            stdout,
            "Installing {} project dependencies [{profile}]...",
            lockfile.packages.len()
        )?;

        let ordered_packages = sort_packages_topologically(&lockfile.packages);
        let mut installed_count = 0;
        let mut cached_count = 0;

        for locked_pkg in &ordered_packages {
            let id = PackageId::new(locked_pkg.name.clone(), locked_pkg.version.clone());
            let def = catalog.definition(&id).ok_or_else(|| {
                let err = CliError::General(format!(
                    "package definition '{id}' not found in active catalog ('{reg_name}')"
                ));
                writeln!(stderr, "{err}").ok();
                err
            })?;

            require_supported_build_system(def, &id, &mut stderr)?;

            let acquired = catalog
                .acquire_locked(&acquirer, project_dir, locked_pkg, false)
                .map_err(|e| {
                    writeln!(stderr, "error: failed to acquire package '{id}': {e}").ok();
                    CliError::Acquisition(Box::new(e))
                })?;

            let build_config = build_configuration_for(def, profile);
            let build_req = BuildRequest::new(id.clone(), acquired.source_dir, acquired.checksum)
                .with_configuration(build_config)
                .with_force(false);

            let build_result = build_engine.build(&build_req).map_err(|e| {
                writeln!(stderr, "{e}").ok();
                CliError::Build(Box::new(e))
            })?;

            let install_req = InstallRequest::new(build_result).with_force(force);
            let install_result = install_engine.install(&install_req).map_err(|e| {
                writeln!(stderr, "{e}").ok();
                CliError::Install(Box::new(e))
            })?;

            if install_result.reused_install {
                cached_count += 1;
                writeln!(
                    stdout,
                    "  [cached]    {} (inventory: {} files)",
                    id,
                    install_result.installed_files.len()
                )?;
            } else {
                installed_count += 1;
                writeln!(
                    stdout,
                    "  [installed] {} ({} files, {} bytes)",
                    id,
                    install_result.installed_files.len(),
                    install_result.total_size_bytes
                )?;
            }
        }

        writeln!(
            stdout,
            "\nInstallation completed: {installed_count} installed, {cached_count} reused from cache."
        )?;
        Ok(())
    }
}
