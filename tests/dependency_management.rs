//! Integration tests for M21 dependency management and linked application builds.

use crux::build::{AppBuildModel, BuildError, BuildFingerprint, BuildProfile};
use crux::install::{
    CURRENT_INSTALL_METADATA_VERSION, INSTALL_METADATA_FILENAME, InstallMetadata, InstalledFile,
    InstalledFileType,
};
use crux::lock::{LockedPackage, Lockfile, PackageSource};
use crux::package::PackageName;
use crux::registry::{
    GitRegistry, RegistryConfig, ensure_official_registry, official_registry_cache_dir,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "crux_m21_{prefix}_{}_{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn run_cli(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to execute crux")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn project_manifest(name: &str) -> String {
    format!(
        r#"# project header
[project]
name = "{name}"
version = "0.1.0"
language = "cpp"
standard = "c++20"
"#
    )
}

/// Creates a validated fake installation for `fmt` at the requested profile.
fn create_fmt_installation(project_root: &Path, profile: BuildProfile) -> PathBuf {
    let fingerprint_hex = match profile {
        BuildProfile::Debug => "d".repeat(64),
        BuildProfile::Release => "e".repeat(64),
    };
    let install_dir = project_root
        .join(".crux")
        .join("installed")
        .join("fmt")
        .join("11.1.1")
        .join(&fingerprint_hex);
    fs::create_dir_all(install_dir.join("include").join("fmt")).unwrap();
    fs::create_dir_all(install_dir.join("lib")).unwrap();

    let header = b"#pragma once\n#ifdef __cplusplus\nextern \"C\" {\n#endif\nint crux_fmt_dep_value(void);\n#ifdef __cplusplus\n}\n#endif\n";
    fs::write(install_dir.join("include/fmt/fmt_dep.h"), header).unwrap();

    // Build a genuine static archive so the link line is exercised for real.
    let cc = find_tool("cc").or_else(|| find_tool("gcc")).expect("C compiler");
    let ar = find_tool("ar").expect("ar");
    let source = install_dir.join("dep_source.c");
    fs::write(
        &source,
        b"int crux_fmt_dep_value(void) { return 42; }\n",
    )
    .unwrap();
    let object = install_dir.join("dep_source.o");
    let status = Command::new(cc)
        .args(["-c", "-o"])
        .arg(&object)
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile dependency fixture");

    let archive = install_dir.join("lib/libcruxfmtfake.a");
    let status = Command::new(ar)
        .arg("rcs")
        .arg(&archive)
        .arg(&object)
        .status()
        .unwrap();
    assert!(status.success(), "failed to archive dependency fixture");
    fs::remove_file(&source).unwrap();
    fs::remove_file(&object).unwrap();

    let archive_size = fs::metadata(&archive).unwrap().len();
    let fingerprint = BuildFingerprint::new(format!("sha256:{fingerprint_hex}")).unwrap();

    let metadata = InstallMetadata {
        format_version: CURRENT_INSTALL_METADATA_VERSION,
        package_name: "fmt".to_string(),
        package_version: "11.1.1".to_string(),
        fingerprint,
        profile,
        installed_files: vec![
            InstalledFile {
                relative_path: PathBuf::from("include/fmt/fmt_dep.h"),
                file_type: InstalledFileType::Header,
                size_bytes: header.len() as u64,
            },
            InstalledFile {
                relative_path: PathBuf::from("lib/libcruxfmtfake.a"),
                file_type: InstalledFileType::StaticLibrary,
                size_bytes: archive_size,
            },
        ],
        total_size_bytes: header.len() as u64 + archive_size,
        success: true,
        installed_at: None,
    };
    fs::write(
        install_dir.join(INSTALL_METADATA_FILENAME),
        toml::to_string_pretty(&metadata).unwrap(),
    )
    .unwrap();

    install_dir
}

fn write_fmt_lockfile(project_root: &Path) {
    let mut lockfile = Lockfile::new();
    lockfile.packages.push(LockedPackage::new(
        PackageName::new("fmt").unwrap(),
        semver::Version::parse("11.1.1").unwrap(),
        PackageSource::registry("default"),
    ));
    lockfile.write_atomic(project_root).unwrap();
}

fn dependency_project(name: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new(name);
    let project = temp.path.join("demo");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("crux.toml"),
        format!(
            "{}\n[dependencies]\nfmt = \"^11\"\n",
            project_manifest("demo")
        ),
    )
    .unwrap();
    fs::write(
        project.join("src/main.cpp"),
        r#"#include <fmt/fmt_dep.h>
#include <iostream>

int main() {
    std::cout << "value=" << crux_fmt_dep_value() << "\n";
    return 0;
}
"#,
    )
    .unwrap();
    (temp, project)
}

#[test]
fn test_add_rejects_duplicate_and_invalid_specs_without_writes() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = TempDir::new("add_reject");
    let project = temp.path.join("demo");
    fs::create_dir_all(&project).unwrap();
    let manifest = format!(
        "{}\n[dependencies]\nfmt = \"^11\"\n",
        project_manifest("demo")
    );
    fs::write(project.join("crux.toml"), &manifest).unwrap();

    let duplicate = run_cli(&project, &["add", "fmt"]);
    assert!(!duplicate.status.success());
    assert!(stderr(&duplicate).contains("already declared"));
    assert_eq!(fs::read_to_string(project.join("crux.toml")).unwrap(), manifest);
    assert!(!project.join("crux.lock").exists());

    let invalid = run_cli(&project, &["add", "fmt@not-a-version"]);
    assert!(!invalid.status.success());
    assert!(stderr(&invalid).contains("invalid package spec"));
    assert_eq!(fs::read_to_string(project.join("crux.toml")).unwrap(), manifest);
    assert!(!project.join("crux.lock").exists());
}

#[test]
fn test_remove_reports_missing_dependency_without_writes() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = TempDir::new("remove_missing");
    let project = temp.path.join("demo");
    fs::create_dir_all(&project).unwrap();
    let manifest = format!(
        "{}\n[dependencies]\nfmt = \"^11\"\n",
        project_manifest("demo")
    );
    fs::write(project.join("crux.toml"), &manifest).unwrap();

    let output = run_cli(&project, &["remove", "zlib"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("not declared in crux.toml"), "{err}");
    assert!(err.contains("declared dependencies: fmt"), "{err}");
    assert_eq!(fs::read_to_string(project.join("crux.toml")).unwrap(), manifest);
}

#[test]
fn test_remove_only_dependency_preserves_formatting_and_installed_files() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = TempDir::new("remove_success");
    let project = temp.path.join("demo");
    fs::create_dir_all(&project).unwrap();

    let manifest = r#"# top comment
[project]
name = "demo"
version = "0.1.0"
language = "cpp"
standard = "c++20"

# dependency comment
[dependencies]
fmt = "^11"

# trailing comment
"#;
    fs::write(project.join("crux.toml"), manifest).unwrap();
    write_fmt_lockfile(&project);

    // Simulate previously installed files that must survive removal.
    let installed = project.join(".crux/installed/fmt/11.1.1/abc");
    fs::create_dir_all(&installed).unwrap();
    fs::write(installed.join("keep.txt"), "keep").unwrap();

    let output = run_cli(&project, &["remove", "fmt"]);
    assert!(
        output.status.success(),
        "remove failed: {}",
        stderr(&output)
    );

    let updated = fs::read_to_string(project.join("crux.toml")).unwrap();
    assert!(updated.starts_with("# top comment\n"));
    assert!(updated.contains("# dependency comment\n[dependencies]\n"));
    assert!(updated.contains("# trailing comment\n"));
    assert!(!updated.contains("fmt ="));

    let lock = fs::read_to_string(project.join("crux.lock")).unwrap();
    assert!(!lock.contains("name = \"fmt\""));
    assert!(lock.contains("lockfile_version = 1"));

    assert!(
        installed.join("keep.txt").is_file(),
        "removal must never delete installed files"
    );
}

#[test]
fn test_app_build_links_installed_dependency_for_debug_and_release() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if find_tool("cc").is_none() && find_tool("gcc").is_none() {
        return;
    }
    if find_tool("ar").is_none() || find_tool("cmake").is_none() {
        return;
    }

    let (_temp, project) = dependency_project("build_linked");
    create_fmt_installation(&project, BuildProfile::Debug);
    create_fmt_installation(&project, BuildProfile::Release);
    write_fmt_lockfile(&project);

    for (profile, dir) in [("debug", "debug"), ("release", "release")] {
        let output = run_cli(&project, &["build", "--profile", profile, "--verbose"]);
        assert!(
            output.status.success(),
            "build ({profile}) failed:\n{}\n{}",
            stdout(&output),
            stderr(&output)
        );
        let out = stdout(&output);
        assert!(out.contains("Dependencies:  1"), "{out}");
        assert!(out.contains("libcruxfmtfake.a"), "{out}");

        let executable = project.join(".crux/build").join(dir).join("demo");
        assert!(executable.is_file(), "missing executable for {profile}");

        let run_output = Command::new(&executable).output().unwrap();
        assert!(run_output.status.success());
        assert_eq!(String::from_utf8_lossy(&run_output.stdout), "value=42\n");
    }

    // The linked dependency must never cause a root-level CMakeLists.txt.
    assert!(!project.join("CMakeLists.txt").exists());

    let generated =
        fs::read_to_string(project.join(".crux/build/generated/CMakeLists.txt")).unwrap();
    let debug_install = project
        .join(".crux/installed/fmt/11.1.1")
        .join("d".repeat(64));
    let release_install = project
        .join(".crux/installed/fmt/11.1.1")
        .join("e".repeat(64));
    assert!(
        generated.contains(&release_install.display().to_string())
            || generated.contains(&debug_install.display().to_string()),
        "generated CMake does not reference an install directory:\n{generated}"
    );
}

#[test]
fn test_missing_dependency_installation_is_actionable() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_temp, project) = dependency_project("missing_install");
    write_fmt_lockfile(&project);

    let err = AppBuildModel::prepare(&project, BuildProfile::Debug).unwrap_err();
    let message = err.to_string();
    assert!(matches!(err, BuildError::MissingDependency { .. }));
    assert!(message.contains("fmt"), "{message}");
    assert!(message.contains("crux install"), "{message}");
}

#[test]
fn test_registry_config_subdir_root_and_validation() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = TempDir::new("subdir");
    let checkout = temp.path.join("checkout");
    fs::create_dir_all(checkout.join("registry")).unwrap();
    fs::write(checkout.join("registry/registry.toml"), "format_version = 1\nname = \"default\"\n")
        .unwrap();
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&checkout)
        .output()
        .unwrap();

    let config = RegistryConfig {
        url: None,
        path: checkout.clone(),
        subdir: Some(PathBuf::from("registry")),
    };
    assert_eq!(config.registry_root(), checkout.join("registry"));
    config.validate().unwrap();

    let registry = GitRegistry::open_configured(&config).unwrap();
    assert_eq!(registry.name(), "default");

    let escaping = RegistryConfig {
        url: None,
        path: checkout.clone(),
        subdir: Some(PathBuf::from("../escape")),
    };
    assert!(escaping.validate().is_err());
    assert!(GitRegistry::open_configured(&escaping).is_err());
}

#[test]
fn test_official_registry_bootstrap_uses_cache_and_reuses_config() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = TempDir::new("official_cache");
    let registries = temp.path.join("registries");
    let checkout = registries.join("official");
    fs::create_dir_all(&checkout).unwrap();

    // Copy the bundled registry contents into the expected cached layout.
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("registry");
    copy_dir(&bundled, &checkout.join("registry")).unwrap();
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&checkout)
        .output()
        .unwrap();

    let project = temp.path.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("crux.toml"), project_manifest("demo")).unwrap();

    unsafe {
        std::env::set_var("CRUX_REGISTRIES_DIR", &registries);
        std::env::set_var("CRUX_CONFIG_PATH", project.join(".crux/config.toml"));
    }

    let (name, config) = ensure_official_registry(Some(&project)).unwrap();
    assert_eq!(name, "default");
    assert_eq!(config.path, checkout);
    assert_eq!(config.subdir.as_deref(), Some(Path::new("registry")));
    assert_eq!(config.registry_root(), checkout.join("registry"));
    assert_eq!(official_registry_cache_dir(), registries);
    assert!(project.join(".crux/config.toml").is_file());

    // Second call reuses the persisted configuration without touching the network.
    let (name_again, config_again) = ensure_official_registry(Some(&project)).unwrap();
    assert_eq!(name_again, "default");
    assert_eq!(config_again, config);

    unsafe {
        std::env::remove_var("CRUX_REGISTRIES_DIR");
        std::env::remove_var("CRUX_CONFIG_PATH");
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
