//! Implementation of the `crux integrate cmake` command.

use crate::build::BuildProfile;
use crate::cli::CliError;
use crate::cli::load_active_catalog;
use crate::integrate::{
    CMakeIntegrationGenerator, DEFAULT_CMAKE_INTEGRATION_SUBPATH, IntegrationError,
    collect_transitive_dependencies, validate_package_installation,
};
use crate::package::PackageName;
use crate::project::manifest::MANIFEST_FILENAME;
use crate::resolution::lockfile::{
    LOCKFILE_FILENAME, LockError, LockedPackage, Lockfile, check_lockfile_sync,
};
use std::io::Write;
use std::path::Path;

pub fn run_integrate_cmake<W: Write, E: Write>(
    project_dir: &Path,
    package_spec: Option<&str>,
    profile: BuildProfile,
    locked: bool,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    let manifest_path = project_dir.join(MANIFEST_FILENAME);
    let lockfile_path = project_dir.join(LOCKFILE_FILENAME);

    if !manifest_path.is_file() {
        let err = CliError::General(format!(
            "manifest not found at '{}'. Run 'crux init' to initialize a project.",
            manifest_path.display()
        ));
        writeln!(stderr, "{err}")?;
        return Err(err);
    }

    if !lockfile_path.is_file() {
        let err = CliError::Integration(Box::new(IntegrationError::LockfileMissing(
            lockfile_path.clone(),
        )));
        writeln!(stderr, "{err}")?;
        return Err(err);
    }

    let lockfile = match Lockfile::load_from_path(&lockfile_path) {
        Ok(lf) => lf,
        Err(e) => {
            let err = CliError::Lock(LockError::Lockfile(e));
            writeln!(stderr, "{err}")?;
            return Err(err);
        }
    };

    if locked {
        let (catalog, _) = load_active_catalog(project_dir, stderr)?;
        if let Err(err) = check_lockfile_sync(project_dir, &catalog) {
            let cli_err = CliError::Integration(Box::new(IntegrationError::LockfileOutOfSync(
                err.to_string(),
            )));
            writeln!(stderr, "{cli_err}")?;
            return Err(cli_err);
        }
    }

    // Determine target packages to integrate
    let target_packages: Vec<LockedPackage> = if let Some(spec) = package_spec {
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

        let matched_name = if lockfile.packages.iter().any(|p| p.name == raw_pkg_name) {
            raw_pkg_name
        } else if let Some(m) = lockfile
            .packages
            .iter()
            .find(|p| p.name.as_str().eq_ignore_ascii_case(raw_pkg_name.as_str()))
        {
            m.name.clone()
        } else {
            let err = CliError::Integration(Box::new(IntegrationError::PackageNotFoundInLockfile(
                name_str.to_string(),
            )));
            writeln!(stderr, "{err}")?;
            return Err(err);
        };

        if let Some(expected_ver) = ver_req {
            let parsed_ver = semver::Version::parse(expected_ver).map_err(|e| {
                let err = CliError::General(format!("invalid version '{expected_ver}': {e}"));
                writeln!(stderr, "{err}").ok();
                err
            })?;
            let locked_pkg = lockfile
                .packages
                .iter()
                .find(|p| p.name == matched_name)
                .unwrap();
            if locked_pkg.version != parsed_ver {
                let err = CliError::General(format!(
                    "package '{name_str}' is locked at version {} in crux.lock, but requested {}",
                    locked_pkg.version, parsed_ver
                ));
                writeln!(stderr, "{err}")?;
                return Err(err);
            }
        }

        collect_transitive_dependencies(&lockfile, matched_name.as_str())
            .map_err(|e| CliError::Integration(Box::new(e)))?
    } else {
        let mut pkgs = lockfile.packages.clone();
        pkgs.sort_by(|a, b| a.name.cmp(&b.name));
        pkgs
    };

    let mut validated = Vec::new();
    for pkg in &target_packages {
        match validate_package_installation(
            project_dir,
            pkg.name.as_str(),
            &pkg.version.to_string(),
            profile,
        ) {
            Ok(v) => validated.push(v),
            Err(e) => {
                let cli_err = CliError::Integration(Box::new(e));
                writeln!(stderr, "{cli_err}")?;
                return Err(cli_err);
            }
        }
    }

    let generator = CMakeIntegrationGenerator::new(project_dir).with_profile(profile);
    let result = generator
        .integrate_packages(&validated)
        .map_err(|e| CliError::Integration(Box::new(e)))?;

    if validated.is_empty() {
        writeln!(stdout, "Integrated 0 packages into CMake configuration.")?;
    } else if validated.len() == 1 {
        writeln!(stdout, "Integrated 1 package into CMake configuration:")?;
        let p = &validated[0];
        let rel_dir = p
            .install_dir
            .strip_prefix(project_dir)
            .unwrap_or(&p.install_dir);
        writeln!(
            stdout,
            "  - {} {} ({})",
            p.package_id.name(),
            p.package_id.version(),
            rel_dir.display()
        )?;
    } else {
        writeln!(
            stdout,
            "Integrated {} packages into CMake configuration:",
            validated.len()
        )?;
        for p in &validated {
            let rel_dir = p
                .install_dir
                .strip_prefix(project_dir)
                .unwrap_or(&p.install_dir);
            writeln!(
                stdout,
                "  - {} {} ({})",
                p.package_id.name(),
                p.package_id.version(),
                rel_dir.display()
            )?;
        }
    }

    writeln!(stdout, "\nGenerated: {}", result.output_path.display())?;
    writeln!(
        stdout,
        "\nTo use in your CMakeLists.txt:\n    include(\"${{CMAKE_CURRENT_SOURCE_DIR}}/{}\")",
        DEFAULT_CMAKE_INTEGRATION_SUBPATH
    )?;

    Ok(())
}
