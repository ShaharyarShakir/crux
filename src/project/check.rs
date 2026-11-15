//! Project manifest verification and sanity checking.

use super::manifest::MANIFEST_FILENAME;
use crate::package::{Dependency, PackageId};
use crate::project::manifest::{Manifest, ManifestError};
use std::fmt;
use std::path::{Path, PathBuf};

/// Errors that can occur during `crux check`.
#[derive(Debug)]
pub enum CheckError {
    ManifestNotFound(PathBuf),
    Manifest(ManifestError),
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckError::ManifestNotFound(path) => {
                write!(f, "error: manifest not found at '{}'", path.display())
            }
            CheckError::Manifest(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for CheckError {}

impl From<ManifestError> for CheckError {
    fn from(err: ManifestError) -> Self {
        CheckError::Manifest(err)
    }
}

/// Report containing metadata of a successfully validated project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub manifest_path: PathBuf,
    pub package_id: PackageId,
    pub package_name: String,
    pub version: String,
    pub dependencies: Vec<Dependency>,
}

/// Verifies and inspects the manifest in the specified directory.
pub fn check_project(manifest_dir: &Path) -> Result<CheckReport, CheckError> {
    let manifest_path = manifest_dir.join(MANIFEST_FILENAME);
    if !manifest_path.is_file() {
        return Err(CheckError::ManifestNotFound(manifest_path));
    }

    let manifest = Manifest::load_from_path(&manifest_path)?;

    Ok(CheckReport {
        manifest_path,
        package_id: manifest.package.id(),
        package_name: manifest.package.name.to_string(),
        version: manifest.package.version.to_string(),
        dependencies: manifest.dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("crux_check_unit_{prefix}_{nanos}"));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_check_project_success() {
        let test_dir = TestDir::new("valid");
        let manifest_path = test_dir.path.join(MANIFEST_FILENAME);
        let content = r#"
[package]
name = "math-lib"
version = "2.0.1"
description = "Fast arithmetic"
"#;
        fs::write(&manifest_path, content).unwrap();

        let report = check_project(&test_dir.path).unwrap();
        assert_eq!(report.package_name, "math-lib");
        assert_eq!(report.version, "2.0.1");
        assert_eq!(report.package_id.to_string(), "math-lib 2.0.1");
        assert!(report.dependencies.is_empty());
    }

    #[test]
    fn test_check_project_with_dependencies() {
        let test_dir = TestDir::new("with_deps");
        let manifest_path = test_dir.path.join(MANIFEST_FILENAME);
        let content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
zlib = "~1.3"
"#;
        fs::write(&manifest_path, content).unwrap();

        let report = check_project(&test_dir.path).unwrap();
        assert_eq!(report.dependencies.len(), 2);
        assert_eq!(report.dependencies[0].name().as_str(), "fmt");
        assert_eq!(report.dependencies[1].name().as_str(), "zlib");
    }

    #[test]
    fn test_check_project_manifest_not_found() {
        let test_dir = TestDir::new("missing");
        let res = check_project(&test_dir.path);
        assert!(matches!(res, Err(CheckError::ManifestNotFound(_))));
    }

    #[test]
    fn test_check_project_invalid_manifest() {
        let test_dir = TestDir::new("invalid");
        let manifest_path = test_dir.path.join(MANIFEST_FILENAME);
        let content = r#"
[package]
name = "math-lib"
version = "invalid-version"
"#;
        fs::write(&manifest_path, content).unwrap();

        let res = check_project(&test_dir.path);
        assert!(matches!(res, Err(CheckError::Manifest(_))));
    }
}
