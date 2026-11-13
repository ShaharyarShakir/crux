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
        let path = std::env::temp_dir().join(format!("crux_integ_check_{prefix}_{nanos}"));
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
fn test_check_valid_manifest() {
    let test_dir = TestDir::new("valid");
    let manifest = r#"
[package]
name = "my-library"
version = "0.2.1"
description = "A C++ utility library"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Checking Crux project..."));
    assert!(stdout.contains("Manifest: crux.toml"));
    assert!(stdout.contains("Package: my-library"));
    assert!(stdout.contains("Version: 0.2.1"));
    assert!(stdout.contains("Status: Valid"));
}

#[test]
fn test_check_valid_manifest_without_description() {
    let test_dir = TestDir::new("no_desc");
    let manifest = r#"
[package]
name = "core-engine"
version = "1.0.0-rc.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Package: core-engine"));
    assert!(stdout.contains("Version: 1.0.0-rc.1"));
    assert!(stdout.contains("Status: Valid"));
}

#[test]
fn test_check_missing_manifest() {
    let test_dir = TestDir::new("missing");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("manifest not found"));
}

#[test]
fn test_check_invalid_toml() {
    let test_dir = TestDir::new("invalid_toml");
    let manifest = "[package\nname = unclosed string";
    fs::write(test_dir.path.join("crux.toml"), manifest).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("failed to parse manifest TOML"));
}

#[test]
fn test_check_missing_package_section() {
    let test_dir = TestDir::new("missing_section");
    let manifest = "name = \"my-lib\"\nversion = \"0.1.0\"\n";
    fs::write(test_dir.path.join("crux.toml"), manifest).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
}

#[test]
fn test_check_missing_package_name() {
    let test_dir = TestDir::new("missing_name");
    let manifest = r#"
[package]
version = "0.1.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("missing field `name`"));
}

#[test]
fn test_check_invalid_package_name() {
    let test_dir = TestDir::new("invalid_name");
    let manifest = r#"
[package]
name = "bad/name"
version = "0.1.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("invalid character '/'"));
}

#[test]
fn test_check_invalid_semantic_version() {
    let test_dir = TestDir::new("invalid_semver");
    let manifest = r#"
[package]
name = "valid-name"
version = "v1.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("invalid semantic version 'v1.0'"));
}

#[test]
fn test_check_rejects_unknown_fields() {
    let test_dir = TestDir::new("unknown_fields");
    let manifest = r#"
[package]
name = "strict-pkg"
version = "1.0.0"
author = "Unknown"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("unknown field `author`"));
}
