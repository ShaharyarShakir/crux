//! Integration tests for the Crux package installation engine.

use crux::build::{BuildConfiguration, BuildEngine, BuildProfile, BuildRequest, CMakeBuildEngine};
use crux::cli::{Action, parse_args, run};
use crux::install::{
    CMakeInstallEngine, CURRENT_INSTALL_METADATA_VERSION, InstallEngine, InstallError,
    InstallMetadata, InstallRequest, InstalledFileType,
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
            "crux_test_install_{}_{}_{}",
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

fn setup_sample_cmake_project(source_dir: &Path) {
    fs::create_dir_all(source_dir).unwrap();

    let cmake_lists = r#"
cmake_minimum_required(VERSION 3.15)
project(sample_math C)

add_library(sample_math STATIC math.c)
add_executable(math_cli main.c)
target_link_libraries(math_cli PRIVATE sample_math)

# Install rules
install(TARGETS sample_math math_cli
    ARCHIVE DESTINATION lib
    LIBRARY DESTINATION lib
    RUNTIME DESTINATION bin
)
install(FILES math.h DESTINATION include)

# Generate and install pkg-config file
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/sample_math.pc"
"prefix=${CMAKE_INSTALL_PREFIX}
libdir=${CMAKE_INSTALL_PREFIX}/lib
includedir=${CMAKE_INSTALL_PREFIX}/include

Name: sample_math
Description: Sample math library
Version: 1.0.0
Libs: -L${libdir} -lsample_math
Cflags: -I${includedir}
")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/sample_math.pc" DESTINATION lib/pkgconfig)
"#;
    let math_h = r#"
#ifndef MATH_H
#define MATH_H
int add(int a, int b);
#endif
"#;
    let math_c = r#"
#include "math.h"
int add(int a, int b) {
    return a + b;
}
"#;
    let main_c = r#"
#include <stdio.h>
#include "math.h"
int main(void) {
    printf("Sum: %d\n", add(3, 4));
    return 0;
}
"#;

    fs::write(source_dir.join("CMakeLists.txt"), cmake_lists).unwrap();
    fs::write(source_dir.join("math.h"), math_h).unwrap();
    fs::write(source_dir.join("math.c"), math_c).unwrap();
    fs::write(source_dir.join("main.c"), main_c).unwrap();
}

#[test]
fn test_installed_file_type_classification() {
    let p = Path::new("include/fmt/core.h");
    assert_eq!(InstalledFileType::classify(p, p), InstalledFileType::Header);

    let p = Path::new("lib/libfmt.a");
    assert_eq!(
        InstalledFileType::classify(p, p),
        InstalledFileType::StaticLibrary
    );

    let p = Path::new("lib/libz.so.1.3.1");
    assert_eq!(
        InstalledFileType::classify(p, p),
        InstalledFileType::SharedLibrary
    );

    let p = Path::new("bin/minigzip");
    assert_eq!(
        InstalledFileType::classify(p, p),
        InstalledFileType::Executable
    );

    let p = Path::new("lib/pkgconfig/fmt.pc");
    assert_eq!(
        InstalledFileType::classify(p, p),
        InstalledFileType::PkgConfig
    );

    let p = Path::new("lib/cmake/fmt/fmt-config.cmake");
    assert_eq!(
        InstalledFileType::classify(p, p),
        InstalledFileType::CMakeModule
    );
}

#[test]
fn test_successful_installation_and_artifacts() {
    let temp_root = TempDirGuard::new("success_install");
    let source_dir = temp_root.path().join("src");
    setup_sample_cmake_project(&source_dir);

    let builds_root = temp_root.path().join("builds");
    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");

    let build_engine = CMakeBuildEngine::new(&builds_root);
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("sample-math", "1.0.0");
    let checksum = dummy_checksum();

    let build_req = BuildRequest::new(pkg.clone(), source_dir, checksum)
        .with_configuration(BuildConfiguration::with_profile(BuildProfile::Release));

    let build_res = build_engine.build(&build_req).unwrap();
    assert!(!build_res.reused_cache);

    // 1. Install package
    let install_req = InstallRequest::new(build_res);
    let install_res = install_engine
        .install(&install_req)
        .expect("installation should succeed");

    assert!(!install_res.reused_install);
    assert_eq!(install_res.package_id, pkg);
    assert!(install_res.install_dir.exists());
    assert!(install_res.metadata_path.exists());
    assert!(install_res.total_size_bytes > 0);

    // Staging root should be clean (temporary staging directory was atomically moved)
    assert!(
        fs::read_dir(&staging_root).unwrap().next().is_none(),
        "staging root should be empty after publication"
    );

    // 2. Verify expected installed files on disk
    let dest_dir = &install_res.install_dir;
    assert!(dest_dir.join("include/math.h").is_file());
    assert!(
        dest_dir.join("lib/libsample_math.a").is_file()
            || dest_dir.join("lib64/libsample_math.a").is_file()
    );
    assert!(dest_dir.join("bin/math_cli").is_file() || dest_dir.join("bin/math_cli.exe").is_file());
    assert!(
        dest_dir.join("lib/pkgconfig/sample_math.pc").is_file()
            || dest_dir.join("lib64/pkgconfig/sample_math.pc").is_file()
    );

    // 3. Verify metadata contents in installed.toml
    let meta_str = fs::read_to_string(&install_res.metadata_path).unwrap();
    let metadata: InstallMetadata = toml::from_str(&meta_str).unwrap();

    assert_eq!(metadata.format_version, CURRENT_INSTALL_METADATA_VERSION);
    assert_eq!(metadata.package_name, "sample-math");
    assert_eq!(metadata.package_version, "1.0.0");
    assert_eq!(metadata.fingerprint, install_res.fingerprint);
    assert_eq!(metadata.profile, BuildProfile::Release);
    assert!(metadata.success);
    assert!(!metadata.installed_files.is_empty());

    // 4. Verify deterministic inventory ordering (alphabetical)
    let paths: Vec<PathBuf> = metadata
        .installed_files
        .iter()
        .map(|f| f.relative_path.clone())
        .collect();
    let mut sorted_paths = paths.clone();
    sorted_paths.sort();
    assert_eq!(
        paths, sorted_paths,
        "installed files must be strictly sorted by relative path"
    );

    // 5. Verify cache reuse
    let install_res2 = install_engine
        .install(&install_req)
        .expect("second install should succeed");
    assert!(
        install_res2.reused_install,
        "second install must reuse existing valid installation"
    );
    assert_eq!(install_res2.install_dir, install_res.install_dir);

    // 6. Verify forced reinstall
    let forced_req = install_req.with_force(true);
    let install_res_forced = install_engine
        .install(&forced_req)
        .expect("forced install should succeed");
    assert!(
        !install_res_forced.reused_install,
        "forced install must not reuse cache"
    );
}

#[test]
fn test_missing_installed_file_invalidates_cache_and_reinstalls() {
    let temp_root = TempDirGuard::new("cache_invalidation");
    let source_dir = temp_root.path().join("src");
    setup_sample_cmake_project(&source_dir);

    let builds_root = temp_root.path().join("builds");
    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");

    let build_engine = CMakeBuildEngine::new(&builds_root);
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("sample-math", "1.0.0");
    let checksum = dummy_checksum();

    let build_req = BuildRequest::new(pkg, source_dir, checksum)
        .with_configuration(BuildConfiguration::with_profile(BuildProfile::Release));

    let build_res = build_engine.build(&build_req).unwrap();
    let install_req = InstallRequest::new(build_res);

    // First install
    let res1 = install_engine.install(&install_req).unwrap();
    assert!(!res1.reused_install);

    // Delete a tracked file (e.g. math.h)
    let math_h = res1.install_dir.join("include/math.h");
    assert!(math_h.is_file());
    fs::remove_file(&math_h).unwrap();
    assert!(!math_h.exists());

    // Second install should detect the missing file, invalidate cache, and reinstall
    let res2 = install_engine.install(&install_req).unwrap();
    assert!(
        !res2.reused_install,
        "missing file must invalidate cache and trigger reinstallation"
    );
    assert!(math_h.is_file(), "missing file must be restored");
}

#[test]
fn test_corrupt_metadata_invalidates_cache() {
    let temp_root = TempDirGuard::new("corrupt_meta");
    let source_dir = temp_root.path().join("src");
    setup_sample_cmake_project(&source_dir);

    let builds_root = temp_root.path().join("builds");
    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");

    let build_engine = CMakeBuildEngine::new(&builds_root);
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("sample-math", "1.0.0");
    let checksum = dummy_checksum();

    let build_req = BuildRequest::new(pkg, source_dir, checksum)
        .with_configuration(BuildConfiguration::with_profile(BuildProfile::Release));

    let build_res = build_engine.build(&build_req).unwrap();
    let install_req = InstallRequest::new(build_res);

    let res1 = install_engine.install(&install_req).unwrap();
    assert!(!res1.reused_install);

    // Corrupt installed.toml with invalid format version
    fs::write(&res1.metadata_path, "format_version = 9999\n").unwrap();

    let res2 = install_engine.install(&install_req).unwrap();
    assert!(
        !res2.reused_install,
        "corrupted metadata must trigger clean reinstall"
    );
}

#[test]
fn test_installation_failure_preserves_valid_install() {
    let temp_root = TempDirGuard::new("install_fail_preserve");
    let source_dir = temp_root.path().join("src");
    setup_sample_cmake_project(&source_dir);

    let builds_root = temp_root.path().join("builds");
    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");

    let build_engine = CMakeBuildEngine::new(&builds_root);
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("sample-math", "1.0.0");
    let checksum = dummy_checksum();

    let build_req = BuildRequest::new(pkg, source_dir, checksum)
        .with_configuration(BuildConfiguration::with_profile(BuildProfile::Release));

    let build_res = build_engine.build(&build_req).unwrap();
    let install_req = InstallRequest::new(build_res.clone());

    // 1. Initial valid installation
    let res = install_engine.install(&install_req).unwrap();
    let math_h = res.install_dir.join("include/math.h");
    assert!(math_h.is_file());

    // 2. Corrupt cmake_install.cmake in the build directory to force installation failure
    let cmake_install_script = build_res.build_dir.join("cmake_install.cmake");
    fs::write(
        &cmake_install_script,
        "message(FATAL_ERROR \"forced install failure\")\n",
    )
    .unwrap();

    // 3. Attempt forced reinstall, which will run cmake --install and fail
    let forced_req = install_req.with_force(true);
    let err = install_engine
        .install(&forced_req)
        .expect_err("should fail when cmake_install fails");

    assert!(matches!(err, InstallError::CMakeExecutionFailed { .. }));

    // 4. Verify previous valid installation was NOT corrupted or deleted
    assert!(
        math_h.is_file(),
        "previous valid installation must be preserved"
    );
    assert!(res.metadata_path.is_file());
}

#[test]
fn test_installation_in_directory_path_with_spaces() {
    let temp_root = TempDirGuard::new("path with spaces");
    let source_dir = temp_root.path().join("source dir with spaces");
    setup_sample_cmake_project(&source_dir);

    let builds_root = temp_root.path().join("builds dir with spaces");
    let install_root = temp_root.path().join("installed dir with spaces");
    let staging_root = temp_root.path().join("staging dir with spaces");

    let build_engine = CMakeBuildEngine::new(&builds_root);
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("sample-math", "1.0.0");
    let checksum = dummy_checksum();

    let build_req = BuildRequest::new(pkg, source_dir, checksum);
    let build_res = build_engine.build(&build_req).unwrap();

    let install_req = InstallRequest::new(build_res);
    let install_res = install_engine
        .install(&install_req)
        .expect("installation with spaces in paths must succeed");

    assert!(install_res.install_dir.exists());
    assert!(install_res.install_dir.join("include/math.h").is_file());
}

#[test]
fn test_missing_or_invalid_build_directory_fails() {
    let temp_root = TempDirGuard::new("invalid_build_dir");
    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("sample", "1.0.0");
    let non_existent_dir = temp_root.path().join("non_existent_build_dir");

    let dummy_fingerprint = crux::build::BuildFingerprint::new(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    )
    .unwrap();

    let fake_result = crux::build::BuildResult {
        package_id: pkg.clone(),
        fingerprint: dummy_fingerprint.clone(),
        source_dir: temp_root.path().join("src"),
        build_dir: non_existent_dir.clone(),
        logs_dir: temp_root.path().join("logs"),
        configure_log_path: temp_root.path().join("logs/configure.log"),
        build_log_path: temp_root.path().join("logs/build.log"),
        metadata_path: temp_root.path().join("metadata.toml"),
        artifacts: Vec::new(),
        profile: BuildProfile::Release,
        duration: std::time::Duration::from_millis(100),
        reused_cache: false,
    };

    let err = install_engine
        .install(&InstallRequest::new(fake_result))
        .unwrap_err();
    assert!(matches!(err, InstallError::MissingBuildDirectory(_)));

    // Create empty dir without CMake configuration
    fs::create_dir_all(&non_existent_dir).unwrap();
    let fake_result2 = crux::build::BuildResult {
        package_id: pkg,
        fingerprint: dummy_fingerprint,
        source_dir: temp_root.path().join("src"),
        build_dir: non_existent_dir,
        logs_dir: temp_root.path().join("logs"),
        configure_log_path: temp_root.path().join("logs/configure.log"),
        build_log_path: temp_root.path().join("logs/build.log"),
        metadata_path: temp_root.path().join("metadata.toml"),
        artifacts: Vec::new(),
        profile: BuildProfile::Release,
        duration: std::time::Duration::from_millis(100),
        reused_cache: false,
    };

    let err2 = install_engine
        .install(&InstallRequest::new(fake_result2))
        .unwrap_err();
    assert!(matches!(err2, InstallError::MissingCMakeConfig(_)));
}

#[test]
fn test_profile_isolation_installs_into_distinct_directories() {
    let temp_root = TempDirGuard::new("profile_isolation");
    let source_dir = temp_root.path().join("src");
    setup_sample_cmake_project(&source_dir);

    let builds_root = temp_root.path().join("builds");
    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");

    let build_engine = CMakeBuildEngine::new(&builds_root);
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("sample-math", "1.0.0");
    let checksum = dummy_checksum();

    // Release build & install
    let req_rel = BuildRequest::new(pkg.clone(), source_dir.clone(), checksum.clone())
        .with_configuration(BuildConfiguration::with_profile(BuildProfile::Release));
    let b_rel = build_engine.build(&req_rel).unwrap();
    let inst_rel = install_engine.install(&InstallRequest::new(b_rel)).unwrap();

    // Debug build & install
    let req_deb = BuildRequest::new(pkg, source_dir, checksum)
        .with_configuration(BuildConfiguration::with_profile(BuildProfile::Debug));
    let b_deb = build_engine.build(&req_deb).unwrap();
    let inst_deb = install_engine.install(&InstallRequest::new(b_deb)).unwrap();

    assert_ne!(
        inst_rel.fingerprint, inst_deb.fingerprint,
        "Release and Debug must have distinct fingerprints"
    );
    assert_ne!(
        inst_rel.install_dir, inst_deb.install_dir,
        "Release and Debug must install into separate directories"
    );
    assert!(inst_rel.install_dir.exists());
    assert!(inst_deb.install_dir.exists());
}

#[test]
fn test_cli_install_args_parsing() {
    assert_eq!(
        parse_args(&["install"]).unwrap(),
        Action::Install {
            package: None,
            profile: BuildProfile::Release,
            force: false,
            locked: false,
        }
    );

    assert_eq!(
        parse_args(&["install", "--locked"]).unwrap(),
        Action::Install {
            package: None,
            profile: BuildProfile::Release,
            force: false,
            locked: true,
        }
    );

    assert_eq!(
        parse_args(&[
            "install",
            "fmt@11.1.1",
            "--profile",
            "debug",
            "--force",
            "--locked"
        ])
        .unwrap(),
        Action::Install {
            package: Some("fmt@11.1.1".to_string()),
            profile: BuildProfile::Debug,
            force: true,
            locked: true,
        }
    );

    assert_eq!(parse_args(&["install", "-h"]).unwrap(), Action::InstallHelp);
    assert_eq!(
        parse_args(&["install", "--help"]).unwrap(),
        Action::InstallHelp
    );
}

#[test]
fn test_cli_install_help_output() {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let res = run(["crux", "install", "--help"], &mut out, &mut err);
    assert!(res.is_ok());
    let output = String::from_utf8(out).unwrap();
    assert!(output.contains("crux-install — Build and install packages"));
    assert!(output.contains("--locked"));
    assert!(output.contains("--profile"));
    assert!(output.contains("--force"));
    assert!(err.is_empty());
}

#[test]
fn test_locked_flag_fails_when_lockfile_missing() {
    let temp_root = TempDirGuard::new("locked_fail_no_lock");
    let mut out = Vec::new();
    let mut err = Vec::new();

    let res = crux::cli::run_install(
        temp_root.path(),
        None,
        BuildProfile::Release,
        false,
        true, // locked = true
        &mut out,
        &mut err,
    );

    assert!(res.is_err());
    let err_msg = String::from_utf8(err).unwrap();
    assert!(err_msg.contains("--locked was specified, but lockfile not found"));
}

#[test]
fn test_locked_flag_does_not_modify_existing_lockfile() {
    let temp_root = TempDirGuard::new("locked_no_modify");
    let lock_path = temp_root.path().join("crux.lock");

    // Write empty lockfile
    let initial_lock_content = r#"lockfile_version = 1
"#;
    fs::write(&lock_path, initial_lock_content).unwrap();

    let mut out = Vec::new();
    let mut err = Vec::new();

    // Run install in --locked mode
    let _ = crux::cli::run_install(
        temp_root.path(),
        None,
        BuildProfile::Release,
        false,
        true,
        &mut out,
        &mut err,
    );

    let current_content = fs::read_to_string(&lock_path).unwrap();
    assert_eq!(
        current_content, initial_lock_content,
        "crux install --locked must never modify crux.lock"
    );
}

#[test]
fn test_symlink_escape_rejected_by_staging_validation() {
    let temp_root = TempDirGuard::new("symlink_escape");
    let build_dir = temp_root.path().join("fake_build");
    fs::create_dir_all(&build_dir).unwrap();

    // Create cmake_install.cmake that creates a symlink targeting outside staging
    let install_script = r#"
file(CREATE_LINK "/etc" "${CMAKE_INSTALL_PREFIX}/leak_link" SYMBOLIC)
file(WRITE "${CMAKE_INSTALL_PREFIX}/file.txt" "content")
"#;
    fs::write(build_dir.join("cmake_install.cmake"), install_script).unwrap();

    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("leak-pkg", "1.0.0");
    let dummy_fingerprint = crux::build::BuildFingerprint::new(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    )
    .unwrap();

    let fake_result = crux::build::BuildResult {
        package_id: pkg,
        fingerprint: dummy_fingerprint,
        source_dir: temp_root.path().join("src"),
        build_dir,
        logs_dir: temp_root.path().join("logs"),
        configure_log_path: temp_root.path().join("logs/configure.log"),
        build_log_path: temp_root.path().join("logs/build.log"),
        metadata_path: temp_root.path().join("metadata.toml"),
        artifacts: Vec::new(),
        profile: BuildProfile::Release,
        duration: std::time::Duration::from_millis(100),
        reused_cache: false,
    };

    let err = install_engine
        .install(&InstallRequest::new(fake_result))
        .expect_err("symlink escaping staging root must be rejected");

    assert!(matches!(err, InstallError::SecurityViolation(_)));
}

#[test]
fn test_internal_symlink_within_staging_is_accepted() {
    let temp_root = TempDirGuard::new("internal_symlink");
    let build_dir = temp_root.path().join("fake_build");
    fs::create_dir_all(&build_dir).unwrap();

    // Create cmake_install.cmake that creates a valid relative symlink within staging
    let install_script = r#"
file(WRITE "${CMAKE_INSTALL_PREFIX}/libz.so.1.3.1" "zlib binary content")
file(CREATE_LINK "libz.so.1.3.1" "${CMAKE_INSTALL_PREFIX}/libz.so" SYMBOLIC)
"#;
    fs::write(build_dir.join("cmake_install.cmake"), install_script).unwrap();

    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("zlib-fake", "1.3.1");
    let dummy_fingerprint = crux::build::BuildFingerprint::new(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    )
    .unwrap();

    let fake_result = crux::build::BuildResult {
        package_id: pkg,
        fingerprint: dummy_fingerprint,
        source_dir: temp_root.path().join("src"),
        build_dir,
        logs_dir: temp_root.path().join("logs"),
        configure_log_path: temp_root.path().join("logs/configure.log"),
        build_log_path: temp_root.path().join("logs/build.log"),
        metadata_path: temp_root.path().join("metadata.toml"),
        artifacts: Vec::new(),
        profile: BuildProfile::Release,
        duration: std::time::Duration::from_millis(100),
        reused_cache: false,
    };

    let res = install_engine
        .install(&InstallRequest::new(fake_result))
        .expect("internal relative symlinks must be accepted");

    assert!(res.install_dir.join("libz.so.1.3.1").is_file());
    assert!(res.install_dir.join("libz.so").exists());
}

#[test]
fn test_empty_installation_is_rejected() {
    let temp_root = TempDirGuard::new("empty_install");
    let build_dir = temp_root.path().join("fake_build");
    fs::create_dir_all(&build_dir).unwrap();

    // Empty cmake_install.cmake produces 0 installed files
    fs::write(build_dir.join("cmake_install.cmake"), "# empty\n").unwrap();

    let install_root = temp_root.path().join("installed");
    let staging_root = temp_root.path().join("staging");
    let install_engine = CMakeInstallEngine::new(&install_root, &staging_root);

    let pkg = dummy_package_id("empty-pkg", "1.0.0");
    let dummy_fingerprint = crux::build::BuildFingerprint::new(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    )
    .unwrap();

    let fake_result = crux::build::BuildResult {
        package_id: pkg,
        fingerprint: dummy_fingerprint,
        source_dir: temp_root.path().join("src"),
        build_dir,
        logs_dir: temp_root.path().join("logs"),
        configure_log_path: temp_root.path().join("logs/configure.log"),
        build_log_path: temp_root.path().join("logs/build.log"),
        metadata_path: temp_root.path().join("metadata.toml"),
        artifacts: Vec::new(),
        profile: BuildProfile::Release,
        duration: std::time::Duration::from_millis(100),
        reused_cache: false,
    };

    let err = install_engine
        .install(&InstallRequest::new(fake_result))
        .expect_err("installation with 0 files must be rejected");

    assert!(matches!(err, InstallError::EmptyInstallation(_)));
}
