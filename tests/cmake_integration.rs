//! Integration tests for Crux CMake Consumer Integration (Milestone 13).

use crux::build::{BuildFingerprint, BuildProfile};
use crux::cli::{Action, CliError, parse_args, run_integrate_cmake};
use crux::install::{
    CURRENT_INSTALL_METADATA_VERSION, InstallMetadata, InstalledFile, InstalledFileType,
};
use crux::integrate::{
    CMakeIntegrationGenerator, IntegrationError, ValidatedPackageInstallation,
    collect_transitive_dependencies, escape_cmake_path, generate_cmake_dependencies,
    sanitize_cmake_identifier, validate_package_installation, write_cmake_dependencies_atomically,
};
use crux::lock::{Checksum, LockedPackage, Lockfile, PackageSource};
use crux::package::{PackageId, PackageName};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// RAII temporary directory guard.
struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let nonce: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let temp_dir = std::env::temp_dir().join(format!("crux_test_{prefix}_{nonce}"));
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

fn create_mock_installed_package(
    project_root: &Path,
    pkg_name: &str,
    pkg_version: &str,
    fingerprint_hex: &str,
    profile: BuildProfile,
    files: &[(&str, InstalledFileType, &[u8])],
) -> ValidatedPackageInstallation {
    let install_dir = project_root
        .join(".crux")
        .join("installed")
        .join(pkg_name)
        .join(pkg_version)
        .join(fingerprint_hex);
    fs::create_dir_all(&install_dir).unwrap();

    let mut installed_files = Vec::new();
    let mut total_size = 0u64;

    for (rel_path_str, file_type, content) in files {
        let rel_path = PathBuf::from(rel_path_str);
        let full_path = install_dir.join(&rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full_path, content).unwrap();

        let size = content.len() as u64;
        total_size += size;
        installed_files.push(InstalledFile {
            relative_path: rel_path,
            file_type: *file_type,
            size_bytes: size,
        });
    }

    let fp = BuildFingerprint::new(format!("sha256:{fingerprint_hex}")).unwrap();
    let metadata = InstallMetadata {
        format_version: CURRENT_INSTALL_METADATA_VERSION,
        package_name: pkg_name.to_string(),
        package_version: pkg_version.to_string(),
        fingerprint: fp.clone(),
        profile,
        installed_files,
        total_size_bytes: total_size,
        success: true,
        installed_at: Some("2026-11-15T12:00:00Z".to_string()),
    };

    let meta_str = toml::to_string_pretty(&metadata).unwrap();
    fs::write(install_dir.join("installed.toml"), meta_str).unwrap();

    let pkg_id = PackageId::new(
        PackageName::new(pkg_name).unwrap(),
        semver::Version::parse(pkg_version).unwrap(),
    );

    ValidatedPackageInstallation {
        package_id: pkg_id,
        fingerprint: fp,
        profile,
        install_dir,
        metadata,
    }
}

#[test]
fn test_cli_integrate_help() {
    assert_eq!(parse_args(&["integrate"]).unwrap(), Action::IntegrateHelp);
    assert_eq!(
        parse_args(&["integrate", "-h"]).unwrap(),
        Action::IntegrateHelp
    );
    assert_eq!(
        parse_args(&["integrate", "--help"]).unwrap(),
        Action::IntegrateHelp
    );

    assert_eq!(
        parse_args(&["integrate", "cmake", "-h"]).unwrap(),
        Action::IntegrateCmakeHelp
    );
    assert_eq!(
        parse_args(&["integrate", "cmake", "--help"]).unwrap(),
        Action::IntegrateCmakeHelp
    );
}

#[test]
fn test_cli_integrate_argument_parsing() {
    assert_eq!(
        parse_args(&["integrate", "cmake"]).unwrap(),
        Action::IntegrateCmake {
            package: None,
            profile: BuildProfile::Release,
            locked: false,
        }
    );

    assert_eq!(
        parse_args(&[
            "integrate",
            "cmake",
            "fmt",
            "--profile",
            "debug",
            "--locked"
        ])
        .unwrap(),
        Action::IntegrateCmake {
            package: Some("fmt".to_string()),
            profile: BuildProfile::Debug,
            locked: true,
        }
    );

    assert_eq!(
        parse_args(&["integrate", "cmake", "fmt@11.1.1", "--profile=release"]).unwrap(),
        Action::IntegrateCmake {
            package: Some("fmt@11.1.1".to_string()),
            profile: BuildProfile::Release,
            locked: false,
        }
    );
}

#[test]
fn test_cli_integrate_invalid_arguments() {
    assert!(matches!(
        parse_args(&["integrate", "unknown"]),
        Err(CliError::UnexpectedArgument(_))
    ));

    assert!(matches!(
        parse_args(&["integrate", "cmake", "--profile"]),
        Err(CliError::MissingArgumentValue(_))
    ));

    assert!(matches!(
        parse_args(&["integrate", "cmake", "--profile", "invalid_profile"]),
        Err(CliError::General(_))
    ));

    assert!(matches!(
        parse_args(&["integrate", "cmake", "fmt", "extra"]),
        Err(CliError::UnexpectedArgument(_))
    ));
}

#[test]
fn test_escape_cmake_path_safety() {
    // Spaces
    assert_eq!(
        escape_cmake_path("path with spaces/foo").unwrap(),
        "path with spaces/foo"
    );

    // Windows backslashes
    assert_eq!(
        escape_cmake_path("path\\with\\backslashes").unwrap(),
        "path/with/backslashes"
    );

    // Semicolons
    assert_eq!(
        escape_cmake_path("path;with;semicolons").unwrap(),
        "path\\;with\\;semicolons"
    );

    // Quotes and dollar signs
    assert_eq!(
        escape_cmake_path("path\"with$quotes").unwrap(),
        "path\\\"with\\$quotes"
    );

    // Rejection of control characters
    assert!(escape_cmake_path("path\nwith\nnewlines").is_err());
    assert!(escape_cmake_path("path\rwith\rcarriage_return").is_err());
    assert!(escape_cmake_path("path\0with\0null").is_err());
}

#[test]
fn test_sanitize_cmake_identifier() {
    assert_eq!(sanitize_cmake_identifier("fmt"), "FMT");
    assert_eq!(sanitize_cmake_identifier("Catch2"), "CATCH2");
    assert_eq!(sanitize_cmake_identifier("zlib"), "ZLIB");
    assert_eq!(
        sanitize_cmake_identifier("my-lib.special"),
        "MY_LIB_SPECIAL"
    );
}

#[test]
fn test_deterministic_cmake_generation() {
    let temp = TempDirGuard::new("deterministic");
    let p1 = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[(
            "include/fmt/core.h",
            InstalledFileType::Header,
            b"// fmt core",
        )],
    );
    let p2 = create_mock_installed_package(
        temp.path(),
        "zlib",
        "1.3.1",
        "32a1e05a8b9415497ee73319854740f95180630bfe1bbdbe5d8ae9b75283fc35",
        BuildProfile::Release,
        &[(
            "include/zlib.h",
            InstalledFileType::Header,
            b"// zlib header",
        )],
    );

    let content1 = generate_cmake_dependencies(temp.path(), &[p1.clone(), p2.clone()]).unwrap();
    let content2 = generate_cmake_dependencies(temp.path(), &[p2, p1]).unwrap();

    // Package insertion order must not affect output; sorted alphabetically
    assert_eq!(content1, content2);
    assert!(content1.contains("set(_CRUX_FMT_PREFIX"));
    assert!(content1.contains("set(_CRUX_ZLIB_PREFIX"));
    assert!(content1.contains("CMAKE_PREFIX_PATH"));
    assert!(content1.contains("cmake_minimum_required(VERSION 3.20)"));
}

#[test]
fn test_multiple_package_prefixes_and_module_paths() {
    let temp = TempDirGuard::new("prefixes");
    let p1 = create_mock_installed_package(
        temp.path(),
        "Catch2",
        "3.8.0",
        "d96e4c70a83e0202888636ef6cfcb05b8a6be96d36e2f1f0b093caee190c76fb",
        BuildProfile::Release,
        &[
            (
                "include/catch2/catch.hpp",
                InstalledFileType::Header,
                b"// catch2",
            ),
            (
                "lib64/libCatch2.a",
                InstalledFileType::StaticLibrary,
                b"!<arch>",
            ),
            (
                "share/cmake/Modules/FindCustom.cmake",
                InstalledFileType::CMakeModule,
                b"# module",
            ),
        ],
    );

    let content = generate_cmake_dependencies(temp.path(), &[p1]).unwrap();
    assert!(content.contains("_CRUX_CATCH2_PREFIX"));
    assert!(content.contains("share/cmake/Modules"));
    assert!(content.contains("CMAKE_MODULE_PATH"));
}

#[test]
fn test_preserve_existing_cmake_prefix_and_module_paths_via_cmake_script() {
    let temp = TempDirGuard::new("cmake_exec");
    let p1 = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[("include/fmt/core.h", InstalledFileType::Header, b"// fmt")],
    );

    let generator = CMakeIntegrationGenerator::new(temp.path());
    let res = generator.integrate_packages(&[p1]).unwrap();

    // Test with a test script executed by CMake
    let test_cmake_script = temp.path().join("test_script.cmake");
    let script_content = format!(
        r#"
set(CMAKE_PREFIX_PATH "/existing/consumer/prefix")
set(CMAKE_MODULE_PATH "/existing/consumer/module")

include("{}")

# Verify Crux prefix is at head of CMAKE_PREFIX_PATH
list(GET CMAKE_PREFIX_PATH 0 _first_prefix)
if(NOT _first_prefix MATCHES ".crux/installed/fmt/11.1.1")
    message(FATAL_ERROR "Crux prefix was not prepended to CMAKE_PREFIX_PATH: '${{CMAKE_PREFIX_PATH}}'")
endif()

# Verify consumer prefix is preserved
if(NOT "/existing/consumer/prefix" IN_LIST CMAKE_PREFIX_PATH)
    message(FATAL_ERROR "Consumer prefix was lost from CMAKE_PREFIX_PATH")
endif()

# Verify module path is preserved
if(NOT "/existing/consumer/module" IN_LIST CMAKE_MODULE_PATH)
    message(FATAL_ERROR "Consumer module path was lost from CMAKE_MODULE_PATH")
endif()

# Test include guard: second include should be a no-op
set(_len_before 0)
list(LENGTH CMAKE_PREFIX_PATH _len_before)
include("{}")
set(_len_after 0)
list(LENGTH CMAKE_PREFIX_PATH _len_after)

if(NOT _len_before EQUAL _len_after)
    message(FATAL_ERROR "Duplicate include altered CMAKE_PREFIX_PATH length")
endif()

message(STATUS "CMake test verification passed successfully")
"#,
        res.output_path.display(),
        res.output_path.display(),
    );

    fs::write(&test_cmake_script, script_content).unwrap();

    let output = Command::new("cmake")
        .current_dir(temp.path())
        .arg("-P")
        .arg(&test_cmake_script)
        .output()
        .expect("failed to execute cmake -P");

    assert!(
        output.status.success(),
        "cmake verification failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_missing_installed_directory_reports_fatal_error_in_cmake() {
    let temp = TempDirGuard::new("missing_dir");
    let p1 = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[("include/fmt/core.h", InstalledFileType::Header, b"// fmt")],
    );

    let generator = CMakeIntegrationGenerator::new(temp.path());
    let res = generator
        .integrate_packages(std::slice::from_ref(&p1))
        .unwrap();

    // Now delete the installation directory to simulate missing installed files
    fs::remove_dir_all(&p1.install_dir).unwrap();

    let test_script = temp.path().join("test_missing.cmake");
    fs::write(
        &test_script,
        format!("include(\"{}\")\n", res.output_path.display()),
    )
    .unwrap();

    let output = Command::new("cmake")
        .current_dir(temp.path())
        .arg("-P")
        .arg(&test_script)
        .output()
        .expect("failed to execute cmake");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Crux dependency 'fmt' (11.1.1) not found at expected path"));
    assert!(stderr.contains("Run 'crux install' to restore installed dependencies"));
}

#[test]
fn test_validation_rejects_unsupported_metadata_version() {
    let temp = TempDirGuard::new("meta_ver");
    let p = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[("include/fmt/core.h", InstalledFileType::Header, b"// fmt")],
    );

    // Corrupt format_version to 999
    let meta_path = p.install_dir.join("installed.toml");
    let content = fs::read_to_string(&meta_path).unwrap();
    let corrupted = content.replace("format_version = 1", "format_version = 999");
    fs::write(&meta_path, corrupted).unwrap();

    let res = validate_package_installation(temp.path(), "fmt", "11.1.1", BuildProfile::Release);
    assert!(matches!(
        res,
        Err(IntegrationError::CorruptInstallation { .. })
    ));
}

#[test]
fn test_validation_rejects_missing_file() {
    let temp = TempDirGuard::new("missing_file");
    let p = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[("include/fmt/core.h", InstalledFileType::Header, b"// fmt")],
    );

    // Delete the indexed file
    fs::remove_file(p.install_dir.join("include/fmt/core.h")).unwrap();

    let res = validate_package_installation(temp.path(), "fmt", "11.1.1", BuildProfile::Release);
    assert!(matches!(
        res,
        Err(IntegrationError::CorruptInstallation { .. })
    ));
}

#[test]
fn test_validation_rejects_file_size_mismatch() {
    let temp = TempDirGuard::new("size_mismatch");
    let p = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[(
            "include/fmt/core.h",
            InstalledFileType::Header,
            b"// fmt original",
        )],
    );

    // Alter file size
    fs::write(p.install_dir.join("include/fmt/core.h"), b"modified size").unwrap();

    let res = validate_package_installation(temp.path(), "fmt", "11.1.1", BuildProfile::Release);
    assert!(matches!(
        res,
        Err(IntegrationError::CorruptInstallation { .. })
    ));
}

#[test]
fn test_validation_rejects_fingerprint_mismatch() {
    let temp = TempDirGuard::new("fp_mismatch");
    let p = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[("include/fmt/core.h", InstalledFileType::Header, b"// fmt")],
    );

    // Corrupt fingerprint digest in metadata
    let meta_path = p.install_dir.join("installed.toml");
    let content = fs::read_to_string(&meta_path).unwrap();
    let corrupted = content.replace(
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        "0000000000000000000000000000000000000000000000000000000000000000",
    );
    fs::write(&meta_path, corrupted).unwrap();

    let res = validate_package_installation(temp.path(), "fmt", "11.1.1", BuildProfile::Release);
    assert!(matches!(
        res,
        Err(IntegrationError::CorruptInstallation { .. })
    ));
}

#[test]
fn test_validation_rejects_path_traversal_in_metadata() {
    let temp = TempDirGuard::new("traversal");
    let p = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[("include/fmt/core.h", InstalledFileType::Header, b"// fmt")],
    );

    let meta_path = p.install_dir.join("installed.toml");
    let mut meta: InstallMetadata =
        toml::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
    meta.installed_files.push(InstalledFile {
        relative_path: PathBuf::from("../secret.txt"),
        file_type: InstalledFileType::Other,
        size_bytes: 10,
    });
    fs::write(&meta_path, toml::to_string_pretty(&meta).unwrap()).unwrap();

    let res = validate_package_installation(temp.path(), "fmt", "11.1.1", BuildProfile::Release);
    assert!(matches!(
        res,
        Err(IntegrationError::SecurityViolation { .. })
    ));
}

#[cfg(unix)]
#[test]
fn test_validation_rejects_symlink_escaping_install_root() {
    use std::os::unix::fs::symlink;

    let temp = TempDirGuard::new("symlink_escape");
    let p = create_mock_installed_package(
        temp.path(),
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[("include/fmt/core.h", InstalledFileType::Header, b"// fmt")],
    );

    // Create an external malicious target and symlink pointing to it
    let external_file = temp.path().join("external_secret.txt");
    fs::write(&external_file, b"secret").unwrap();

    let bad_link = p.install_dir.join("lib/escape_link.so");
    fs::create_dir_all(bad_link.parent().unwrap()).unwrap();
    symlink(&external_file, &bad_link).unwrap();

    let meta_path = p.install_dir.join("installed.toml");
    let mut meta: InstallMetadata =
        toml::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
    meta.installed_files.push(InstalledFile {
        relative_path: PathBuf::from("lib/escape_link.so"),
        file_type: InstalledFileType::SharedLibrary,
        size_bytes: 6,
    });
    fs::write(&meta_path, toml::to_string_pretty(&meta).unwrap()).unwrap();

    let res = validate_package_installation(temp.path(), "fmt", "11.1.1", BuildProfile::Release);
    assert!(matches!(
        res,
        Err(IntegrationError::SecurityViolation { .. })
    ));
}

#[cfg(unix)]
#[test]
fn test_validation_accepts_valid_internal_symlink() {
    use std::os::unix::fs::symlink;

    let temp = TempDirGuard::new("valid_symlink");
    let p = create_mock_installed_package(
        temp.path(),
        "zlib",
        "1.3.1",
        "32a1e05a8b9415497ee73319854740f95180630bfe1bbdbe5d8ae9b75283fc35",
        BuildProfile::Release,
        &[
            ("include/zlib.h", InstalledFileType::Header, b"// zlib"),
            (
                "lib/libz.so.1.3.1",
                InstalledFileType::SharedLibrary,
                b"ELF...",
            ),
        ],
    );

    // Create valid internal relative symlink: libz.so -> libz.so.1.3.1
    let link_path = p.install_dir.join("lib/libz.so");
    symlink("libz.so.1.3.1", &link_path).unwrap();

    let meta_path = p.install_dir.join("installed.toml");
    let mut meta: InstallMetadata =
        toml::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
    meta.installed_files.push(InstalledFile {
        relative_path: PathBuf::from("lib/libz.so"),
        file_type: InstalledFileType::SharedLibrary,
        size_bytes: 0,
    });
    fs::write(&meta_path, toml::to_string_pretty(&meta).unwrap()).unwrap();

    let res = validate_package_installation(temp.path(), "zlib", "1.3.1", BuildProfile::Release);
    assert!(res.is_ok());
}

#[test]
fn test_transitive_dependency_collection() {
    let mut lockfile = Lockfile::new();

    let leaf_id = PackageId::new(
        PackageName::new("leaf").unwrap(),
        semver::Version::parse("1.0.0").unwrap(),
    );
    let mid_id = PackageId::new(
        PackageName::new("mid").unwrap(),
        semver::Version::parse("1.0.0").unwrap(),
    );
    let root_id = PackageId::new(
        PackageName::new("root").unwrap(),
        semver::Version::parse("1.0.0").unwrap(),
    );

    let leaf = LockedPackage::new(
        leaf_id.name().clone(),
        leaf_id.version().clone(),
        PackageSource::default_registry(),
    )
    .with_checksum(
        Checksum::new_sha256("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .unwrap(),
    );

    let mid = LockedPackage::new(
        mid_id.name().clone(),
        mid_id.version().clone(),
        PackageSource::default_registry(),
    )
    .with_checksum(
        Checksum::new_sha256("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .unwrap(),
    )
    .with_dependencies(vec![leaf_id]);

    let root = LockedPackage::new(
        root_id.name().clone(),
        root_id.version().clone(),
        PackageSource::default_registry(),
    )
    .with_checksum(
        Checksum::new_sha256("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .unwrap(),
    )
    .with_dependencies(vec![mid_id]);

    lockfile.packages.push(leaf);
    lockfile.packages.push(mid);
    lockfile.packages.push(root);

    let collected = collect_transitive_dependencies(&lockfile, "root").unwrap();
    assert_eq!(collected.len(), 3);
    assert_eq!(collected[0].name.as_str(), "leaf");
    assert_eq!(collected[1].name.as_str(), "mid");
    assert_eq!(collected[2].name.as_str(), "root");
}

#[test]
fn test_atomic_write_safety() {
    let temp = TempDirGuard::new("atomic_write");
    let output_file = temp.path().join(".crux/cmake/CruxDependencies.cmake");

    write_cmake_dependencies_atomically(&output_file, "# initial content\n").unwrap();
    assert_eq!(
        fs::read_to_string(&output_file).unwrap(),
        "# initial content\n"
    );

    write_cmake_dependencies_atomically(&output_file, "# replaced content\n").unwrap();
    assert_eq!(
        fs::read_to_string(&output_file).unwrap(),
        "# replaced content\n"
    );
}

#[test]
fn test_cmake_consumer_build_and_run_end_to_end() {
    let temp = TempDirGuard::new("e2e_consumer");

    // 1. Set up a sample installed package with CMake config
    let pkg_files = &[
        ("include/sample_calc.h", InstalledFileType::Header, b"#pragma once\ninline int calc_add(int a, int b) { return a + b; }\n".as_slice()),
        ("lib/libsample_calc.a", InstalledFileType::StaticLibrary, b"!<arch>\nfake_lib\n".as_slice()),
        ("lib/cmake/SampleCalc/SampleCalcConfig.cmake", InstalledFileType::CMakeModule,
            b"add_library(SampleCalc::SampleCalc INTERFACE IMPORTED)\nset_target_properties(SampleCalc::SampleCalc PROPERTIES INTERFACE_INCLUDE_DIRECTORIES \"${CMAKE_CURRENT_LIST_DIR}/../../../include\")\n".as_slice()),
    ];

    let p = create_mock_installed_package(
        temp.path(),
        "SampleCalc",
        "1.0.0",
        "a1b2c3d4e5f67890123456789abcdef0123456789abcdef0123456789abcdef0",
        BuildProfile::Release,
        pkg_files,
    );

    let generator = CMakeIntegrationGenerator::new(temp.path());
    let res = generator.integrate_packages(&[p]).unwrap();
    assert!(res.output_path.is_file());

    // 2. Set up a consumer C++ CMake project
    let consumer_dir = temp.path().join("consumer");
    fs::create_dir_all(&consumer_dir).unwrap();

    let cmakelists = format!(
        r#"
cmake_minimum_required(VERSION 3.20)
project(sample_consumer CXX)

include("{}/.crux/cmake/CruxDependencies.cmake")

find_package(SampleCalc CONFIG REQUIRED)

add_executable(consumer_app main.cpp)
target_link_libraries(consumer_app PRIVATE SampleCalc::SampleCalc)
"#,
        temp.path().display()
    );

    let main_cpp = r#"
#include <sample_calc.h>
#include <iostream>

int main() {
    int res = calc_add(40, 2);
    std::cout << "Calc result: " << res << std::endl;
    return 0;
}
"#;

    fs::write(consumer_dir.join("CMakeLists.txt"), cmakelists).unwrap();
    fs::write(consumer_dir.join("main.cpp"), main_cpp).unwrap();

    let build_dir = consumer_dir.join("build");

    // Configure consumer
    let cfg = Command::new("cmake")
        .current_dir(&consumer_dir)
        .arg("-B")
        .arg(&build_dir)
        .arg("-S")
        .arg(&consumer_dir)
        .output()
        .expect("failed to configure consumer project");

    assert!(
        cfg.status.success(),
        "configure failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cfg.stdout),
        String::from_utf8_lossy(&cfg.stderr)
    );

    // Verify system library isolation in CMakeCache.txt
    let cache_content = fs::read_to_string(build_dir.join("CMakeCache.txt")).unwrap();
    assert!(
        cache_content.contains("SampleCalc_DIR:PATH="),
        "CMakeCache.txt missing SampleCalc_DIR"
    );
    assert!(
        cache_content.contains(".crux/installed/SampleCalc/1.0.0"),
        "SampleCalc_DIR does not point to Crux installation prefix"
    );

    // Build consumer
    let bld = Command::new("cmake")
        .current_dir(&consumer_dir)
        .arg("--build")
        .arg(&build_dir)
        .output()
        .expect("failed to build consumer project");

    assert!(
        bld.status.success(),
        "build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&bld.stdout),
        String::from_utf8_lossy(&bld.stderr)
    );

    // Run executable
    let exe_path = build_dir.join("consumer_app");
    let run_res = Command::new(&exe_path)
        .current_dir(&consumer_dir)
        .output()
        .expect("failed to execute consumer application");

    assert!(run_res.status.success());
    let stdout = String::from_utf8_lossy(&run_res.stdout);
    assert!(stdout.contains("Calc result: 42"));
}

#[test]
fn test_cmake_consumer_relocation_validation() {
    let temp = TempDirGuard::new("relocation_base with spaces");

    // 1. Setup project with spaces in path
    let project_dir = temp.path().join("my project with spaces");
    fs::create_dir_all(&project_dir).unwrap();

    let pkg_files = &[
        ("include/reloc_calc.h", InstalledFileType::Header, b"#pragma once\ninline int reloc_mul(int a, int b) { return a * b; }\n".as_slice()),
        ("lib/libreloc_calc.a", InstalledFileType::StaticLibrary, b"!<arch>\nfake_lib\n".as_slice()),
        ("lib/cmake/RelocCalc/RelocCalcConfig.cmake", InstalledFileType::CMakeModule,
            b"add_library(RelocCalc::RelocCalc INTERFACE IMPORTED)\nset_target_properties(RelocCalc::RelocCalc PROPERTIES INTERFACE_INCLUDE_DIRECTORIES \"${CMAKE_CURRENT_LIST_DIR}/../../../include\")\n".as_slice()),
    ];

    let p = create_mock_installed_package(
        &project_dir,
        "RelocCalc",
        "2.0.0",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        BuildProfile::Release,
        pkg_files,
    );

    let generator = CMakeIntegrationGenerator::new(&project_dir);
    let res = generator.integrate_packages(&[p]).unwrap();
    assert!(res.output_path.is_file());

    // Setup consumer inside project_dir
    let cmakelists = r#"
cmake_minimum_required(VERSION 3.20)
project(reloc_consumer CXX)

include("${CMAKE_CURRENT_SOURCE_DIR}/.crux/cmake/CruxDependencies.cmake")

find_package(RelocCalc CONFIG REQUIRED)

add_executable(reloc_app main.cpp)
target_link_libraries(reloc_app PRIVATE RelocCalc::RelocCalc)
"#;
    let main_cpp = r#"
#include <reloc_calc.h>
#include <iostream>

int main() {
    std::cout << "Reloc mul: " << reloc_mul(6, 7) << std::endl;
    return 0;
}
"#;

    fs::write(project_dir.join("CMakeLists.txt"), cmakelists).unwrap();
    fs::write(project_dir.join("main.cpp"), main_cpp).unwrap();

    // 2. Relocate complete project to a completely different path
    let relocated_temp = TempDirGuard::new("relocated_target");
    let target_dir = relocated_temp.path().join("new_relocated_location");

    // Copy entire project tree to target_dir
    fn copy_dir_all(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                copy_dir_all(&src_path, &dst_path);
            } else {
                fs::copy(&src_path, &dst_path).unwrap();
            }
        }
    }
    copy_dir_all(&project_dir, &target_dir);

    // 3. Configure and build at target_dir WITHOUT regenerating CruxDependencies.cmake
    let build_dir = target_dir.join("build");

    let cfg = Command::new("cmake")
        .current_dir(&target_dir)
        .arg("-B")
        .arg(&build_dir)
        .arg("-S")
        .arg(&target_dir)
        .output()
        .expect("failed to configure relocated project");

    assert!(
        cfg.status.success(),
        "relocated configure failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cfg.stdout),
        String::from_utf8_lossy(&cfg.stderr)
    );

    let bld = Command::new("cmake")
        .current_dir(&target_dir)
        .arg("--build")
        .arg(&build_dir)
        .output()
        .expect("failed to build relocated project");

    assert!(
        bld.status.success(),
        "relocated build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&bld.stdout),
        String::from_utf8_lossy(&bld.stderr)
    );

    let exe_path = build_dir.join("reloc_app");
    let run_res = Command::new(&exe_path)
        .current_dir(&target_dir)
        .output()
        .expect("failed to execute relocated application");

    assert!(run_res.status.success());
    let stdout = String::from_utf8_lossy(&run_res.stdout);
    assert!(stdout.contains("Reloc mul: 42"));
}

#[test]
fn test_cli_integrate_cmake_with_lockfile() {
    let temp = TempDirGuard::new("cli_integrate");
    let project_dir = temp.path();

    // 1. Create crux.toml
    let manifest = r#"
[package]
name = "my_consumer"
version = "0.1.0"

[dependencies]
fmt = "^11.1"
"#;
    fs::write(project_dir.join("crux.toml"), manifest).unwrap();

    // 2. Create crux.lock with fmt
    let mut lockfile = Lockfile::new();
    let fmt_id = PackageId::new(
        PackageName::new("fmt").unwrap(),
        semver::Version::parse("11.1.1").unwrap(),
    );
    let locked_fmt = LockedPackage::new(
        fmt_id.name().clone(),
        fmt_id.version().clone(),
        PackageSource::default_registry(),
    )
    .with_checksum(
        Checksum::new_sha256("7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5")
            .unwrap(),
    );
    lockfile.packages.push(locked_fmt);
    fs::write(
        project_dir.join("crux.lock"),
        lockfile.to_toml_string().unwrap(),
    )
    .unwrap();

    // 3. Create mock installation
    create_mock_installed_package(
        project_dir,
        "fmt",
        "11.1.1",
        "7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5",
        BuildProfile::Release,
        &[("include/fmt/core.h", InstalledFileType::Header, b"// fmt")],
    );

    // 4. Run `crux integrate cmake` inside project_dir
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = run_integrate_cmake(
        project_dir,
        None,
        BuildProfile::Release,
        false,
        &mut stdout,
        &mut stderr,
    );

    assert!(res.is_ok(), "stderr: {}", String::from_utf8_lossy(&stderr));
    let stdout_str = String::from_utf8_lossy(&stdout);
    assert!(stdout_str.contains("Integrated 1 package into CMake configuration:"));
    assert!(stdout_str.contains("fmt 11.1.1"));
    assert!(
        project_dir
            .join(".crux/cmake/CruxDependencies.cmake")
            .is_file()
    );
}

#[test]
fn test_cli_integrate_cmake_missing_locked_package_error() {
    let temp = TempDirGuard::new("cli_missing_pkg");
    let project_dir = temp.path();

    fs::write(
        project_dir.join("crux.toml"),
        "[package]\nname = \"test_pkg\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let mut lockfile = Lockfile::new();
    let missing_id = PackageId::new(
        PackageName::new("missing-lib").unwrap(),
        semver::Version::parse("2.0.0").unwrap(),
    );
    let locked_missing = LockedPackage::new(
        missing_id.name().clone(),
        missing_id.version().clone(),
        PackageSource::default_registry(),
    );
    lockfile.packages.push(locked_missing);
    fs::write(
        project_dir.join("crux.lock"),
        lockfile.to_toml_string().unwrap(),
    )
    .unwrap();

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = run_integrate_cmake(
        project_dir,
        None,
        BuildProfile::Release,
        false,
        &mut stdout,
        &mut stderr,
    );

    assert!(res.is_err());
    let stderr_str = String::from_utf8_lossy(&stderr);
    assert!(stderr_str.contains(
        "package 'missing-lib' (2.0.0) is locked in crux.lock but not installed on disk"
    ));
    assert!(stderr_str.contains("Run 'crux install' to install dependencies"));
}

#[test]
fn test_cli_integrate_cmake_locked_mode_does_not_modify_lockfile() {
    let temp = TempDirGuard::new("cli_locked_mode");
    let project_dir = temp.path();

    let manifest_str = "[package]\nname = \"test_pkg\"\nversion = \"0.1.0\"\n";
    fs::write(project_dir.join("crux.toml"), manifest_str).unwrap();

    let lockfile = Lockfile::new();
    let lock_path = project_dir.join("crux.lock");
    let original_lock_content = lockfile.to_toml_string().unwrap();
    fs::write(&lock_path, &original_lock_content).unwrap();

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = run_integrate_cmake(
        project_dir,
        None,
        BuildProfile::Release,
        true,
        &mut stdout,
        &mut stderr,
    );

    assert!(res.is_ok(), "stderr: {}", String::from_utf8_lossy(&stderr));
    let after_lock_content = fs::read_to_string(&lock_path).unwrap();
    assert_eq!(original_lock_content, after_lock_content);
}
