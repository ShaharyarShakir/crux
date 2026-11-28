//! Current-installation records.
//!
//! A package version can have several installations side by side, one per build
//! fingerprint. Fingerprints are hashes, so nothing about a directory name says
//! which installation is current. The install engine records that explicitly in
//! `<install_root>/<package>/<version>/current-<profile>.toml`.

use crate::build::{BuildFingerprint, BuildProfile};
use crate::core::filesystem::write_atomically;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Current format version for current-installation records.
pub(crate) const CURRENT_RECORD_FORMAT_VERSION: u32 = 1;

/// Which installation is current for one `(package, version, profile)`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CurrentInstallation {
    pub format_version: u32,
    pub package_name: String,
    pub package_version: String,
    pub profile: BuildProfile,
    pub fingerprint: BuildFingerprint,
    /// Checksum (`sha256:<hex>`) of the source that produced the installation.
    /// Absent when unknown, e.g. for adopted pre-record installations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_checksum: Option<String>,
    /// Dependency installations (`name = "<version>@sha256:<hex>"`) this one
    /// was built against. Empty when there are none or they are unknown.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, String>,
}

/// Path of the record for `profile` inside a package version directory.
pub(crate) fn current_record_path(version_dir: &Path, profile: BuildProfile) -> PathBuf {
    version_dir.join(format!("current-{}.toml", profile.as_str()))
}

/// Reads the record for `profile`.
///
/// Returns `Ok(None)` when there is no record, and `Err` with a reason when one
/// exists but cannot be trusted.
pub(crate) fn read_current_record(
    version_dir: &Path,
    profile: BuildProfile,
) -> Result<Option<CurrentInstallation>, String> {
    let path = current_record_path(version_dir, profile);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("failed to read '{}': {err}", path.display())),
    };
    let record: CurrentInstallation = toml::from_str(&text)
        .map_err(|err| format!("failed to parse '{}': {err}", path.display()))?;

    if record.format_version != CURRENT_RECORD_FORMAT_VERSION {
        return Err(format!(
            "'{}' has unsupported format version {}",
            path.display(),
            record.format_version
        ));
    }
    // Deserialization does not validate fingerprints, and this one is used as
    // a directory name.
    BuildFingerprint::new(record.fingerprint.as_str())
        .map_err(|err| format!("'{}': {err}", path.display()))?;
    Ok(Some(record))
}

/// Atomically writes the record into `version_dir`.
pub(crate) fn write_current_record(
    version_dir: &Path,
    record: &CurrentInstallation,
) -> io::Result<()> {
    let text = toml::to_string_pretty(record).map_err(io::Error::other)?;
    write_atomically(
        &current_record_path(version_dir, record.profile),
        text.as_bytes(),
    )
}

/// Removes the record for `profile`, if any.
pub(crate) fn remove_current_record(version_dir: &Path, profile: BuildProfile) -> io::Result<()> {
    match fs::remove_file(current_record_path(version_dir, profile)) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::filesystem::unique_temp_suffix;

    fn record() -> CurrentInstallation {
        CurrentInstallation {
            format_version: CURRENT_RECORD_FORMAT_VERSION,
            package_name: "demo".to_string(),
            package_version: "1.0.0".to_string(),
            profile: BuildProfile::Debug,
            fingerprint: BuildFingerprint::new(format!("sha256:{}", "a".repeat(64))).unwrap(),
            source_checksum: None,
            dependencies: BTreeMap::from([("leaf".to_string(), "1.0.0@sha256:x".to_string())]),
        }
    }

    #[test]
    fn records_round_trip_per_profile_and_reject_untrusted_content() {
        let dir = std::env::temp_dir().join(format!("crux_current_{}", unique_temp_suffix()));
        assert_eq!(read_current_record(&dir, BuildProfile::Debug), Ok(None));

        write_current_record(&dir, &record()).unwrap();
        assert_eq!(
            read_current_record(&dir, BuildProfile::Debug),
            Ok(Some(record()))
        );
        assert_eq!(read_current_record(&dir, BuildProfile::Release), Ok(None));

        let path = current_record_path(&dir, BuildProfile::Debug);
        let good = fs::read_to_string(&path).unwrap();
        for bad in [
            String::new(),
            good[..good.len() / 2].to_string(),
            good.replace("format_version = 1", "format_version = 2"),
            good.replace(&format!("sha256:{}", "a".repeat(64)), "sha256:../../x"),
            good.replace(&format!("sha256:{}", "a".repeat(64)), "x"),
        ] {
            fs::write(&path, &bad).unwrap();
            assert!(
                read_current_record(&dir, BuildProfile::Debug).is_err(),
                "accepted: {bad:?}"
            );
        }

        remove_current_record(&dir, BuildProfile::Debug).unwrap();
        remove_current_record(&dir, BuildProfile::Debug).unwrap();
        let _ = fs::remove_dir_all(dir);
    }
}
