//! Offline-safe consistency tests for the M22 official package catalog.

use crux::package::{PackageId, PackageName};
use crux::registry::{
    CURRENT_REGISTRY_FORMAT_VERSION, GitRegistry, PackageSourceDefinition, RegistryError,
    init_test_git_registry,
};
use crux::resolution::catalog::PackageCatalog;
use crux::resolver::Resolver;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const SUPPORTED_PACKAGES: &[(&str, &str)] = &[
    ("benchmark", "1.10.0"),
    ("cJSON", "1.7.19"),
    ("CLI11", "2.7.2"),
    ("catch2", "3.8.0"),
    ("cxxopts", "3.3.1"),
    ("doctest", "2.5.3"),
    ("fmt", "11.1.1"),
    ("googletest", "1.18.0"),
    ("nlohmann-json", "3.12.0"),
    ("pcre2", "10.49.0"),
    ("spdlog", "1.17.0"),
    ("tomlplusplus", "3.4.0"),
    ("yaml-cpp", "0.9.0"),
    ("yyjson", "0.13.0"),
    ("zlib", "1.3.1"),
];

const ALL_PACKAGE_NAMES: &[&str] = &[
    "CLI11",
    "OpenSSL",
    "benchmark",
    "bzip2",
    "c-ares",
    "cJSON",
    "catch2",
    "cxxopts",
    "doctest",
    "elfutils",
    "fmt",
    "googletest",
    "libarchive",
    "libcap",
    "libcurl",
    "libdrm",
    "libelf",
    "libevdev",
    "libevent",
    "libgpiod",
    "libi2c",
    "libinput",
    "libseccomp",
    "libserialport",
    "libssh2",
    "libudev",
    "liburing",
    "libusb",
    "libuv",
    "libxml2",
    "lmdb",
    "lz4",
    "mbedTLS",
    "musl",
    "nghttp2",
    "nlohmann-json",
    "paho-mqtt",
    "pcre2",
    "protobuf",
    "spdlog",
    "sqlite",
    "systemd",
    "tomlplusplus",
    "xxhash",
    "xz",
    "yaml-cpp",
    "yyjson",
    "zeromq",
    "zlib",
    "zstd",
];

fn official_registry_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("registry")
}

fn open_official_registry() -> GitRegistry {
    GitRegistry::open(official_registry_root()).expect("official registry must load")
}

fn temp_test_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("crux_m22_{name}_{}_{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("failed to create temp test dir");
    dir
}

#[test]
fn official_registry_contains_all_fifty_catalog_packages() {
    let registry = open_official_registry();

    assert_eq!(
        registry.manifest().format_version,
        CURRENT_REGISTRY_FORMAT_VERSION
    );
    assert_eq!(registry.package_count(), 50);

    let actual: BTreeSet<String> = registry
        .package_names()
        .into_iter()
        .map(|name| name.as_str().to_string())
        .collect();
    let expected: BTreeSet<String> = ALL_PACKAGE_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    assert_eq!(actual, expected);

    // Enumeration must be byte-for-byte repeatable.
    assert_eq!(registry.package_names(), registry.package_names());
}

#[test]
fn every_official_definition_has_concrete_archive_source() {
    let registry = open_official_registry();

    for name in registry.package_names() {
        for version in registry.get_package_versions(&name).unwrap() {
            let id = PackageId::new(name.clone(), version.version.clone());
            let definition = registry
                .get_package_definition(&id)
                .unwrap_or_else(|| panic!("missing definition for {id}"));

            assert!(
                definition
                    .metadata
                    .description
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty()),
                "{id} must have a description"
            );

            match &definition.source {
                PackageSourceDefinition::Archive { url, checksum } => {
                    assert!(
                        url.starts_with("https://") || url.starts_with("http://"),
                        "{id} has non-HTTP archive URL: {url}"
                    );
                    let digest = checksum.digest();
                    assert_eq!(digest.len(), 64, "{id} has invalid checksum length");
                    assert!(
                        digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
                        "{id} has non-hex checksum"
                    );
                    assert!(
                        digest.bytes().all(|byte| !byte.is_ascii_uppercase()),
                        "{id} checksum must be normalized to lowercase"
                    );
                }
                PackageSourceDefinition::Git { .. } => {
                    panic!("{id} uses a git source, which Crux cannot acquire yet")
                }
            }

            let system = definition
                .build
                .as_ref()
                .map(|build| build.system.as_str())
                .unwrap_or_default();
            assert!(
                system == "cmake" || system == "catalog-only",
                "{id} has unexpected build system '{system}'"
            );
        }
    }
}

#[test]
fn supported_tier_is_explicit_and_resolvable_offline() {
    let registry = open_official_registry();
    let mut dependencies = Vec::new();

    for (name, version) in SUPPORTED_PACKAGES {
        let name = PackageName::new(*name).unwrap();
        let id = PackageId::new(name.clone(), version.parse().unwrap());
        let definition = registry
            .get_package_definition(&id)
            .unwrap_or_else(|| panic!("missing supported definition for {id}"));
        assert_eq!(
            definition.build.as_ref().map(|build| build.system.as_str()),
            Some("cmake"),
            "{id} must declare the supported CMake backend"
        );
        dependencies.push(crux::dependency::Dependency::try_new(name.as_str(), "*").unwrap());
    }

    assert_eq!(SUPPORTED_PACKAGES.len(), 15);
    let graph = Resolver::new(&registry)
        .resolve(None, &dependencies)
        .expect("supported tier must resolve without network access");
    assert_eq!(graph.package_count(), 15);
}

#[test]
fn archived_package_without_checksum_is_rejected() {
    let dir = temp_test_dir("missing_checksum");
    init_test_git_registry(&dir).unwrap();

    let path = dir
        .join("packages")
        .join("fmt")
        .join("11.1.1")
        .join("package.toml");
    let content = fs::read_to_string(&path).unwrap();
    fs::write(path, content.replace("checksum = ", "old_checksum = ")).unwrap();

    let result = GitRegistry::open(&dir);
    assert!(
        result.is_err(),
        "archive definitions without a checksum must be rejected"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn invalid_dependency_requirement_is_rejected() {
    let dir = temp_test_dir("bad_dependency");
    init_test_git_registry(&dir).unwrap();

    let path = dir
        .join("packages")
        .join("fmt")
        .join("11.1.1")
        .join("package.toml");
    fs::write(
        path,
        r#"[package]
name = "fmt"
version = "11.1.1"

[source]
type = "archive"
url = "https://example.com/fmt.zip"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"

[build]
system = "cmake"

[dependencies]
zlib = "definitely-not-semver"
"#,
    )
    .unwrap();

    let result = GitRegistry::open(&dir);
    assert!(matches!(result, Err(RegistryError::Validation(_))));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn invalid_checksum_algorithm_is_rejected() {
    let dir = temp_test_dir("bad_checksum");
    init_test_git_registry(&dir).unwrap();

    let path = dir
        .join("packages")
        .join("fmt")
        .join("11.1.1")
        .join("package.toml");
    let content = fs::read_to_string(&path).unwrap();
    fs::write(path, content.replace("sha256:a251", "md5:a251")).unwrap();

    let result = GitRegistry::open(&dir);
    assert!(matches!(result, Err(RegistryError::Validation(_))));

    let _ = fs::remove_dir_all(&dir);
}
