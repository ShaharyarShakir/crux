//! M0 characterisation: golden digest for `BuildFingerprint::compute`.
//!
//! The digest below is intentionally fixed. It was produced by hashing the
//! canonical byte stream implemented in `src/build/fingerprint.rs`; the exact
//! inputs are listed in [`fixed_inputs`] and repeated in the M0 report.
//!
//! No installed compiler, CMake binary, generator probe, clock, or environment
//! variable participates in this test: all inputs are constructed literals.

use crux::build::{BuildConfiguration, BuildFingerprint, BuildProfile, ToolchainInfo};
use crux::lock::Checksum;
use crux::package::{PackageId, PackageName};
use std::collections::BTreeMap;

const GOLDEN_FINGERPRINT: &str =
    "sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe";

type FixedInputs = (PackageId, Checksum, ToolchainInfo, BuildConfiguration);

/// Fixed, machine-independent inputs for the golden digest.
///
/// Canonical byte stream fed to SHA-256 by the unchanged implementation:
///
/// ```text
/// char-lib@1.2.3\n
/// 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n
/// debug\n
/// cmake version 3.31.0\n
/// Unix Makefiles\n
/// gcc (GCC) 14.2.0\n
/// g++ (GCC) 14.2.0\n
/// /opt/toolchains/char-gcc/cc\n
/// /opt/toolchains/char-gcc/c++\n
/// shared:1\n
/// -DFIRST=1\0
/// -DSECOND=2\0
/// CRUX_M0_A=alpha\0
/// CRUX_M0_B=beta\0
/// ```
///
/// `cmake_args` are deliberately declared out of order: `compute` sorts them.
/// `extra_env` is a `BTreeMap`, so its iteration order is fixed by key.
/// `jobs` and `configuration.generator` are present for realism but are not
/// hashed by this version of the algorithm.
fn fixed_inputs() -> FixedInputs {
    let package_id = PackageId::new(
        PackageName::new("char-lib").unwrap(),
        semver::Version::parse("1.2.3").unwrap(),
    );
    let checksum =
        Checksum::new_sha256("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .unwrap();
    let toolchain = ToolchainInfo {
        cmake_version: "cmake version 3.31.0".to_string(),
        c_compiler_version: Some("gcc (GCC) 14.2.0".to_string()),
        cxx_compiler_version: Some("g++ (GCC) 14.2.0".to_string()),
        generator: "Unix Makefiles".to_string(),
    };

    let mut extra_env = BTreeMap::new();
    extra_env.insert("CRUX_M0_B".to_string(), "beta".to_string());
    extra_env.insert("CRUX_M0_A".to_string(), "alpha".to_string());

    let config = BuildConfiguration {
        profile: BuildProfile::Debug,
        generator: Some("Unix Makefiles".to_string()),
        cmake_args: vec!["-DSECOND=2".to_string(), "-DFIRST=1".to_string()],
        c_compiler: Some("/opt/toolchains/char-gcc/cc".into()),
        cxx_compiler: Some("/opt/toolchains/char-gcc/c++".into()),
        jobs: Some(8),
        build_shared_libs: Some(true),
        extra_env,
    };

    (package_id, checksum, toolchain, config)
}

#[test]
fn build_fingerprint_matches_golden_digest() {
    let (package_id, checksum, toolchain, config) = fixed_inputs();
    let fingerprint = BuildFingerprint::compute(&package_id, &checksum, &toolchain, &config);

    assert_eq!(
        fingerprint.as_str(),
        GOLDEN_FINGERPRINT,
        "golden digest drifted; do not change the fingerprint algorithm to satisfy this test"
    );
    assert_eq!(
        fingerprint.hex_digest(),
        &GOLDEN_FINGERPRINT["sha256:".len()..]
    );
    assert!(BuildFingerprint::new(fingerprint.as_str()).is_ok());
}

#[test]
fn build_fingerprint_golden_computation_is_repeatable() {
    let (package_id, checksum, toolchain, config) = fixed_inputs();
    let first = BuildFingerprint::compute(&package_id, &checksum, &toolchain, &config);

    for _ in 0..8 {
        assert_eq!(
            BuildFingerprint::compute(&package_id, &checksum, &toolchain, &config),
            first
        );
    }
}
