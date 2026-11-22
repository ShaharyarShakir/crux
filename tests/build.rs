//! Integration tests for the Crux CMake build engine.

use crux::build::{
    ArtifactType, BuildConfiguration, BuildEngine, BuildError, BuildFingerprint, BuildProfile,
    BuildRequest, CMakeBuildEngine, ToolchainInfo,
};
use crux::lock::Checksum;
use crux::package::{PackageId, PackageName};
use std::fs;
use std::path::{Path, PathBuf};

/// RAII helper to clean up test directories on drop.
struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let temp_dir = std::env::temp_dir().join(format!(
            "crux_test_build_{}_{}_{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        Self { path: temp_dir }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn dummy_checksum() -> Checksum {
    Checksum::new_sha256("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        .unwrap()
}

fn dummy_package_id(name: &str, version: &str) -> PackageId {
    PackageId::new(
        PackageName::new(name).unwrap(),
        semver::Version::parse(version).unwrap(),
    )
}

fn dummy_toolchain() -> ToolchainInfo {
    ToolchainInfo {
        cmake_version: "cmake version 3.31.0".to_string(),
        c_compiler_version: Some("gcc (GCC) 14.2.0".to_string()),
        cxx_compiler_version: Some("g++ (GCC) 14.2.0".to_string()),
        generator: "Unix Makefiles".to_string(),
    }
}

#[test]
fn test_build_profile_display_and_parsing() {
    assert_eq!(BuildProfile::Debug.as_cmake_build_type(), "Debug");
    assert_eq!(BuildProfile::Release.as_cmake_build_type(), "Release");

    assert_eq!(BuildProfile::Debug.as_str(), "debug");
    assert_eq!(BuildProfile::Release.as_str(), "release");

    assert_eq!(
        "debug".parse::<BuildProfile>().unwrap(),
        BuildProfile::Debug
    );
    assert_eq!(
        "DEBUG".parse::<BuildProfile>().unwrap(),
        BuildProfile::Debug
    );
    assert_eq!(
        "release".parse::<BuildProfile>().unwrap(),
        BuildProfile::Release
    );
    assert_eq!(
        "Release".parse::<BuildProfile>().unwrap(),
        BuildProfile::Release
    );

    assert!("invalid".parse::<BuildProfile>().is_err());
}

#[test]
fn test_artifact_type_classification() {
    assert_eq!(
        ArtifactType::from_path(Path::new("libfoo.a")),
        ArtifactType::StaticLibrary
    );
    assert_eq!(
        ArtifactType::from_path(Path::new("foo.lib")),
        ArtifactType::StaticLibrary
    );
    assert_eq!(
        ArtifactType::from_path(Path::new("libfoo.so")),
        ArtifactType::SharedLibrary
    );
    assert_eq!(
        ArtifactType::from_path(Path::new("libfoo.so.1.2")),
        ArtifactType::SharedLibrary
    );
    assert_eq!(
        ArtifactType::from_path(Path::new("libfoo.dylib")),
        ArtifactType::SharedLibrary
    );
    assert_eq!(
        ArtifactType::from_path(Path::new("header.hpp")),
        ArtifactType::HeaderOnly
    );
    assert_eq!(
        ArtifactType::from_path(Path::new("header.h")),
        ArtifactType::HeaderOnly
    );
}

#[test]
fn test_build_fingerprint_determinism_and_divergence() {
    let pkg = dummy_package_id("test-lib", "1.0.0");
    let checksum = dummy_checksum();
    let toolchain = dummy_toolchain();
    let config = BuildConfiguration::default();

    let fp1 = BuildFingerprint::compute(&pkg, &checksum, &toolchain, &config);
    let fp2 = BuildFingerprint::compute(&pkg, &checksum, &toolchain, &config);

    assert_eq!(fp1, fp2);
    assert!(fp1.as_str().starts_with("sha256:"));
    assert_eq!(fp1.hex_digest().len(), 64);

    // Profile change causes divergence
    let mut config_debug = config.clone();
    config_debug.profile = BuildProfile::Debug;
    let fp_debug = BuildFingerprint::compute(&pkg, &checksum, &toolchain, &config_debug);
    assert_ne!(fp1, fp_debug);

    // CMake args change causes divergence
    let mut config_args = config.clone();
    config_args.cmake_args.push("-DFOO=BAR".to_string());
    let fp_args = BuildFingerprint::compute(&pkg, &checksum, &toolchain, &config_args);
    assert_ne!(fp1, fp_args);

    // Source checksum change causes divergence
    let other_checksum =
        Checksum::new_sha256("1111111111111111111111111111111111111111111111111111111111111111")
            .unwrap();
    let fp_other_source = BuildFingerprint::compute(&pkg, &other_checksum, &toolchain, &config);
    assert_ne!(fp1, fp_other_source);

    // Package version change causes divergence
    let pkg_v2 = dummy_package_id("test-lib", "2.0.0");
    let fp_v2 = BuildFingerprint::compute(&pkg_v2, &checksum, &toolchain, &config);
    assert_ne!(fp1, fp_v2);
}

#[test]
fn test_toolchain_detection_with_system_cmake() {
    let temp_root = TempDirGuard::new("toolchain");
    let engine = CMakeBuildEngine::new(temp_root.path());
    let config = BuildConfiguration::default();

    let toolchain = engine
        .detect_toolchain(&config)
        .expect("toolchain detection");
    assert!(toolchain.cmake_version.to_lowercase().contains("cmake"));
    assert!(!toolchain.generator.is_empty());
}

#[test]
fn test_missing_source_and_missing_cmakelists() {
    let temp_root = TempDirGuard::new("errors");
    let engine = CMakeBuildEngine::new(temp_root.path());
    let pkg = dummy_package_id("foo", "1.0.0");
    let checksum = dummy_checksum();

    // 1. Missing source dir
    let req = BuildRequest::new(
        pkg.clone(),
        temp_root.path().join("non_existent"),
        checksum.clone(),
    );
    let err = engine.build(&req).unwrap_err();
    assert!(matches!(err, BuildError::MissingSource(_)));

    // 2. Existing dir without CMakeLists.txt
    let empty_dir = temp_root.path().join("empty_source");
    fs::create_dir_all(&empty_dir).unwrap();
    let req2 = BuildRequest::new(pkg, empty_dir, checksum);
    let err2 = engine.build(&req2).unwrap_err();
    assert!(matches!(err2, BuildError::MissingCMakeLists(_)));
}

#[test]
fn test_compile_sample_c_project_with_cmake() {
    let temp_root = TempDirGuard::new("real_build");
    let source_dir = temp_root.path().join("src");
    fs::create_dir_all(&source_dir).unwrap();

    // Create simple CMakeLists.txt and C source files
    let cmakelists = r#"
cmake_minimum_required(VERSION 3.15)
project(sample_math C)

add_library(sample_math STATIC math.c)
add_executable(math_cli main.c)
target_link_libraries(math_cli PRIVATE sample_math)
"#;
    fs::write(source_dir.join("CMakeLists.txt"), cmakelists).unwrap();

    let math_c = r#"
int add(int a, int b) {
    return a + b;
}
"#;
    fs::write(source_dir.join("math.c"), math_c).unwrap();

    let main_c = r#"
int add(int a, int b);
int main(void) {
    return add(2, 3) == 5 ? 0 : 1;
}
"#;
    fs::write(source_dir.join("main.c"), main_c).unwrap();

    let builds_root = temp_root.path().join("builds");
    let engine = CMakeBuildEngine::new(&builds_root);
    let pkg = dummy_package_id("sample-math", "1.0.0");
    let checksum = dummy_checksum();

    let req = BuildRequest::new(pkg.clone(), source_dir.clone(), checksum.clone())
        .with_configuration(BuildConfiguration::with_profile(BuildProfile::Release));

    // 1. First build (clean compilation)
    let result = engine.build(&req).expect("first build should succeed");
    assert!(!result.reused_cache);
    assert!(result.build_dir.exists());
    assert!(result.configure_log_path.exists());
    assert!(result.build_log_path.exists());
    assert!(result.metadata_path.exists());
    assert!(!result.artifacts.is_empty());

    // Verify source directory was not polluted (no CMakeCache.txt in source)
    assert!(!source_dir.join("CMakeCache.txt").exists());
    assert!(!source_dir.join("CMakeFiles").exists());

    // Check produced library and executable
    let has_lib = result
        .artifacts
        .iter()
        .any(|a| a.name.contains("sample_math") && a.artifact_type == ArtifactType::StaticLibrary);
    assert!(
        has_lib,
        "should find static library artifact: {:?}",
        result.artifacts
    );

    let has_exe = result
        .artifacts
        .iter()
        .any(|a| a.name.contains("math_cli") && a.artifact_type == ArtifactType::Executable);
    assert!(
        has_exe,
        "should find executable artifact: {:?}",
        result.artifacts
    );

    // 2. Re-running the build reuses the cache
    let result_cached = engine.build(&req).expect("second build should succeed");
    assert!(result_cached.reused_cache);
    assert_eq!(result_cached.fingerprint, result.fingerprint);

    // 3. Forcing rebuild executes fresh compilation
    let req_forced = req.with_force(true);
    let result_forced = engine
        .build(&req_forced)
        .expect("forced build should succeed");
    assert!(!result_forced.reused_cache);

    // 4. Building with Debug profile uses a distinct fingerprint and directory
    let req_debug = BuildRequest::new(pkg, source_dir, checksum)
        .with_configuration(BuildConfiguration::with_profile(BuildProfile::Debug));
    let result_debug = engine
        .build(&req_debug)
        .expect("debug build should succeed");
    assert!(!result_debug.reused_cache);
    assert_ne!(result_debug.fingerprint, result.fingerprint);
    assert_ne!(result_debug.build_dir, result.build_dir);
}

#[test]
fn test_configure_failure_diagnostics() {
    let temp_root = TempDirGuard::new("configure_fail");
    let source_dir = temp_root.path().join("src");
    fs::create_dir_all(&source_dir).unwrap();

    // Invalid CMake syntax
    let broken_cmakelists = r#"
cmake_minimum_required(VERSION 3.15)
project(broken_project C)
this_is_an_invalid_cmake_command(foo bar)
"#;
    fs::write(source_dir.join("CMakeLists.txt"), broken_cmakelists).unwrap();

    let engine = CMakeBuildEngine::new(temp_root.path().join("builds"));
    let pkg = dummy_package_id("broken-pkg", "0.1.0");
    let checksum = dummy_checksum();
    let req = BuildRequest::new(pkg, source_dir, checksum);

    let err = engine.build(&req).unwrap_err();
    match err {
        BuildError::ConfigureFailed {
            exit_code,
            log_path,
            summary,
            ..
        } => {
            assert!(exit_code.is_some());
            assert!(log_path.exists());
            assert!(
                summary.contains("Unknown CMake command")
                    || summary.contains("invalid_cmake_command")
            );
        }
        other => panic!("expected ConfigureFailed error, got: {other:?}"),
    }
}

#[test]
fn test_compilation_failure_diagnostics() {
    let temp_root = TempDirGuard::new("build_fail");
    let source_dir = temp_root.path().join("src");
    fs::create_dir_all(&source_dir).unwrap();

    // Valid CMake, invalid C code
    let cmakelists = r#"
cmake_minimum_required(VERSION 3.15)
project(compile_fail C)
add_executable(fail_bin main.c)
"#;
    fs::write(source_dir.join("CMakeLists.txt"), cmakelists).unwrap();

    let broken_c = r#"
int main(void) {
    this is completely invalid C code syntax @@#$;
    return 0;
}
"#;
    fs::write(source_dir.join("main.c"), broken_c).unwrap();

    let engine = CMakeBuildEngine::new(temp_root.path().join("builds"));
    let pkg = dummy_package_id("compile-fail-pkg", "0.1.0");
    let checksum = dummy_checksum();
    let req = BuildRequest::new(pkg, source_dir, checksum);

    let err = engine.build(&req).unwrap_err();
    match err {
        BuildError::BuildFailed {
            exit_code,
            log_path,
            summary,
            ..
        } => {
            assert!(exit_code.is_some());
            assert!(log_path.exists());
            assert!(summary.contains("error:") || summary.contains("syntax"));
        }
        other => panic!("expected BuildFailed error, got: {other:?}"),
    }
}

#[test]
fn test_spaces_in_paths_safety() {
    let temp_root = TempDirGuard::new("spaces path");
    let source_dir = temp_root.path().join("source dir with spaces");
    fs::create_dir_all(&source_dir).unwrap();

    let cmakelists = r#"
cmake_minimum_required(VERSION 3.15)
project(spaces_project C)
add_library(spaces_lib STATIC "lib spaces.c")
"#;
    fs::write(source_dir.join("CMakeLists.txt"), cmakelists).unwrap();
    fs::write(
        source_dir.join("lib spaces.c"),
        "int foo(void) { return 42; }",
    )
    .unwrap();

    let builds_root = temp_root.path().join("builds with spaces");
    let engine = CMakeBuildEngine::new(&builds_root);
    let pkg = dummy_package_id("spaces-pkg", "1.0.0");
    let checksum = dummy_checksum();
    let req = BuildRequest::new(pkg, source_dir, checksum);

    let result = engine
        .build(&req)
        .expect("build with spaces in paths must succeed");
    assert!(result.build_dir.exists());
    assert!(!result.artifacts.is_empty());
}

#[test]
fn test_cli_build_help_command() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .arg("--help")
        .output()
        .expect("crux build --help execution");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("crux-build — Build the local Crux application or a package with CMake")
    );
    assert!(stdout.contains("--profile <type>"));
    assert!(stdout.contains("--force"));
}

#[test]
fn test_cli_build_unexpected_flags() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .arg("--nonexistent-flag")
        .output()
        .expect("crux build execution");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("error: unexpected argument '--nonexistent-flag'"));
}

#[test]
fn test_cli_build_invalid_profile() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .arg("--profile")
        .arg("invalid_profile_type")
        .output()
        .expect("crux build execution");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid build profile 'invalid_profile_type'"));
}
