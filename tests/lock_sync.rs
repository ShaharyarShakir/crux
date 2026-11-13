use crux::lock::Lockfile;
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
        let path = std::env::temp_dir().join(format!("crux_integ_locksync_{prefix}_{nanos}"));
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
fn test_generate_empty_dependency_lockfile() {
    let test_dir = TestDir::new("empty_deps");
    let manifest_content = r#"
[package]
name = "empty-app"
version = "0.1.0"
description = "App with no dependencies"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Generated crux.lock (0 packages locked)"));
    assert!(stdout.contains("Status: Success"));

    let lock_path = test_dir.path.join("crux.lock");
    assert!(lock_path.is_file());
    let lock_content = fs::read_to_string(&lock_path).unwrap();
    assert_eq!(lock_content.trim(), "lockfile_version = 1");
}

#[test]
fn test_generate_direct_and_transitive_dependencies() {
    let test_dir = TestDir::new("direct_transitive");
    // openssl in dev catalog depends on zlib ^1.3
    let manifest_content = r#"
[package]
name = "crypto-app"
version = "1.0.0"

[dependencies]
openssl = "^3.4"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Generated crux.lock (2 packages locked)"));
    assert!(stdout.contains("openssl 3.4.0 (registry+default)"));
    assert!(stdout.contains("zlib 1.3.1 (registry+default)"));

    let lock = Lockfile::load_from_path(&test_dir.path.join("crux.lock")).unwrap();
    assert_eq!(lock.package_count(), 2);
    assert_eq!(lock.packages[0].name.as_str(), "openssl");
    assert_eq!(lock.packages[0].version.to_string(), "3.4.0");
    assert_eq!(
        lock.packages[0].dependencies,
        vec![crux::package::PackageId::try_new("zlib", "1.3.1").unwrap()]
    );
    assert_eq!(lock.packages[1].name.as_str(), "zlib");
    assert_eq!(lock.packages[1].version.to_string(), "1.3.1");
}

#[test]
fn test_lock_preserves_compatible_direct_and_transitive_versions() {
    let test_dir = TestDir::new("preserve_compat");
    let manifest_content = r#"
[package]
name = "network-app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
openssl = ">=3.0, <4.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Create an existing lockfile locking fmt 11.1.0, openssl 3.0.0, and zlib 1.2.11
    // (Dev catalog has fmt 11.1.1, openssl 3.4.0, zlib 1.3.1 available)
    let existing_lock = r#"lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.0"
source = "registry+default"

[[package]]
name = "openssl"
version = "3.0.0"
source = "registry+default"
dependencies = [
    "zlib 1.2.11",
]

[[package]]
name = "zlib"
version = "1.2.11"
source = "registry+default"
"#;
    fs::write(test_dir.path.join("crux.lock"), existing_lock).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Synchronized crux.lock (3 packages locked)"));
    assert!(stdout.contains("fmt 11.1.0 (registry+default)"));
    assert!(stdout.contains("openssl 3.0.0 (registry+default)"));
    assert!(stdout.contains("zlib 1.2.11 (registry+default)"));

    let lock = Lockfile::load_from_path(&test_dir.path.join("crux.lock")).unwrap();
    assert_eq!(lock.packages[0].name.as_str(), "fmt");
    assert_eq!(lock.packages[0].version.to_string(), "11.1.0");
    assert_eq!(lock.packages[1].name.as_str(), "openssl");
    assert_eq!(lock.packages[1].version.to_string(), "3.0.0");
    assert_eq!(lock.packages[2].name.as_str(), "zlib");
    assert_eq!(lock.packages[2].version.to_string(), "1.2.11");
}

#[test]
fn test_lock_update_flag_ignores_preferences_and_upgrades() {
    let test_dir = TestDir::new("update_flag");
    let manifest_content = r#"
[package]
name = "network-app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
openssl = ">=3.0, <4.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let existing_lock = r#"lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.0"
source = "registry+default"

[[package]]
name = "openssl"
version = "3.0.0"
source = "registry+default"
dependencies = [
    "zlib 1.2.11",
]

[[package]]
name = "zlib"
version = "1.2.11"
source = "registry+default"
"#;
    fs::write(test_dir.path.join("crux.lock"), existing_lock).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--update")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Updated crux.lock (3 packages locked)"));
    assert!(stdout.contains("fmt 11.1.1 (registry+default)"));
    assert!(stdout.contains("openssl 3.4.0 (registry+default)"));
    assert!(stdout.contains("zlib 1.3.1 (registry+default)"));

    let lock = Lockfile::load_from_path(&test_dir.path.join("crux.lock")).unwrap();
    assert_eq!(lock.packages[0].version.to_string(), "11.1.1");
    assert_eq!(lock.packages[1].version.to_string(), "3.4.0");
    assert_eq!(lock.packages[2].version.to_string(), "1.3.1");
}

#[test]
fn test_lock_updates_incompatible_locked_version() {
    let test_dir = TestDir::new("incompat_upgrade");
    // Manifest requires fmt ^11.0
    let manifest_content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
fmt = "^11.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Lockfile currently has fmt 10.0.0 (incompatible with ^11.0)
    let existing_lock = r#"lockfile_version = 1

[[package]]
name = "fmt"
version = "10.0.0"
source = "registry+default"
"#;
    fs::write(test_dir.path.join("crux.lock"), existing_lock).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let lock = Lockfile::load_from_path(&test_dir.path.join("crux.lock")).unwrap();
    // fmt 10.0.0 must be upgraded to highest compatible: 11.1.1
    assert_eq!(lock.packages[0].version.to_string(), "11.1.1");
}

#[test]
fn test_lock_resolves_newly_introduced_dependencies() {
    let test_dir = TestDir::new("new_dep");
    let manifest_content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
zlib = "^1.3"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Existing lockfile only had fmt 11.1.0
    let existing_lock = r#"lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.0"
source = "registry+default"
"#;
    fs::write(test_dir.path.join("crux.lock"), existing_lock).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let lock = Lockfile::load_from_path(&test_dir.path.join("crux.lock")).unwrap();
    assert_eq!(lock.package_count(), 2);
    // fmt 11.1.0 preserved
    assert_eq!(lock.packages[0].name.as_str(), "fmt");
    assert_eq!(lock.packages[0].version.to_string(), "11.1.0");
    // new dep zlib resolved to 1.3.1
    assert_eq!(lock.packages[1].name.as_str(), "zlib");
    assert_eq!(lock.packages[1].version.to_string(), "1.3.1");
}

#[test]
fn test_lock_removes_obsolete_dependencies() {
    let test_dir = TestDir::new("remove_obsolete");
    // Manifest requires ONLY fmt
    let manifest_content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Lockfile previously locked fmt AND catch2
    let existing_lock = r#"lockfile_version = 1

[[package]]
name = "catch2"
version = "3.4.0"
source = "registry+default"

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
    fs::write(test_dir.path.join("crux.lock"), existing_lock).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let lock = Lockfile::load_from_path(&test_dir.path.join("crux.lock")).unwrap();
    assert_eq!(lock.package_count(), 1);
    assert_eq!(lock.packages[0].name.as_str(), "fmt");
}

#[test]
fn test_check_synchronized_lockfile_passes() {
    let test_dir = TestDir::new("check_synced");
    let manifest_content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Generate initial lockfile
    let gen_output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();
    assert!(gen_output.status.success());

    let lock_before = fs::read_to_string(test_dir.path.join("crux.lock")).unwrap();

    // Run check
    let check_output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();

    assert!(check_output.status.success());
    let stdout = String::from_utf8(check_output.stdout).unwrap();
    assert!(stdout.contains("Lockfile: crux.lock"));
    assert!(stdout.contains("Format version: 1"));
    assert!(stdout.contains("Packages: 1"));
    assert!(stdout.contains("Status: Valid (synchronized)"));

    // Verify lockfile was not touched
    let lock_after = fs::read_to_string(test_dir.path.join("crux.lock")).unwrap();
    assert_eq!(lock_before, lock_after);
}

#[test]
fn test_check_detects_missing_packages() {
    let test_dir = TestDir::new("check_missing_pkg");
    let manifest_content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Lockfile is empty
    let empty_lock = "lockfile_version = 1\n";
    fs::write(test_dir.path.join("crux.lock"), empty_lock).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("missing package 'fmt'"));
}

#[test]
fn test_check_detects_incompatible_version() {
    let test_dir = TestDir::new("check_incompat_ver");
    let manifest_content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Lockfile has fmt 10.0.0 which violates ^11.1
    let lock_content = r#"lockfile_version = 1

[[package]]
name = "fmt"
version = "10.0.0"
source = "registry+default"
"#;
    fs::write(test_dir.path.join("crux.lock"), lock_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("does not satisfy manifest requirement"));
}

#[test]
fn test_check_detects_obsolete_entries() {
    let test_dir = TestDir::new("check_obsolete_pkg");
    let manifest_content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Lockfile has fmt AND catch2, but catch2 is not in crux.toml
    let lock_content = r#"lockfile_version = 1

[[package]]
name = "catch2"
version = "3.4.0"
source = "registry+default"

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
    fs::write(test_dir.path.join("crux.lock"), lock_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("obsolete package 'catch2 3.4.0'"));
}

#[test]
fn test_check_detects_invalid_dependency_relationship() {
    let test_dir = TestDir::new("check_broken_rel");
    let manifest_content = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
openssl = "^3.4"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // Lockfile lists openssl depending on zlib 1.3.1, but zlib is not in the lockfile
    let lock_content = r#"lockfile_version = 1

[[package]]
name = "openssl"
version = "3.4.0"
source = "registry+default"
dependencies = [
    "zlib 1.3.1",
]
"#;
    fs::write(test_dir.path.join("crux.lock"), lock_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("not present in the lockfile"));
}

#[test]
fn test_determinism_repeated_lock_commands_idempotent() {
    let test_dir = TestDir::new("idempotent");
    let manifest_content = r#"
[package]
name = "crypto-app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
openssl = "^3.4"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    // 1st run
    let out1 = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();
    assert!(out1.status.success());
    let lock_content1 = fs::read_to_string(test_dir.path.join("crux.lock")).unwrap();

    // 2nd run
    let out2 = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();
    assert!(out2.status.success());
    let lock_content2 = fs::read_to_string(test_dir.path.join("crux.lock")).unwrap();

    // 3rd run with --update
    let out3 = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--update")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();
    assert!(out3.status.success());
    let lock_content3 = fs::read_to_string(test_dir.path.join("crux.lock")).unwrap();

    assert_eq!(lock_content1, lock_content2);
    assert_eq!(lock_content2, lock_content3);
}

#[test]
fn test_filesystem_safety_resolution_failure_preserves_existing_lockfile() {
    let test_dir = TestDir::new("safety_preserve");
    let manifest_content_good = r#"
[package]
name = "safe-app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content_good).unwrap();

    // Generate valid lockfile
    let out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let original_lock = fs::read_to_string(test_dir.path.join("crux.lock")).unwrap();

    // Now introduce an unresolvable dependency in crux.toml
    let manifest_content_bad = r#"
[package]
name = "safe-app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
ghost-package = "^1.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content_bad).unwrap();

    let out_fail = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .unwrap();
    assert!(!out_fail.status.success());

    // Verify existing crux.lock was NOT modified or destroyed
    let after_lock = fs::read_to_string(test_dir.path.join("crux.lock")).unwrap();
    assert_eq!(original_lock, after_lock);

    // Verify no temporary files remain
    let entries = fs::read_dir(&test_dir.path).unwrap();
    for entry in entries {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(!name.starts_with(".crux.lock.tmp"));
    }
}

#[test]
fn test_resolve_preview_never_writes_lockfile() {
    let test_dir = TestDir::new("preview_no_write");
    let manifest_content = r#"
[package]
name = "preview-app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Resolved 1 dependencies:"));
    assert!(stdout.contains("fmt 11.1.1 (registry+default)"));

    // Ensure crux.lock was NOT created
    assert!(!test_dir.path.join("crux.lock").exists());
}
