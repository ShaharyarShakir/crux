use crux::package::{NameValidationError, PackageId, PackageIdError, PackageMetadata, PackageName};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
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
        let path = std::env::temp_dir().join(format!("crux_integ_package_{prefix}_{nanos}"));
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
fn test_valid_package_identities() {
    let cases = [
        ("zlib", "1.3.1", "zlib 1.3.1"),
        ("fmt", "11.1.1", "fmt 11.1.1"),
        ("openssl", "3.4.0", "openssl 3.4.0"),
        ("boost_system", "1.84.0", "boost_system 1.84.0"),
    ];

    for (name_str, ver_str, expected_display) in cases {
        let pkg_id = PackageId::try_new(name_str, ver_str).unwrap_or_else(|e| {
            panic!("failed to create PackageId for '{name_str} {ver_str}': {e}")
        });

        assert_eq!(pkg_id.name().as_str(), name_str);
        assert_eq!(pkg_id.version().to_string(), ver_str);
        assert_eq!(pkg_id.to_string(), expected_display);

        // Parsing from string format
        let parsed: PackageId = expected_display
            .parse()
            .unwrap_or_else(|e| panic!("failed to parse '{expected_display}': {e}"));
        assert_eq!(parsed, pkg_id);
    }
}

#[test]
fn test_different_versions_of_same_package() {
    let id_v1 = PackageId::try_new("fmt", "10.0.0").unwrap();
    let id_v2 = PackageId::try_new("fmt", "11.1.1").unwrap();

    assert_ne!(id_v1, id_v2);
    assert_eq!(id_v1.name(), id_v2.name());
    assert_ne!(id_v1.version(), id_v2.version());

    assert_eq!(id_v1.cmp_precedence(&id_v2), Ordering::Less);
    assert_eq!(id_v2.cmp_precedence(&id_v1), Ordering::Greater);
    assert_eq!(id_v1.cmp(&id_v2), Ordering::Less);
}

#[test]
fn test_equality_of_identical_package_identities() {
    let id1 = PackageId::try_new("openssl", "3.4.0").unwrap();
    let id2 = PackageId::try_new("openssl", "3.4.0").unwrap();

    assert_eq!(id1, id2);
    assert_eq!(id1.cmp(&id2), Ordering::Equal);
    assert_eq!(id1.cmp_precedence(&id2), Ordering::Equal);

    let mut set = HashSet::new();
    set.insert(id1.clone());
    assert!(set.contains(&id2));
    assert_eq!(set.len(), 1);
}

#[test]
fn test_different_package_names() {
    let id_fmt = PackageId::try_new("fmt", "1.0.0").unwrap();
    let id_zlib = PackageId::try_new("zlib", "1.0.0").unwrap();

    assert_ne!(id_fmt, id_zlib);
    assert_eq!(id_fmt.version(), id_zlib.version());

    // Lexicographical ordering by name when versions are identical
    assert_eq!(id_fmt.cmp(&id_zlib), Ordering::Less);
    assert_eq!(id_zlib.cmp(&id_fmt), Ordering::Greater);
    assert_eq!(id_fmt.cmp_precedence(&id_zlib), Ordering::Less);
}

#[test]
fn test_semantic_version_ordering() {
    let versions = [
        "0.9.0", "1.0.0", "1.0.1", "1.1.0", "1.2.3", "2.0.0", "2.1.0", "10.0.0",
    ];

    let parsed_versions: Vec<semver::Version> = versions
        .iter()
        .map(|v| semver::Version::parse(v).unwrap())
        .collect();

    for i in 0..parsed_versions.len() - 1 {
        let v_curr = &parsed_versions[i];
        let v_next = &parsed_versions[i + 1];

        assert!(
            v_curr < v_next,
            "expected {v_curr} to be less than {v_next}"
        );
        assert_eq!(
            v_curr.cmp_precedence(v_next),
            Ordering::Less,
            "expected cmp_precedence for {v_curr} < {v_next}"
        );
    }
}

#[test]
fn test_prerelease_ordering() {
    let prereleases = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
    ];

    let parsed: Vec<semver::Version> = prereleases
        .iter()
        .map(|v| semver::Version::parse(v).unwrap())
        .collect();

    for i in 0..parsed.len() - 1 {
        let v_curr = &parsed[i];
        let v_next = &parsed[i + 1];

        assert!(
            v_curr < v_next,
            "expected {v_curr} to be less than {v_next}"
        );
        assert_eq!(
            v_curr.cmp_precedence(v_next),
            Ordering::Less,
            "expected cmp_precedence for {v_curr} < {v_next}"
        );
    }
}

#[test]
fn test_build_metadata_behavior() {
    let v_plain = semver::Version::parse("1.0.0").unwrap();
    let v_b1 = semver::Version::parse("1.0.0+build.1").unwrap();
    let v_b2 = semver::Version::parse("1.0.0+build.2").unwrap();
    let v_date = semver::Version::parse("1.0.0+20261113").unwrap();

    // SemVer 2.0.0 Clause 10: Build metadata MUST be ignored when determining version precedence
    assert_eq!(v_b1.cmp_precedence(&v_plain), Ordering::Equal);
    assert_eq!(v_b1.cmp_precedence(&v_b2), Ordering::Equal);
    assert_eq!(v_b2.cmp_precedence(&v_date), Ordering::Equal);
    assert_eq!(v_plain.cmp_precedence(&v_date), Ordering::Equal);

    // However, exact Version equality in Rust does compare all fields
    assert_ne!(v_b1, v_b2);
    assert_ne!(v_b1, v_plain);

    // Pre-release versions with build metadata also obey Clause 10
    let v_pre_b1 = semver::Version::parse("1.0.0-alpha+001").unwrap();
    let v_pre_b2 = semver::Version::parse("1.0.0-alpha+002").unwrap();
    assert_eq!(v_pre_b1.cmp_precedence(&v_pre_b2), Ordering::Equal);

    // PackageId precedence equality
    let pkg_b1 = PackageId::try_new("crux-test", "1.0.0+build.1").unwrap();
    let pkg_b2 = PackageId::try_new("crux-test", "1.0.0+build.2").unwrap();
    assert_eq!(pkg_b1.cmp_precedence(&pkg_b2), Ordering::Equal);
}

#[test]
fn test_optional_metadata_fields() {
    let name = PackageName::new("mylib").unwrap();
    let version = semver::Version::parse("1.0.0").unwrap();

    // 1. Minimal metadata
    let minimal = PackageMetadata::new(name.clone(), version.clone());
    assert_eq!(minimal.name().as_str(), "mylib");
    assert_eq!(minimal.version().to_string(), "1.0.0");
    assert_eq!(minimal.description(), None);
    assert_eq!(minimal.license(), None);
    assert_eq!(minimal.homepage(), None);
    assert_eq!(minimal.repository(), None);

    // 2. Partial metadata
    let with_license = PackageMetadata::new(name.clone(), version.clone()).with_license("MIT");
    assert_eq!(with_license.license(), Some("MIT"));
    assert_eq!(with_license.description(), None);

    // 3. Complete metadata
    let full = PackageMetadata::new(name, version)
        .with_description("A high performance math library")
        .with_license("Apache-2.0 OR MIT")
        .with_homepage("https://example.com/math")
        .with_repository("https://github.com/example/math");

    assert_eq!(full.description(), Some("A high performance math library"));
    assert_eq!(full.license(), Some("Apache-2.0 OR MIT"));
    assert_eq!(full.homepage(), Some("https://example.com/math"));
    assert_eq!(full.repository(), Some("https://github.com/example/math"));

    // 4. TOML serialization roundtrip
    let toml_str = toml::to_string(&full).unwrap();
    let decoded: PackageMetadata = toml::from_str(&toml_str).unwrap();
    assert_eq!(decoded, full);
}

#[test]
fn test_invalid_package_names_and_versions() {
    // Empty name
    assert_eq!(PackageName::new(""), Err(NameValidationError::Empty));

    // Hyphen start
    assert_eq!(
        PackageName::new("-test"),
        Err(NameValidationError::StartsWithHyphen)
    );

    // Invalid characters
    assert_eq!(
        PackageName::new("lib@v1"),
        Err(NameValidationError::InvalidCharacter('@'))
    );
    assert_eq!(
        PackageName::new("lib/name"),
        Err(NameValidationError::InvalidCharacter('/'))
    );
    assert_eq!(
        PackageName::new("lib name"),
        Err(NameValidationError::InvalidCharacter(' '))
    );

    // Reserved names
    assert_eq!(
        PackageName::new("nul"),
        Err(NameValidationError::ReservedName("nul".to_string()))
    );
    assert_eq!(
        PackageName::new("COM1"),
        Err(NameValidationError::ReservedName("COM1".to_string()))
    );

    // Name too long (> 128 characters)
    let too_long = "a".repeat(129);
    assert_eq!(
        PackageName::new(&too_long),
        Err(NameValidationError::TooLong(129))
    );

    // Invalid versions
    assert!(semver::Version::parse("v1.0.0").is_err());
    assert!(semver::Version::parse("1.0").is_err());
    assert!(semver::Version::parse("1.0.0.0").is_err());
    assert!(semver::Version::parse("not-a-version").is_err());
    assert!(semver::Version::parse("").is_err());

    // Invalid PackageId strings
    assert!(matches!(
        PackageId::from_str(""),
        Err(PackageIdError::InvalidFormat(_))
    ));
    assert!(matches!(
        PackageId::from_str("only-name"),
        Err(PackageIdError::InvalidFormat(_))
    ));
    assert!(matches!(
        PackageId::from_str("name 1.0.0 extra"),
        Err(PackageIdError::InvalidFormat(_))
    ));
    assert!(matches!(
        PackageId::from_str("-badname 1.0.0"),
        Err(PackageIdError::InvalidName(_))
    ));
    assert!(matches!(
        PackageId::from_str("goodname badversion"),
        Err(PackageIdError::InvalidVersion(_))
    ));
}

#[test]
fn test_manifest_integration_with_full_metadata_via_cli() {
    let test_dir = TestDir::new("full_meta_cli");
    let manifest_content = r#"
[package]
name = "geometry-engine"
version = "3.2.0-beta.1+build.42"
description = "Computational geometry algorithms"
license = "MIT"
homepage = "https://example.org/geometry"
repository = "https://github.com/example/geometry"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("check")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Manifest: crux.toml"));
    assert!(stdout.contains("Package: geometry-engine"));
    assert!(stdout.contains("Version: 3.2.0-beta.1+build.42"));
    assert!(stdout.contains("Status: Valid"));
}
