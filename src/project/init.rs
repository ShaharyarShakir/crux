//! Package initialization logic for Crux projects.

use super::manifest::{
    MANIFEST_FILENAME, Manifest, ManifestValidationError, ProjectLanguage,
    default_standard_for_language, is_valid_c_standard, is_valid_cpp_standard, normalize_standard,
};
use crate::package::{NameValidationError, validate_package_name};
use crate::resolution::lockfile::{LOCKFILE_FILENAME, Lockfile, LockfileError};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Errors that can occur during project initialization.
#[derive(Debug)]
pub enum InitError {
    ManifestAlreadyExists(PathBuf),
    FileAlreadyExists(PathBuf),
    InvalidPackageName(NameValidationError),
    InvalidDirectory(String),
    Validation(ManifestValidationError),
    Lockfile(LockfileError),
    Io(io::Error),
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InitError::ManifestAlreadyExists(path) => {
                write!(f, "error: manifest already exists at '{}'", path.display())
            }
            InitError::FileAlreadyExists(path) => {
                write!(f, "error: file already exists at '{}'", path.display())
            }
            InitError::InvalidPackageName(err) => {
                write!(f, "error: invalid project name: {err}")
            }
            InitError::InvalidDirectory(reason) => {
                write!(f, "error: invalid directory: {reason}")
            }
            InitError::Validation(err) => {
                write!(f, "error: {err}")
            }
            InitError::Lockfile(err) => {
                write!(f, "error: failed to generate lockfile: {err}")
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

impl From<ManifestValidationError> for InitError {
    fn from(err: ManifestValidationError) -> Self {
        InitError::Validation(err)
    }
}

impl From<LockfileError> for InitError {
    fn from(err: LockfileError) -> Self {
        InitError::Lockfile(err)
    }
}

/// Options controlling project initialization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitOptions<'a> {
    pub custom_name: Option<&'a str>,
    pub language: Option<ProjectLanguage>,
    pub standard: Option<&'a str>,
}

/// Information returned after successful project initialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitSummary {
    pub name: String,
    pub manifest_path: PathBuf,
    pub lockfile_path: PathBuf,
    pub language: ProjectLanguage,
    pub standard: Option<String>,
    pub created_files: Vec<PathBuf>,
}

/// Normalizes a project name into a safe identifier suitable for C/C++ header directories and namespaces.
pub fn sanitize_header_dir_name(project_name: &str) -> String {
    project_name.to_lowercase().replace('-', "_")
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

/// Creates a new file atomically, failing safely if the file already exists.
fn create_new_file(path: &Path, content: &[u8]) -> Result<(), InitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            return Err(InitError::FileAlreadyExists(path.to_path_buf()));
        }
        Err(err) => return Err(InitError::Io(err)),
    };

    file.write_all(content)?;
    Ok(())
}

/// Initializes a new Crux package in the specified directory using default options (C++20).
pub fn init_project(
    target_dir: &Path,
    custom_name: Option<&str>,
) -> Result<InitSummary, InitError> {
    init_project_with_options(
        target_dir,
        &InitOptions {
            custom_name,
            language: None,
            standard: None,
        },
    )
}

/// Initializes a new Crux package with the given options.
///
/// # Behavior
/// 1. Determines and validates the project name.
/// 2. Validates language and language-standard combinations.
/// 3. Ensures destination directory exists and checks that no target files already exist.
/// 4. Generates minimal Crux directory structure (`src/`).
/// 5. Writes starter main file for C and C++ or leaves `src/` empty for Other.
/// 6. Generates valid `crux.toml` and initialized `crux.lock`.
pub fn init_project_with_options(
    target_dir: &Path,
    options: &InitOptions<'_>,
) -> Result<InitSummary, InitError> {
    let project_name = match options.custom_name {
        Some(name) => {
            validate_package_name(name)?;
            name.to_string()
        }
        None => derive_package_name(target_dir)?,
    };

    let language = options.language.unwrap_or(ProjectLanguage::Cpp);

    let standard = match options.standard {
        Some(s) => match language {
            ProjectLanguage::C => {
                if !is_valid_c_standard(s) {
                    return Err(InitError::Validation(
                        ManifestValidationError::UnsupportedStandard {
                            language,
                            standard: s.to_string(),
                        },
                    ));
                }
                Some(s.to_ascii_lowercase())
            }
            ProjectLanguage::Cpp => {
                if !is_valid_cpp_standard(s) {
                    return Err(InitError::Validation(
                        ManifestValidationError::UnsupportedStandard {
                            language,
                            standard: s.to_string(),
                        },
                    ));
                }
                Some(normalize_standard(s))
            }
            ProjectLanguage::Other => {
                return Err(InitError::Validation(
                    ManifestValidationError::StandardNotAllowedForLanguage {
                        language,
                        standard: s.to_string(),
                    },
                ));
            }
        },
        None => default_standard_for_language(language).map(|s| s.to_string()),
    };

    let manifest_path = target_dir.join(MANIFEST_FILENAME);
    let lockfile_path = target_dir.join(LOCKFILE_FILENAME);

    // Initial check for manifest existence
    if manifest_path.exists() {
        return Err(InitError::ManifestAlreadyExists(manifest_path));
    }
    if lockfile_path.exists() {
        return Err(InitError::FileAlreadyExists(lockfile_path));
    }

    // Prepare manifest
    let mut manifest = Manifest::new(&project_name)?;
    manifest = manifest.with_language(language);
    if let Some(ref std_str) = standard {
        manifest = manifest.with_standard(std_str);
    }
    let manifest_bytes = manifest.to_toml_string().into_bytes();

    // Prepare lockfile
    let lockfile = Lockfile::new();
    let lockfile_bytes = lockfile.to_toml_string()?.into_bytes();

    // Build the list of files to be created
    let mut files_to_create: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    files_to_create.push((manifest_path.clone(), manifest_bytes));
    files_to_create.push((lockfile_path.clone(), lockfile_bytes));

    match language {
        ProjectLanguage::C => {
            let main = format!(
                "#include <stdio.h>\n\nint main(void) {{\n    printf(\"Hello from {project_name}!\\n\");\n    return 0;\n}}\n"
            );
            files_to_create.push((target_dir.join("src").join("main.c"), main.into_bytes()));
        }
        ProjectLanguage::Cpp => {
            let main = format!(
                "#include <iostream>\n\nint main() {{\n    std::cout << \"Hello from {project_name}!\\n\";\n    return 0;\n}}\n"
            );
            files_to_create.push((target_dir.join("src").join("main.cpp"), main.into_bytes()));
        }
        ProjectLanguage::Other => {
            // No starter source or headers for Other
        }
    }

    // Verify no files to be created already exist
    for (path, _) in &files_to_create {
        if path.exists() {
            if path == &manifest_path {
                return Err(InitError::ManifestAlreadyExists(path.clone()));
            }
            return Err(InitError::FileAlreadyExists(path.clone()));
        }
    }

    // Create minimal directory structure (src/ only)
    // Architectural rule: Crux recursively discovers application source files under src/ only.
    // Optional directories (include/, tests/, src/modules/, .crux/) are user-created or created
    // by future build commands when needed.
    fs::create_dir_all(target_dir.join("src"))?;

    let mut created_files = Vec::with_capacity(files_to_create.len());
    for (path, content) in files_to_create {
        create_new_file(&path, &content)?;
        created_files.push(path);
    }

    Ok(InitSummary {
        name: project_name,
        manifest_path,
        lockfile_path,
        language,
        standard,
        created_files,
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

    #[test]
    fn test_init_c_project() {
        let test_dir = TestDir::new("c_project");
        let options = InitOptions {
            custom_name: Some("c-math-app"),
            language: Some(ProjectLanguage::C),
            standard: None,
        };
        let summary = init_project_with_options(&test_dir.path, &options).unwrap();

        assert_eq!(summary.name, "c-math-app");
        assert_eq!(summary.language, ProjectLanguage::C);
        assert_eq!(summary.standard.as_deref(), Some("c17"));
        assert!(summary.manifest_path.is_file());
        assert!(summary.lockfile_path.is_file());

        let manifest_content = fs::read_to_string(&summary.manifest_path).unwrap();
        assert!(manifest_content.contains("[project]"));
        assert!(manifest_content.contains("name = \"c-math-app\""));
        assert!(manifest_content.contains("language = \"c\""));
        assert!(manifest_content.contains("standard = \"c17\""));

        let main_c = test_dir.path.join("src").join("main.c");
        assert!(main_c.is_file());
        let main_content = fs::read_to_string(&main_c).unwrap();
        assert!(main_content.contains("Hello from c-math-app!"));

        assert!(!test_dir.path.join("include").exists());
        assert!(!test_dir.path.join("tests").exists());
        assert!(!test_dir.path.join(".crux").exists());
        assert!(!test_dir.path.join("src").join("modules").exists());
        assert!(!test_dir.path.join("CMakeLists.txt").exists());
    }

    #[test]
    fn test_init_cpp_project() {
        let test_dir = TestDir::new("cpp_project");
        let options = InitOptions {
            custom_name: Some("cpp-calc-service"),
            language: Some(ProjectLanguage::Cpp),
            standard: None,
        };
        let summary = init_project_with_options(&test_dir.path, &options).unwrap();

        assert_eq!(summary.name, "cpp-calc-service");
        assert_eq!(summary.language, ProjectLanguage::Cpp);
        assert_eq!(summary.standard.as_deref(), Some("c++20"));
        assert!(summary.manifest_path.is_file());
        assert!(summary.lockfile_path.is_file());

        let manifest_content = fs::read_to_string(&summary.manifest_path).unwrap();
        assert!(manifest_content.contains("[project]"));
        assert!(manifest_content.contains("name = \"cpp-calc-service\""));
        assert!(manifest_content.contains("language = \"cpp\""));
        assert!(manifest_content.contains("standard = \"c++20\""));

        let main_cpp = test_dir.path.join("src").join("main.cpp");
        assert!(main_cpp.is_file());
        let main_content = fs::read_to_string(&main_cpp).unwrap();
        assert!(main_content.contains("Hello from cpp-calc-service!"));

        assert!(!test_dir.path.join("include").exists());
        assert!(!test_dir.path.join("tests").exists());
        assert!(!test_dir.path.join(".crux").exists());
        assert!(!test_dir.path.join("src").join("modules").exists());
        assert!(!test_dir.path.join("CMakeLists.txt").exists());
    }

    #[test]
    fn test_init_other_project() {
        let test_dir = TestDir::new("other_project");
        let options = InitOptions {
            custom_name: Some("other-tool"),
            language: Some(ProjectLanguage::Other),
            standard: None,
        };
        let summary = init_project_with_options(&test_dir.path, &options).unwrap();

        assert_eq!(summary.name, "other-tool");
        assert_eq!(summary.language, ProjectLanguage::Other);
        assert_eq!(summary.standard, None);
        assert!(summary.manifest_path.is_file());
        assert!(summary.lockfile_path.is_file());

        let manifest_content = fs::read_to_string(&summary.manifest_path).unwrap();
        assert!(manifest_content.contains("[project]"));
        assert!(manifest_content.contains("name = \"other-tool\""));
        assert!(manifest_content.contains("language = \"other\""));
        assert!(!manifest_content.contains("standard ="));

        assert!(test_dir.path.join("src").is_dir());
        assert!(!test_dir.path.join("include").exists());
        assert!(!test_dir.path.join("tests").exists());
        assert!(!test_dir.path.join(".crux").exists());

        // Ensure NO main files or CMakeLists.txt exist
        assert!(!test_dir.path.join("src").join("main.c").exists());
        assert!(!test_dir.path.join("src").join("main.cpp").exists());
        assert!(!test_dir.path.join("CMakeLists.txt").exists());
    }

    #[test]
    fn test_init_standard_override() {
        let test_dir = TestDir::new("std_override");
        let options = InitOptions {
            custom_name: Some("c-legacy"),
            language: Some(ProjectLanguage::C),
            standard: Some("c11"),
        };
        let summary = init_project_with_options(&test_dir.path, &options).unwrap();
        assert_eq!(summary.standard.as_deref(), Some("c11"));

        let manifest = Manifest::load_from_path(&summary.manifest_path).unwrap();
        assert_eq!(manifest.standard(), Some("c11"));
    }

    #[test]
    fn test_init_rejects_mismatched_standard() {
        let test_dir = TestDir::new("mismatch_std");
        let options = InitOptions {
            custom_name: Some("bad-std-project"),
            language: Some(ProjectLanguage::C),
            standard: Some("c++20"),
        };
        let result = init_project_with_options(&test_dir.path, &options);
        assert!(matches!(
            result,
            Err(InitError::Validation(
                ManifestValidationError::UnsupportedStandard { .. }
            ))
        ));
    }

    #[test]
    fn test_init_rejects_standard_for_other() {
        let test_dir = TestDir::new("other_with_std");
        let options = InitOptions {
            custom_name: Some("bad-other"),
            language: Some(ProjectLanguage::Other),
            standard: Some("c17"),
        };
        let result = init_project_with_options(&test_dir.path, &options);
        assert!(matches!(
            result,
            Err(InitError::Validation(
                ManifestValidationError::StandardNotAllowedForLanguage { .. }
            ))
        ));
    }

    #[test]
    fn test_init_rejects_existing_lockfile() {
        let test_dir = TestDir::new("existing_lockfile");
        let lockfile_path = test_dir.path.join(LOCKFILE_FILENAME);
        fs::write(&lockfile_path, "# existing lockfile\n").unwrap();

        let options = InitOptions {
            custom_name: Some("conflicting-lock"),
            language: Some(ProjectLanguage::Cpp),
            standard: None,
        };
        let result = init_project_with_options(&test_dir.path, &options);
        assert!(matches!(result, Err(InitError::FileAlreadyExists(_))));
        assert_eq!(
            fs::read_to_string(&lockfile_path).unwrap(),
            "# existing lockfile\n"
        );
    }

    #[test]
    fn test_init_rejects_existing_starter_file() {
        let test_dir = TestDir::new("existing_source");
        let main_path = test_dir.path.join("src").join("main.cpp");
        fs::create_dir_all(main_path.parent().unwrap()).unwrap();
        fs::write(&main_path, "// existing code\n").unwrap();

        let options = InitOptions {
            custom_name: Some("conflicting-source"),
            language: Some(ProjectLanguage::Cpp),
            standard: None,
        };
        let result = init_project_with_options(&test_dir.path, &options);
        assert!(matches!(result, Err(InitError::FileAlreadyExists(_))));
        assert_eq!(
            fs::read_to_string(&main_path).unwrap(),
            "// existing code\n"
        );
    }
}
