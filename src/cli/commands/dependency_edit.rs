//! Shared transactional write helper for dependency-editing commands.
//!
//! `crux add` and `crux remove` both update `crux.toml` and `crux.lock`. The
//! manifest is written first so the project never references a dependency that
//! is absent from the manifest, and it is restored if the lockfile write fails.
//! Keeping this logic in one place guarantees both commands share identical
//! transactional semantics.

use crate::cli::CliError;
use crate::core::filesystem::write_atomically;
use crate::resolution::lockfile::{LockError, Lockfile};
use std::io::Write;
use std::path::Path;

/// Writes an updated manifest and lockfile as a single transaction.
///
/// Returns a lockfile error (after restoring the previous manifest contents)
/// when the lockfile cannot be written.
pub(crate) fn commit_manifest_and_lockfile<W: Write, E: Write>(
    project_dir: &Path,
    manifest_path: &Path,
    original_content: &str,
    updated_content: &str,
    lockfile: &Lockfile,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError> {
    write_atomically(manifest_path, updated_content.as_bytes()).map_err(|err| {
        writeln!(stderr, "error: failed to write crux.toml: {err}").ok();
        CliError::Io(err.to_string())
    })?;

    writeln!(stdout, "Updated crux.toml")?;

    if let Err(err) = lockfile.write_atomic(project_dir) {
        if let Err(restore_err) = write_atomically(manifest_path, original_content.as_bytes()) {
            writeln!(
                stderr,
                "error: failed to write crux.lock: {err}\n\
                 error: additionally failed to restore crux.toml: {restore_err}"
            )?;
        } else {
            writeln!(
                stderr,
                "error: failed to write crux.lock: {err}\n\
                 crux.toml was restored to its previous contents"
            )?;
        }
        return Err(CliError::Lock(LockError::Lockfile(err)));
    }

    writeln!(
        stdout,
        "Updated crux.lock ({} packages locked)",
        lockfile.package_count()
    )?;
    if lockfile.packages.is_empty() {
        writeln!(stdout, "  (no packages)")?;
    } else {
        for pkg in &lockfile.packages {
            writeln!(stdout, "  {} {} ({})", pkg.name, pkg.version, pkg.source)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::PackageSource;
    use crate::package::PackageName;
    use crate::resolution::lockfile::LockedPackage;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "crux_dependency_edit_{name}_{}_{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_lockfile() -> Lockfile {
        let mut lockfile = Lockfile::new();
        lockfile.packages.push(LockedPackage::new(
            PackageName::new("fmt").unwrap(),
            semver::Version::parse("11.1.1").unwrap(),
            PackageSource::registry("default"),
        ));
        lockfile
    }

    #[test]
    fn test_commit_writes_manifest_and_lockfile() {
        let dir = temp_dir("success");
        let manifest_path = dir.join(crate::project::manifest::MANIFEST_FILENAME);
        fs::write(&manifest_path, "original\n").unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        commit_manifest_and_lockfile(
            &dir,
            &manifest_path,
            "original\n",
            "updated\n",
            &sample_lockfile(),
            &mut out,
            &mut err,
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&manifest_path).unwrap(), "updated\n");
        assert!(dir.join(crate::resolution::lockfile::LOCKFILE_FILENAME).is_file());
        let stdout = String::from_utf8(out).unwrap();
        assert!(stdout.contains("Updated crux.toml"));
        assert!(stdout.contains("Updated crux.lock (1 packages locked)"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_failed_lockfile_write_restores_manifest() {
        let dir = temp_dir("rollback");
        let manifest_path = dir.join(crate::project::manifest::MANIFEST_FILENAME);
        fs::write(&manifest_path, "original\n").unwrap();

        // A directory at the lockfile path makes the atomic rename fail.
        let lockfile_path = dir.join(crate::resolution::lockfile::LOCKFILE_FILENAME);
        fs::create_dir_all(&lockfile_path).unwrap();
        fs::write(lockfile_path.join("occupied"), b"x").unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = commit_manifest_and_lockfile(
            &dir,
            &manifest_path,
            "original\n",
            "updated\n",
            &sample_lockfile(),
            &mut out,
            &mut err,
        );

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(&manifest_path).unwrap(),
            "original\n",
            "manifest must be restored after a failed lockfile write"
        );
        let stderr = String::from_utf8(err).unwrap();
        assert!(stderr.contains("failed to write crux.lock"));
        assert!(stderr.contains("crux.toml was restored"));
        let _ = fs::remove_dir_all(dir);
    }
}
