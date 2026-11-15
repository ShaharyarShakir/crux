//! Installation inventory scanning, validation, and prefix promotion.

use super::InstallError;
use super::metadata::{INSTALL_METADATA_FILENAME, InstalledFile, InstalledFileType};
use crate::core::paths::normalize_path;
use std::fs;
use std::path::{Component, Path};

/// Recursively scans and validates staged files.
pub fn scan_staging_dir(staging_dir: &Path) -> Result<(Vec<InstalledFile>, u64), InstallError> {
    let mut installed_files = Vec::new();
    let mut total_size = 0u64;

    scan_recursive(
        staging_dir,
        staging_dir,
        &mut installed_files,
        &mut total_size,
    )?;

    if installed_files.is_empty() {
        return Err(InstallError::EmptyInstallation(staging_dir.to_path_buf()));
    }

    installed_files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok((installed_files, total_size))
}

fn scan_recursive(
    current_dir: &Path,
    staging_root: &Path,
    files: &mut Vec<InstalledFile>,
    total_size: &mut u64,
) -> Result<(), InstallError> {
    let entries = fs::read_dir(current_dir).map_err(InstallError::Io)?;

    for entry in entries {
        let entry = entry.map_err(InstallError::Io)?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        // Ignore temporary metadata artifacts
        if file_name == INSTALL_METADATA_FILENAME || file_name.starts_with('.') {
            continue;
        }

        let meta = fs::symlink_metadata(&path).map_err(InstallError::Io)?;
        let rel_path = path
            .strip_prefix(staging_root)
            .map_err(|_| {
                InstallError::SecurityViolation(format!(
                    "path '{}' escapes staging root '{}'",
                    path.display(),
                    staging_root.display()
                ))
            })?
            .to_path_buf();

        // Validate no traversal components
        if rel_path
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(InstallError::SecurityViolation(format!(
                "path traversal detected in staged path: '{}'",
                rel_path.display()
            )));
        }

        if meta.is_symlink() {
            let target = fs::read_link(&path).map_err(InstallError::Io)?;
            let resolved = if target.is_relative() {
                let parent = path.parent().unwrap_or(staging_root);
                normalize_path(&parent.join(&target))
            } else {
                target.clone()
            };

            if !resolved.starts_with(staging_root) {
                return Err(InstallError::SecurityViolation(format!(
                    "symlink '{}' points outside installation staging root: '{}'",
                    rel_path.display(),
                    target.display()
                )));
            }

            let file_type = InstalledFileType::classify(&rel_path, &path);
            let size_bytes = meta.len();
            *total_size += size_bytes;

            files.push(InstalledFile {
                relative_path: rel_path,
                file_type,
                size_bytes,
            });
        } else if meta.is_dir() {
            scan_recursive(&path, staging_root, files, total_size)?;
        } else if meta.is_file() {
            let file_type = InstalledFileType::classify(&rel_path, &path);
            let size_bytes = meta.len();
            *total_size += size_bytes;

            files.push(InstalledFile {
                relative_path: rel_path,
                file_type,
                size_bytes,
            });
        } else {
            return Err(InstallError::SecurityViolation(format!(
                "unsupported filesystem object type at '{}'",
                rel_path.display()
            )));
        }
    }

    Ok(())
}

/// Promotes contents installed to `usr/local` or `usr` prefix up to the root staging directory.
pub fn promote_unwrapped_prefix(staging_dir: &Path) -> Result<(), InstallError> {
    let usr_local = staging_dir.join("usr").join("local");
    let prefix = if usr_local.is_dir() {
        usr_local
    } else {
        let usr = staging_dir.join("usr");
        if usr.is_dir() {
            usr
        } else {
            return Ok(());
        }
    };

    merge_dir_contents(&prefix, staging_dir)?;
    let _ = fs::remove_dir_all(staging_dir.join("usr"));
    Ok(())
}

pub fn merge_dir_contents(src: &Path, dst: &Path) -> Result<(), InstallError> {
    if !src.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(src).map_err(InstallError::Io)?;
    for entry in entries {
        let entry = entry.map_err(InstallError::Io)?;
        let src_path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if src_path.is_dir() && dest_path.is_dir() {
            merge_dir_contents(&src_path, &dest_path)?;
            let _ = fs::remove_dir(src_path);
        } else {
            fs::rename(&src_path, &dest_path).map_err(InstallError::Io)?;
        }
    }
    Ok(())
}
