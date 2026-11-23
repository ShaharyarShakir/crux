//! Project configuration model and query API for Crux projects.

use super::build_settings::BuildSettings;
use super::manifest::{MANIFEST_FILENAME, Manifest, ManifestError, ProjectLanguage};
use crate::package::Dependency;
use std::path::Path;

/// Represents the validated configuration of a Crux project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    pub name: String,
    pub version: semver::Version,
    pub language: Option<ProjectLanguage>,
    pub standard: Option<String>,
    pub is_project: bool,
    /// Declared direct dependencies, sorted by name.
    pub dependencies: Vec<Dependency>,
    /// `[build]` settings; apply to the application only, never dependencies.
    pub build: BuildSettings,
}

impl ProjectConfig {
    /// Creates a `ProjectConfig` from a parsed `Manifest`.
    pub fn from_manifest(manifest: &Manifest) -> Self {
        Self {
            name: manifest.package.name.to_string(),
            version: manifest.package.version.clone(),
            language: manifest.language,
            standard: manifest.standard.clone(),
            is_project: manifest.is_project,
            dependencies: manifest.dependencies.clone(),
            build: manifest.build.clone(),
        }
    }

    /// Loads and parses the project configuration from a directory containing `crux.toml`.
    pub fn load_from_dir(project_dir: &Path) -> Result<Self, ManifestError> {
        let manifest_path = project_dir.join(MANIFEST_FILENAME);
        let manifest = Manifest::load_from_path(&manifest_path)?;
        Ok(Self::from_manifest(&manifest))
    }

    /// Returns the project name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the project semantic version.
    pub fn version(&self) -> &semver::Version {
        &self.version
    }

    /// Returns the configured project language, if set.
    pub fn language(&self) -> Option<ProjectLanguage> {
        self.language
    }

    /// Returns the configured language standard, if set.
    pub fn standard(&self) -> Option<&str> {
        self.standard.as_deref()
    }

    /// Returns true if this is an ordinary Crux `[project]` (as opposed to a legacy `[package]`).
    pub fn is_project(&self) -> bool {
        self.is_project
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("crux_config_test_{prefix}_{nanos}"));
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
    fn test_project_config_from_manifest() {
        let toml_str = r#"
[project]
name = "my-sample-app"
version = "1.0.0"
language = "cpp"
standard = "c++20"

[dependencies]
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        let config = ProjectConfig::from_manifest(&manifest);

        assert_eq!(config.name(), "my-sample-app");
        assert_eq!(config.version().to_string(), "1.0.0");
        assert_eq!(config.language(), Some(ProjectLanguage::Cpp));
        assert_eq!(config.standard(), Some("c++20"));
        assert!(config.is_project());
    }

    #[test]
    fn test_project_config_load_from_dir() {
        let test_dir = TestDir::new("load_dir");
        let manifest_content = r#"
[project]
name = "dir-app"
version = "0.5.0"
language = "c"
standard = "c17"
"#;
        fs::write(test_dir.path.join(MANIFEST_FILENAME), manifest_content).unwrap();

        let config = ProjectConfig::load_from_dir(&test_dir.path).unwrap();
        assert_eq!(config.name(), "dir-app");
        assert_eq!(config.version().to_string(), "0.5.0");
        assert_eq!(config.language(), Some(ProjectLanguage::C));
        assert_eq!(config.standard(), Some("c17"));
    }

    #[test]
    fn test_project_config_legacy_package() {
        let toml_str = r#"
[package]
name = "legacy-pkg"
version = "0.1.0"
"#;
        let manifest = Manifest::from_toml_str(toml_str).unwrap();
        let config = ProjectConfig::from_manifest(&manifest);

        assert_eq!(config.name(), "legacy-pkg");
        assert_eq!(config.language(), None);
        assert_eq!(config.standard(), None);
        assert!(!config.is_project());
    }
}
