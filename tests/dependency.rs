use crux::dependency::Dependency;
use crux::manifest::{Manifest, ManifestError, ManifestValidationError};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
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
        let path = std::env::temp_dir().join(format!("crux_integ_dep_{prefix}_{nanos}"));
        fs::create_dir_all(&path).expect("failed to create temporary test directory");
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_manifest_without_dependencies() {
    let toml_str = r#"
[package]
name = "no-deps"
version = "1.0.0"
description = "Project without dependencies"
"#;
    let manifest = Manifest::from_toml_str(toml_str).unwrap();
    assert!(manifest.dependencies().is_empty());

    let serialized = manifest.to_toml_string();
    assert!(!serialized.contains("[dependencies]"));
}

#[test]
fn test_manifest_with_empty_dependencies_section() {
    let toml_str = r#"
[package]
name = "empty-deps"
version = "0.1.0"

[dependencies]
"#;
    let manifest = Manifest::from_toml_str(toml_str).unwrap();
    assert!(manifest.dependencies().is_empty());
}

#[test]
fn test_manifest_with_one_dependency() {
    let toml_str = r#"
[package]
name = "single-dep"
version = "0.1.0"

[dependencies]
fmt = "^11.1"
"#;
    let manifest = Manifest::from_toml_str(toml_str).unwrap();
    assert_eq!(manifest.dependencies().len(), 1);

    let dep = &manifest.dependencies()[0];
    assert_eq!(dep.name().as_str(), "fmt");
    assert_eq!(dep.version_req().to_string(), "^11.1");
    assert!(dep.matches(&semver::Version::parse("11.1.0").unwrap()));
    assert!(dep.matches(&semver::Version::parse("11.2.5").unwrap()));
    assert!(!dep.matches(&semver::Version::parse("12.0.0").unwrap()));
}

#[test]
fn test_manifest_with_multiple_dependencies_and_sorting() {
    let toml_str = r#"
[package]
name = "multi-dep"
version = "0.1.0"

[dependencies]
zlib = ">=1.2, <2.0"
fmt = "^11.1"
openssl = "~3.4"
"#;
    let manifest = Manifest::from_toml_str(toml_str).unwrap();
    assert_eq!(manifest.dependencies().len(), 3);

    // Dependencies are ordered alphabetically: fmt, openssl, zlib
    assert_eq!(manifest.dependencies()[0].name().as_str(), "fmt");
    assert_eq!(
        manifest.dependencies()[0].version_req().to_string(),
        "^11.1"
    );

    assert_eq!(manifest.dependencies()[1].name().as_str(), "openssl");
    assert_eq!(manifest.dependencies()[1].version_req().to_string(), "~3.4");

    assert_eq!(manifest.dependencies()[2].name().as_str(), "zlib");
    assert_eq!(
        manifest.dependencies()[2].version_req().to_string(),
        ">=1.2, <2.0"
    );

    // Round-trip serialization preserves sorting
    let serialized = manifest.to_toml_string();
    let parsed_again = Manifest::from_toml_str(&serialized).unwrap();
    assert_eq!(parsed_again, manifest);
}

#[test]
fn test_valid_caret_requirements() {
    let cases = [
        ("^1.0", "1.0.0", true),
        ("^1.0", "1.9.9", true),
        ("^1.0", "2.0.0", false),
        ("^0.2.3", "0.2.3", true),
        ("^0.2.3", "0.2.4", true),
        ("^0.2.3", "0.3.0", false),
    ];

    for (req_str, ver_str, expected_match) in cases {
        let dep = Dependency::try_new("test-pkg", req_str).unwrap();
        let ver = semver::Version::parse(ver_str).unwrap();
        assert_eq!(
            dep.matches(&ver),
            expected_match,
            "failed matching {req_str} against {ver_str}"
        );
    }
}

#[test]
fn test_valid_tilde_requirements() {
    let cases = [
        ("~1.2", "1.2.0", true),
        ("~1.2", "1.2.9", true),
        ("~1.2", "1.3.0", false),
        ("~1.2.3", "1.2.3", true),
        ("~1.2.3", "1.2.9", true),
        ("~1.2.3", "1.3.0", false),
    ];

    for (req_str, ver_str, expected_match) in cases {
        let dep = Dependency::try_new("test-pkg", req_str).unwrap();
        let ver = semver::Version::parse(ver_str).unwrap();
        assert_eq!(
            dep.matches(&ver),
            expected_match,
            "failed matching {req_str} against {ver_str}"
        );
    }
}

#[test]
fn test_valid_comparator_requirements() {
    let cases = [
        (">=1.2, <2.0", "1.2.0", true),
        (">=1.2, <2.0", "1.9.9", true),
        (">=1.2, <2.0", "2.0.0", false),
        (">=1.2, <2.0", "1.1.9", false),
        ("=1.3.1", "1.3.1", true),
        ("=1.3.1", "1.3.2", false),
        ("*", "99.0.0", true),
    ];

    for (req_str, ver_str, expected_match) in cases {
        let dep = Dependency::try_new("test-pkg", req_str).unwrap();
        let ver = semver::Version::parse(ver_str).unwrap();
        assert_eq!(
            dep.matches(&ver),
            expected_match,
            "failed matching {req_str} against {ver_str}"
        );
    }
}

#[test]
fn test_invalid_semantic_version_requirements() {
    let invalid_reqs = [
        "1.0.0.0",
        "invalid-req",
        "^",
        "~",
        ">=",
        "=1.0.0.0",
        "1.0 - 2.0 - 3.0",
    ];

    for req in invalid_reqs {
        let toml_str = format!(
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
fmt = "{req}"
"#
        );
        let res = Manifest::from_toml_str(&toml_str);
        assert!(
            matches!(
                res,
                Err(ManifestError::Validation(
                    ManifestValidationError::InvalidDependencyVersionReq { .. }
                ))
            ),
            "expected validation error for invalid requirement '{req}'"
        );
    }
}

#[test]
fn test_invalid_dependency_names() {
    let invalid_names = [
        "-starts-with-hyphen",
        "has space",
        "has/slash",
        "has@at",
        "con",
        "nul",
        "..",
    ];

    for name in invalid_names {
        let toml_str = format!(
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
"{name}" = "^1.0"
"#
        );
        let res = Manifest::from_toml_str(&toml_str);
        assert!(
            matches!(
                res,
                Err(ManifestError::Validation(
                    ManifestValidationError::InvalidDependencyName { .. }
                ))
            ),
            "expected validation error for invalid dependency name '{name}'"
        );
    }
}

#[test]
fn test_empty_dependency_declaration_rejected() {
    let toml_str = r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
fmt = ""
"#;
    let res = Manifest::from_toml_str(toml_str);
    assert!(matches!(
        res,
        Err(ManifestError::Validation(
            ManifestValidationError::EmptyDependencyVersionReq { .. }
        ))
    ));
}

#[test]
fn test_malformed_dependency_toml_types() {
    // Integer instead of string
    let toml_int = r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
fmt = 11
"#;
    assert!(matches!(
        Manifest::from_toml_str(toml_int),
        Err(ManifestError::Validation(
            ManifestValidationError::InvalidDependencyVersionReq { .. }
        ))
    ));

    // Array instead of string
    let toml_array = r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
fmt = ["^11.0"]
"#;
    assert!(matches!(
        Manifest::from_toml_str(toml_array),
        Err(ManifestError::Validation(
            ManifestValidationError::InvalidDependencyVersionReq { .. }
        ))
    ));
}

#[test]
fn test_cli_check_with_dependencies() {
    let test_dir = TestDir::new("cli_check_deps");
    let manifest_content = r#"
[package]
name = "my-app"
version = "0.1.0"
description = "My C++ application"

[dependencies]
fmt = "^11.1"
zlib = "^1.3"
openssl = "^3.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("Checking Crux project..."));
    assert!(stdout.contains("Manifest: crux.toml"));
    assert!(stdout.contains("Package: my-app"));
    assert!(stdout.contains("Version: 0.1.0"));
    assert!(stdout.contains("Dependencies:"));
    assert!(stdout.contains("fmt"));
    assert!(stdout.contains("^11.1"));
    assert!(stdout.contains("zlib"));
    assert!(stdout.contains("^1.3"));
    assert!(stdout.contains("openssl"));
    assert!(stdout.contains("^3.0"));
    assert!(stdout.contains("Status: Valid"));
}

#[test]
fn test_cli_check_rejects_invalid_dependency_name() {
    let test_dir = TestDir::new("cli_bad_dep_name");
    let manifest_content = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
"-bad-name" = "^1.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("invalid dependency name '-bad-name'"));
}

#[test]
fn test_cli_check_rejects_invalid_dependency_version_req() {
    let test_dir = TestDir::new("cli_bad_dep_req");
    let manifest_content = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = "1.0.0.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("invalid version requirement for dependency 'fmt'"));
}
