use crux::lock::{
    Checksum, ChecksumAlgorithm, LockedPackage, Lockfile, LockfileError, LockfileValidationError,
    PackageSource,
};

use crux::package::{PackageId, PackageName};
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
        let path = std::env::temp_dir().join(format!("crux_integ_lock_{prefix}_{nanos}"));
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
fn test_lockfile_empty_roundtrip() {
    let lockfile = Lockfile::new();
    assert_eq!(lockfile.lockfile_version, 1);
    assert_eq!(lockfile.package_count(), 0);

    let serialized = lockfile.to_toml_string().unwrap();
    assert!(serialized.contains("lockfile_version = 1"));

    let deserialized = Lockfile::from_toml_str(&serialized).unwrap();
    assert_eq!(deserialized, lockfile);
}

#[test]
fn test_lockfile_multiple_packages_and_versions() {
    let mut lockfile = Lockfile::new();

    let pkg_fmt = LockedPackage::new(
        PackageName::new("fmt").unwrap(),
        semver::Version::parse("11.1.1").unwrap(),
        PackageSource::default_registry(),
    )
    .with_checksum(
        Checksum::parse("sha256:d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620")
            .unwrap(),
    );

    let pkg_zlib_old = LockedPackage::new(
        PackageName::new("zlib").unwrap(),
        semver::Version::parse("1.2.11").unwrap(),
        PackageSource::default_registry(),
    );

    let pkg_zlib_new = LockedPackage::new(
        PackageName::new("zlib").unwrap(),
        semver::Version::parse("1.3.1").unwrap(),
        PackageSource::default_registry(),
    )
    .with_checksum(
        Checksum::parse("sha256:9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23")
            .unwrap(),
    );

    lockfile.add_package(pkg_fmt).unwrap();
    lockfile.add_package(pkg_zlib_old).unwrap();
    lockfile.add_package(pkg_zlib_new).unwrap();

    assert_eq!(lockfile.package_count(), 3);

    let serialized = lockfile.to_toml_string().unwrap();
    let deserialized = Lockfile::from_toml_str(&serialized).unwrap();
    assert_eq!(deserialized, lockfile);

    // Verify ordering: fmt (11.1.1), zlib (1.2.11), zlib (1.3.1)
    assert_eq!(deserialized.packages[0].name.as_str(), "fmt");
    assert_eq!(deserialized.packages[1].name.as_str(), "zlib");
    assert_eq!(deserialized.packages[1].version.to_string(), "1.2.11");
    assert_eq!(deserialized.packages[2].name.as_str(), "zlib");
    assert_eq!(deserialized.packages[2].version.to_string(), "1.3.1");
}

#[test]
fn test_lockfile_deterministic_serialization() {
    let pkg1 = LockedPackage::new(
        PackageName::new("zlib").unwrap(),
        semver::Version::parse("1.3.1").unwrap(),
        PackageSource::default_registry(),
    );
    let pkg2 = LockedPackage::new(
        PackageName::new("fmt").unwrap(),
        semver::Version::parse("11.1.1").unwrap(),
        PackageSource::default_registry(),
    );
    let pkg3 = LockedPackage::new(
        PackageName::new("openssl").unwrap(),
        semver::Version::parse("3.4.0").unwrap(),
        PackageSource::default_registry(),
    );

    // Lockfile A added in order 1, 2, 3
    let mut lf_a = Lockfile::new();
    lf_a.add_package(pkg1.clone()).unwrap();
    lf_a.add_package(pkg2.clone()).unwrap();
    lf_a.add_package(pkg3.clone()).unwrap();

    // Lockfile B added in order 3, 1, 2
    let mut lf_b = Lockfile::new();
    lf_b.add_package(pkg3).unwrap();
    lf_b.add_package(pkg1).unwrap();
    lf_b.add_package(pkg2).unwrap();

    let toml_a = lf_a.to_toml_string().unwrap();
    let toml_b = lf_b.to_toml_string().unwrap();

    // Must produce 100% bit-identical string outputs
    assert_eq!(toml_a, toml_b);

    // Multiple invocations of to_toml_string on same instance are idempotent
    let toml_a_again = lf_a.to_toml_string().unwrap();
    assert_eq!(toml_a, toml_a_again);
}

#[test]
fn test_lockfile_rejects_duplicate_package_identities() {
    let toml_content = r#"
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
    let res = Lockfile::from_toml_str(toml_content);
    assert!(matches!(
        res,
        Err(LockfileError::Validation(
            LockfileValidationError::DuplicatePackage { .. }
        ))
    ));
}

#[test]
fn test_lockfile_rejects_unsupported_version() {
    let toml_content = r#"
lockfile_version = 99

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
    let res = Lockfile::from_toml_str(toml_content);
    assert!(matches!(
        res,
        Err(LockfileError::Validation(
            LockfileValidationError::UnsupportedVersion {
                found: 99,
                supported: 1
            }
        ))
    ));
}

#[test]
fn test_lockfile_rejects_missing_required_fields() {
    // Missing lockfile_version
    let no_version = r#"
[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
"#;
    assert!(matches!(
        Lockfile::from_toml_str(no_version),
        Err(LockfileError::Toml(_))
    ));

    // Missing package name
    let no_name = r#"
lockfile_version = 1

[[package]]
version = "11.1.1"
source = "registry+default"
"#;
    assert!(matches!(
        Lockfile::from_toml_str(no_name),
        Err(LockfileError::Toml(_))
    ));

    // Missing package version
    let no_pkg_version = r#"
lockfile_version = 1

[[package]]
name = "fmt"
source = "registry+default"
"#;
    assert!(matches!(
        Lockfile::from_toml_str(no_pkg_version),
        Err(LockfileError::Toml(_))
    ));

    // Missing package source
    let no_source = r#"
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
"#;
    assert!(matches!(
        Lockfile::from_toml_str(no_source),
        Err(LockfileError::Toml(_))
    ));
}

#[test]
fn test_lockfile_optional_checksum_behavior() {
    // 1. Without checksum
    let no_cs_toml = r#"
lockfile_version = 1

[[package]]
name = "local-math"
version = "0.1.0"
source = "path+../local-math"
"#;
    let lf_no_cs = Lockfile::from_toml_str(no_cs_toml).unwrap();
    assert_eq!(lf_no_cs.packages[0].checksum, None);

    // 2. With valid checksum
    let with_cs_toml = r#"
lockfile_version = 1

[[package]]
name = "local-math"
version = "0.1.0"
source = "registry+default"
checksum = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    let lf_with_cs = Lockfile::from_toml_str(with_cs_toml).unwrap();
    assert!(lf_with_cs.packages[0].checksum.is_some());
    assert_eq!(
        lf_with_cs.packages[0]
            .checksum
            .as_ref()
            .unwrap()
            .algorithm(),
        ChecksumAlgorithm::Sha256
    );

    // 3. With invalid checksum format
    let bad_cs_toml = r#"
lockfile_version = 1

[[package]]
name = "local-math"
version = "0.1.0"
source = "registry+default"
checksum = "not-a-valid-checksum"
"#;
    assert!(matches!(
        Lockfile::from_toml_str(bad_cs_toml),
        Err(LockfileError::Toml(_))
    ));
}

#[test]
fn test_lockfile_with_dependencies() {
    let toml_content = r#"
lockfile_version = 1

[[package]]
name = "app"
version = "1.0.0"
source = "registry+default"
dependencies = [
    "fmt 11.1.1",
    "zlib 1.3.1",
]

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"

[[package]]
name = "zlib"
version = "1.3.1"
source = "registry+default"
"#;
    let lf = Lockfile::from_toml_str(toml_content).unwrap();
    assert_eq!(lf.package_count(), 3);

    let app_pkg = &lf.packages[0];
    assert_eq!(app_pkg.name.as_str(), "app");
    assert_eq!(app_pkg.dependencies.len(), 2);
    assert_eq!(
        app_pkg.dependencies[0],
        PackageId::try_new("fmt", "11.1.1").unwrap()
    );
    assert_eq!(
        app_pkg.dependencies[1],
        PackageId::try_new("zlib", "1.3.1").unwrap()
    );

    let reserialized = lf.to_toml_string().unwrap();
    assert!(reserialized.contains("\"fmt 11.1.1\""));
    assert!(reserialized.contains("\"zlib 1.3.1\""));
}

#[test]
fn test_cli_lock_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--help")
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("crux-lock — Inspect and validate the project lockfile"));
    assert!(stdout.contains("--check"));
}

#[test]
fn test_cli_lock_no_args_generates_lockfile() {
    let test_dir = TestDir::new("cli_lock_gen");
    let manifest_content = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Generated crux.lock"));
    assert!(stdout.contains("fmt 11.1.1 (registry+default)"));
    assert!(test_dir.path.join("crux.lock").is_file());
}

#[test]
fn test_cli_lock_missing_manifest_fails() {
    let test_dir = TestDir::new("cli_lock_missing_manifest");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("manifest not found at"));
}

#[test]
fn test_cli_lock_check_valid_lockfile() {
    let test_dir = TestDir::new("cli_valid");
    let lock_content = r#"
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
checksum = "sha256:d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620"

[[package]]
name = "zlib"
version = "1.3.1"
source = "registry+default"
"#;
    let lock_path = test_dir.path.join("crux.lock");
    fs::write(&lock_path, lock_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Checking Crux lockfile..."));
    assert!(stdout.contains("Lockfile: crux.lock"));
    assert!(stdout.contains("Format version: 1"));
    assert!(stdout.contains("Packages: 2"));
    assert!(stdout.contains("Status: Valid"));

    // Verify lockfile was not modified during check
    let after_content = fs::read_to_string(&lock_path).unwrap();
    assert_eq!(after_content, lock_content);
}

#[test]
fn test_cli_lock_check_empty_lockfile() {
    let test_dir = TestDir::new("cli_empty");
    let lock_content = "lockfile_version = 1\n";
    fs::write(test_dir.path.join("crux.lock"), lock_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Format version: 1"));
    assert!(stdout.contains("Packages: 0"));
    assert!(stdout.contains("Status: Valid"));
}

#[test]
fn test_cli_lock_check_missing_lockfile() {
    let test_dir = TestDir::new("cli_missing");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("lockfile not found at"));
}

#[test]
fn test_cli_lock_check_invalid_toml() {
    let test_dir = TestDir::new("cli_invalid_toml");
    fs::write(
        test_dir.path.join("crux.lock"),
        "not a valid toml file :::::",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("failed to parse lockfile TOML"));
}

#[test]
fn test_cli_lock_check_unsupported_version() {
    let test_dir = TestDir::new("cli_unsupported_ver");
    fs::write(test_dir.path.join("crux.lock"), "lockfile_version = 42\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Invalid"));
    assert!(stderr.contains("unsupported lockfile version '42'"));
}

#[test]
fn test_cli_lock_unexpected_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("lock")
        .arg("--unknown-flag")
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unexpected argument '--unknown-flag'"));
}
