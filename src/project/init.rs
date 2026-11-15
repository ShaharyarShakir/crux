//! Package initialization logic for Crux projects.

use super::manifest::MANIFEST_FILENAME;
use crate::package::{NameValidationError, validate_package_name};
use crate::project::manifest::Manifest;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Errors that can occur during project initialization.
#[derive(Debug)]
pub enum InitError {
    ManifestAlreadyExists(PathBuf),
    InvalidPackageName(NameValidationError),
    InvalidDirectory(String),
    Io(io::Error),
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InitError::ManifestAlreadyExists(path) => {
                write!(f, "error: manifest already exists at '{}'", path.display())
            }
            InitError::InvalidPackageName(err) => {
                write!(f, "error: invalid project name: {err}")
            }
            InitError::InvalidDirectory(reason) => {
                write!(f, "error: invalid directory: {reason}")
            }
            InitError::Io(err) => write!(f, "error: I/O error: {err}"),
        }
    }
}

impl std::error::Error for InitError {}

impl From<io::Error> for InitError {
    fn from(err: io::Error) -> Self {
        InitError::Io(err)
    }
}

impl From<NameValidationError> for InitError {
    fn from(err: NameValidationError) -> Self {
        InitError::InvalidPackageName(err)
    }
}

/// Information returned after successful project initialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitSummary {
    pub name: String,
    pub manifest_path: PathBuf,
}

/// Derives a package name from a directory path.
pub fn derive_package_name(dir: &Path) -> Result<String, InitError> {
    let resolved = if dir.as_os_str().is_empty() || dir == Path::new(".") {
        std::env::current_dir().map_err(InitError::Io)?
    } else if dir.is_relative() {
        std::env::current_dir().map_err(InitError::Io)?.join(dir)
    } else {
        dir.to_path_buf()
    };

    let dir_name = resolved
        .file_name()
        .ok_or_else(|| {
            InitError::InvalidDirectory(
                "cannot derive project name from root directory; specify a name with '--name <NAME>'"
                    .to_string(),
            )
        })?
        .to_str()
        .ok_or_else(|| {
            InitError::InvalidDirectory(
                "directory name is not valid UTF-8; specify a name with '--name <NAME>'".to_string(),
            )
        })?;

    validate_package_name(dir_name)?;
    Ok(dir_name.to_string())
}

/// Initializes a new Crux package in the specified directory.
///
/// # Behavior
/// 1. Determines the project name from `custom_name` or the directory name.
/// 2. Validates the project name against standard Crux rules.
/// 3. Ensures `crux.toml` does not already exist (atomic check using `create_new`).
/// 4. Writes initial manifest contents.
pub fn init_project(
    target_dir: &Path,
    custom_name: Option<&str>,
) -> Result<InitSummary, InitError> {
    let project_name = match custom_name {
        Some(name) => {
            validate_package_name(name)?;
            name.to_string()
        }
        None => derive_package_name(target_dir)?,
    };

    let manifest = Manifest::new(&project_name)?;
    let manifest_path = target_dir.join(MANIFEST_FILENAME);

    // Atomically create the file only if it doesn't already exist to avoid race conditions and overwrites.
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            return Err(InitError::ManifestAlreadyExists(manifest_path));
        }
        Err(err) => return Err(InitError::Io(err)),
    };

    file.write_all(manifest.to_toml_string().as_bytes())?;

    Ok(InitSummary {
        name: project_name,
        manifest_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
            let path = std::env::temp_dir().join(format!("crux_unit_{prefix}_{nanos}"));
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
    fn test_init_default_name_in_empty_dir() {
        let test_dir = TestDir::new("my_pkg");
        let summary = init_project(&test_dir.path, None).unwrap();

        assert!(summary.name.starts_with("crux_unit_my_pkg_"));
        assert!(summary.manifest_path.exists());

        let contents = fs::read_to_string(&summary.manifest_path).unwrap();
        assert!(contents.contains(&format!("name = \"{}\"", summary.name)));
        assert!(contents.contains("version = \"0.1.0\""));
    }

    #[test]
    fn test_init_with_custom_name() {
        let test_dir = TestDir::new("arbitrary_dir");
        let summary = init_project(&test_dir.path, Some("custom-c-lib")).unwrap();

        assert_eq!(summary.name, "custom-c-lib");
        let contents = fs::read_to_string(&summary.manifest_path).unwrap();
        assert!(contents.contains("name = \"custom-c-lib\""));
    }

    #[test]
    fn test_init_rejects_existing_manifest() {
        let test_dir = TestDir::new("existing_test");
        let manifest_path = test_dir.path.join(MANIFEST_FILENAME);
        fs::write(&manifest_path, "# existing content\n").unwrap();

        let result = init_project(&test_dir.path, Some("my-lib"));
        assert!(matches!(result, Err(InitError::ManifestAlreadyExists(_))));

        // Verify the original content was not altered
        let contents = fs::read_to_string(&manifest_path).unwrap();
        assert_eq!(contents, "# existing content\n");
    }

    #[test]
    fn test_init_rejects_invalid_custom_name() {
        let test_dir = TestDir::new("invalid_name_test");
        let result = init_project(&test_dir.path, Some("-bad-name"));
        assert!(matches!(
            result,
            Err(InitError::InvalidPackageName(
                NameValidationError::StartsWithHyphen
            ))
        ));
        assert!(!test_dir.path.join(MANIFEST_FILENAME).exists());
    }

    #[test]
    fn test_init_in_directory_path_with_spaces() {
        let parent = TestDir::new("space_test");
        let spaced_path = parent.path.join("path with spaces").join("my_proj");
        fs::create_dir_all(&spaced_path).unwrap();

        let summary = init_project(&spaced_path, None).unwrap();
        assert_eq!(summary.name, "my_proj");
        assert!(summary.manifest_path.exists());
    }
}
