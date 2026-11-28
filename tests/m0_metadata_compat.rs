//! M0 characterisation: build/install metadata deserialization compatibility.
//!
//! Fixtures are literal TOML documents based on the existing
//! `BuildMetadata` / `InstallMetadata` structs. No production serialization
//! format is changed and no migration is performed here.

use crux::build::{ArtifactType, BuildMetadata, BuildProfile};
use crux::install::InstallMetadata;
use std::path::PathBuf;

const FINGERPRINT: &str = "sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe";
const SOURCE_CHECKSUM: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Current `metadata.toml` shape written by `BuildMetadata`.
const CURRENT_BUILD_FIXTURE: &str = r#"
format_version = 1
package_name = "char-lib"
package_version = "1.2.3"
source_checksum = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
fingerprint = "sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe"
profile = "debug"
source_dir = "/fixtures/char-lib/src"
build_dir = "/fixtures/char-lib/build"
configure_exit_code = 0
build_exit_code = 0
duration_millis = 1234
success = true

[[artifacts]]
name = "libchar_lib.a"
relative_path = "libchar_lib.a"
path = "/fixtures/char-lib/build/libchar_lib.a"
artifact_type = "StaticLibrary"
size_bytes = 64

[toolchain]
cmake_version = "cmake version 3.31.0"
c_compiler_version = "gcc (GCC) 14.2.0"
cxx_compiler_version = "g++ (GCC) 14.2.0"
generator = "Unix Makefiles"

[configuration]
profile = "debug"
generator = "Unix Makefiles"
cmake_args = ["-DFIRST=1", "-DSECOND=2"]
c_compiler = "/opt/toolchains/char-gcc/cc"
cxx_compiler = "/opt/toolchains/char-gcc/c++"
jobs = 8
build_shared_libs = true

[configuration.extra_env]
CRUX_M0_A = "alpha"
CRUX_M0_B = "beta"
"#;

/// Older `metadata.toml`: every `#[serde(default)]` field in
/// `BuildConfiguration` is absent, while required fields remain present.
const OLDER_BUILD_FIXTURE: &str = r#"
format_version = 1
package_name = "char-lib"
package_version = "1.2.3"
source_checksum = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
fingerprint = "sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe"
profile = "release"
source_dir = "/fixtures/char-lib/src"
build_dir = "/fixtures/char-lib/build"
configure_exit_code = 0
build_exit_code = 0
duration_millis = 900
success = true

[[artifacts]]
name = "libchar_lib.a"
relative_path = "libchar_lib.a"
path = "/fixtures/char-lib/build/libchar_lib.a"
artifact_type = "StaticLibrary"
size_bytes = 64

[toolchain]
cmake_version = "cmake version 3.31.0"
c_compiler_version = "gcc (GCC) 14.2.0"
cxx_compiler_version = "g++ (GCC) 14.2.0"
generator = "Unix Makefiles"

[configuration]
profile = "release"
"#;

/// Current installed.toml without the optional `installed_at` field.
const INSTALL_FIXTURE: &str = r#"
format_version = 1
package_name = "char-lib"
package_version = "1.2.3"
fingerprint = "sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe"
profile = "debug"
total_size_bytes = 20
success = true

[[installed_files]]
relative_path = "include/char.h"
file_type = "header"
size_bytes = 20
"#;

#[test]
fn current_build_metadata_fixture_deserializes() {
    let metadata: BuildMetadata = toml::from_str(CURRENT_BUILD_FIXTURE).unwrap();

    assert_eq!(metadata.format_version, 1);
    assert_eq!(metadata.package_name, "char-lib");
    assert_eq!(metadata.package_version, "1.2.3");
    assert_eq!(metadata.source_checksum, SOURCE_CHECKSUM);
    assert_eq!(metadata.fingerprint.as_str(), FINGERPRINT);
    assert_eq!(metadata.profile, BuildProfile::Debug);
    assert_eq!(metadata.source_dir, PathBuf::from("/fixtures/char-lib/src"));
    assert_eq!(
        metadata.build_dir,
        PathBuf::from("/fixtures/char-lib/build")
    );
    assert_eq!(metadata.configure_exit_code, 0);
    assert_eq!(metadata.build_exit_code, 0);
    assert_eq!(metadata.duration_millis, 1234);
    assert!(metadata.success);

    assert_eq!(metadata.toolchain.generator, "Unix Makefiles");
    assert_eq!(metadata.toolchain.cmake_version, "cmake version 3.31.0");

    assert_eq!(metadata.configuration.profile, BuildProfile::Debug);
    assert_eq!(
        metadata.configuration.generator.as_deref(),
        Some("Unix Makefiles")
    );
    assert_eq!(
        metadata.configuration.cmake_args,
        vec!["-DFIRST=1", "-DSECOND=2"]
    );
    assert_eq!(
        metadata.configuration.c_compiler,
        Some(PathBuf::from("/opt/toolchains/char-gcc/cc"))
    );
    assert_eq!(
        metadata.configuration.cxx_compiler,
        Some(PathBuf::from("/opt/toolchains/char-gcc/c++"))
    );
    assert_eq!(metadata.configuration.jobs, Some(8));
    assert_eq!(metadata.configuration.build_shared_libs, Some(true));
    assert_eq!(
        metadata.configuration.extra_env.get("CRUX_M0_A"),
        Some(&"alpha".to_string())
    );
    assert_eq!(
        metadata.configuration.extra_env.get("CRUX_M0_B"),
        Some(&"beta".to_string())
    );

    assert_eq!(metadata.artifacts.len(), 1);
    assert_eq!(metadata.artifacts[0].name, "libchar_lib.a");
    assert_eq!(
        metadata.artifacts[0].artifact_type,
        ArtifactType::StaticLibrary
    );
    assert_eq!(metadata.artifacts[0].size_bytes, 64);
}

#[test]
fn older_build_metadata_uses_existing_serde_defaults() {
    let metadata: BuildMetadata = toml::from_str(OLDER_BUILD_FIXTURE).unwrap();

    assert_eq!(metadata.format_version, 1);
    assert_eq!(metadata.profile, BuildProfile::Release);
    assert_eq!(metadata.configuration.profile, BuildProfile::Release);

    assert_eq!(metadata.configuration.generator, None);
    assert!(metadata.configuration.cmake_args.is_empty());
    assert_eq!(metadata.configuration.c_compiler, None);
    assert_eq!(metadata.configuration.cxx_compiler, None);
    assert_eq!(metadata.configuration.jobs, None);
    assert_eq!(metadata.configuration.build_shared_libs, None);
    assert!(metadata.configuration.extra_env.is_empty());

    assert_eq!(metadata.artifacts.len(), 1);
    assert!(metadata.success);
}

#[test]
fn install_metadata_without_optional_installed_at_deserializes() {
    let metadata: InstallMetadata = toml::from_str(INSTALL_FIXTURE).unwrap();

    assert_eq!(metadata.format_version, 1);
    assert_eq!(metadata.package_name, "char-lib");
    assert_eq!(metadata.package_version, "1.2.3");
    assert_eq!(metadata.fingerprint.as_str(), FINGERPRINT);
    assert_eq!(metadata.profile, BuildProfile::Debug);
    assert_eq!(metadata.installed_files.len(), 1);
    assert_eq!(metadata.total_size_bytes, 20);
    assert!(metadata.success);
    assert_eq!(metadata.installed_at, None);
}

fn assert_rejected(fixture: &str, label: &str) {
    let result = toml::from_str::<BuildMetadata>(fixture);
    assert!(
        result.is_err(),
        "{label} must be rejected without panicking"
    );
}

#[test]
fn invalid_build_metadata_is_rejected() {
    let missing_fingerprint = CURRENT_BUILD_FIXTURE.replace(
        "fingerprint = \"sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe\"\n",
        "",
    );
    assert_rejected(&missing_fingerprint, "missing fingerprint");

    let invalid_profile =
        CURRENT_BUILD_FIXTURE.replace("profile = \"debug\"", "profile = \"nightly\"");
    assert_rejected(&invalid_profile, "unknown build profile");

    let invalid_artifact = CURRENT_BUILD_FIXTURE.replace(
        "artifact_type = \"StaticLibrary\"",
        "artifact_type = \"Banana\"",
    );
    assert_rejected(&invalid_artifact, "unknown artifact type");

    let wrong_type =
        CURRENT_BUILD_FIXTURE.replace("format_version = 1", "format_version = \"one\"");
    assert_rejected(&wrong_type, "non-numeric format version");
}

#[test]
fn characterise_unvalidated_build_metadata_fields() {
    // Characterisation, not an endorsement: `BuildMetadata` uses derived
    // `Deserialize`, so neither the format version nor the fingerprint string
    // passes through the validating `BuildFingerprint::new` constructor here.
    let future_fixture =
        CURRENT_BUILD_FIXTURE.replace("format_version = 1", "format_version = 999");
    let future: BuildMetadata = toml::from_str(&future_fixture).unwrap();
    assert_eq!(future.format_version, 999);

    let malformed_fingerprint = CURRENT_BUILD_FIXTURE.replace(
        "sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe",
        "sha256:not-hex",
    );
    let malformed: BuildMetadata = toml::from_str(&malformed_fingerprint).unwrap();
    assert_eq!(malformed.fingerprint.as_str(), "sha256:not-hex");
}
