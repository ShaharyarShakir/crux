//! Regression tests for production catalog/registry selection.

mod common;

use common::{configure_registry, init_fixture_registry};
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
        let path = std::env::temp_dir().join(format!("crux_catalog_select_{prefix}_{nanos}"));
        fs::create_dir_all(&path).expect("failed to create temporary test directory");
        Self { path }
    }

    fn write_manifest(&self, name: &str) {
        fs::write(
            self.path.join("crux.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
        )
        .unwrap();
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_fresh_project_lock_bootstraps_official_registry_offline() {
    let dir = TestDir::new("fresh_bootstrap");
    dir.write_manifest("fresh-app");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&dir.path)
        .env("CRUX_HOME", dir.path.join("home"))
        .env("CRUX_REGISTRIES_DIR", dir.path.join("registries"))
        .env("CRUX_OFFLINE", "1")
        .env_remove("CRUX_CONFIG_PATH")
        .env_remove("CRUX_REGISTRY_PATH")
        .output()
        .expect("failed to execute crux binary");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Using registry 'default'"));
    assert!(!stdout.contains("in-memory test catalog"));

    let config = fs::read_to_string(dir.path.join(".crux").join("config.toml")).unwrap();
    assert!(config.contains("[registries.default]"));
    assert!(config.contains("subdir = \"registry\""));
    assert!(dir.path.join("crux.lock").is_file());
}

#[test]
fn test_broken_configured_registry_fails_without_in_memory_fallback() {
    let dir = TestDir::new("broken_registry");
    dir.write_manifest("broken-app");
    fs::create_dir_all(dir.path.join(".crux")).unwrap();
    fs::write(
        dir.path.join(".crux").join("config.toml"),
        format!(
            "[registries.default]\npath = \"{}\"\n",
            dir.path.join("missing-registry").display()
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .current_dir(&dir.path)
        .env("CRUX_OFFLINE", "1")
        .env_remove("CRUX_CONFIG_PATH")
        .env_remove("CRUX_REGISTRY_PATH")
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("in-memory test catalog"));
    assert!(
        stderr.contains("registry") || stderr.contains("Git"),
        "unexpected stderr: {stderr}"
    );
    assert!(!dir.path.join("crux.lock").exists());
}

#[test]
fn test_invalid_direct_registry_override_fails_for_lock() {
    let dir = TestDir::new("invalid_override");
    dir.write_manifest("override-app");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&dir.path)
        .env("CRUX_REGISTRY_PATH", dir.path.join("does-not-exist"))
        .env_remove("CRUX_CONFIG_PATH")
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("in-memory test catalog"));
    assert!(
        stderr.contains("registry") || stderr.contains("Git repository"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn test_custom_registry_is_used_by_lock_and_resolve() {
    let dir = TestDir::new("custom_registry");
    fs::write(
        dir.path.join("crux.toml"),
        "[package]\nname = \"custom-app\"\nversion = \"0.1.0\"\n\n[dependencies]\nfmt = \"^11.1\"\n",
    )
    .unwrap();
    let registry = init_fixture_registry(&dir.path.join("fixture-registry"));

    let mut command = Command::new(env!("CARGO_BIN_EXE_crux"));
    configure_registry(
        &mut command,
        &registry,
        &dir.path.join("unused-config.toml"),
    );
    let output = command
        .arg("resolve")
        .current_dir(&dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Using registry 'default'"));
    assert!(stdout.contains("fmt 11.1.1 (registry+default)"));
    assert!(!stdout.contains("in-memory test catalog"));
}
