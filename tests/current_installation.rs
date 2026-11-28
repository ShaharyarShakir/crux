//! Current-installation selection and local path dependency rebuilds.
//!
//! Selection tests craft installations on disk. Rebuild tests drive the real
//! `crux` binary with CMake and a C compiler, using only temporary local
//! packages, Git repositories, and an isolated registry.

mod common;

use common::{configure_registry, init_fixture_registry};
use crux::build::{BuildFingerprint, BuildProfile};
use crux::install::{
    CURRENT_INSTALL_METADATA_VERSION, InstallMetadata, InstalledFile, InstalledFileType,
};
use crux::integrate::{IntegrationError, validate_package_installation};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("crux_current_{prefix}_{}_{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

fn digest(seed: char) -> String {
    seed.to_string().repeat(64)
}

fn version_dir(project: &Path, name: &str) -> PathBuf {
    project.join(".crux/installed").join(name).join("1.0.0")
}

fn install(project: &Path, seed: char, profile: BuildProfile) -> PathBuf {
    let dir = version_dir(project, "demo").join(digest(seed));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("demo.h"), "x").unwrap();
    let metadata = InstallMetadata {
        format_version: CURRENT_INSTALL_METADATA_VERSION,
        package_name: "demo".to_string(),
        package_version: "1.0.0".to_string(),
        fingerprint: BuildFingerprint::new(format!("sha256:{}", digest(seed))).unwrap(),
        profile,
        installed_files: vec![InstalledFile {
            relative_path: PathBuf::from("demo.h"),
            file_type: InstalledFileType::Header,
            size_bytes: 1,
        }],
        total_size_bytes: 1,
        success: true,
        installed_at: None,
    };
    fs::write(
        dir.join("installed.toml"),
        toml::to_string_pretty(&metadata).unwrap(),
    )
    .unwrap();
    dir
}

fn record_text(seed: char, profile: &str) -> String {
    format!(
        "format_version = 1\npackage_name = \"demo\"\npackage_version = \"1.0.0\"\n\
         profile = \"{profile}\"\nfingerprint = \"sha256:{}\"\n",
        digest(seed)
    )
}

fn record(project: &Path, seed: char, profile: &str) {
    fs::write(
        version_dir(project, "demo").join(format!("current-{profile}.toml")),
        record_text(seed, profile),
    )
    .unwrap();
}

fn selected(project: &Path, profile: BuildProfile) -> Result<PathBuf, IntegrationError> {
    validate_package_installation(project, "demo", "1.0.0", profile).map(|v| v.install_dir)
}

fn corrupt_reason(result: Result<PathBuf, IntegrationError>) -> String {
    match result {
        Err(IntegrationError::CorruptInstallation { reason, .. }) => reason,
        other => panic!("expected CorruptInstallation, got {other:?}"),
    }
}

#[test]
fn no_installation_is_reported_as_not_installed() {
    let temp = TempDir::new("none");
    assert!(matches!(
        selected(&temp.path, BuildProfile::Debug),
        Err(IntegrationError::PackageNotInstalled { .. })
    ));
}

#[test]
fn a_single_legacy_installation_is_accepted() {
    let temp = TempDir::new("single");
    let only = install(&temp.path, 'b', BuildProfile::Debug);
    assert_eq!(selected(&temp.path, BuildProfile::Debug).unwrap(), only);

    // A broken sibling does not make the one valid installation ambiguous.
    let broken = version_dir(&temp.path, "demo").join(digest('a'));
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("installed.toml"), "not toml {").unwrap();
    assert_eq!(selected(&temp.path, BuildProfile::Debug).unwrap(), only);
}

#[test]
fn the_record_selects_among_several_installations() {
    let temp = TempDir::new("record");
    install(&temp.path, 'a', BuildProfile::Debug);
    let middle = install(&temp.path, 'b', BuildProfile::Debug);
    install(&temp.path, 'c', BuildProfile::Debug);
    record(&temp.path, 'b', "debug");
    assert_eq!(selected(&temp.path, BuildProfile::Debug).unwrap(), middle);
}

#[test]
fn a_record_naming_a_missing_installation_selects_nothing() {
    let temp = TempDir::new("missing_target");
    // Exactly one other valid installation exists: it must not be picked up.
    install(&temp.path, 'a', BuildProfile::Debug);
    record(&temp.path, 'f', "debug");
    let reason = corrupt_reason(selected(&temp.path, BuildProfile::Debug));
    assert!(reason.contains("current-installation record"), "{reason}");
}

#[test]
fn a_record_naming_an_invalid_installation_selects_nothing() {
    let temp = TempDir::new("invalid_target");
    install(&temp.path, 'a', BuildProfile::Debug);
    let current = install(&temp.path, 'b', BuildProfile::Debug);
    record(&temp.path, 'b', "debug");
    fs::remove_file(current.join("demo.h")).unwrap();
    let reason = corrupt_reason(selected(&temp.path, BuildProfile::Debug));
    assert!(reason.contains("missing on disk"), "{reason}");
}

#[test]
fn several_legacy_installations_without_a_record_are_ambiguous() {
    let temp = TempDir::new("ambiguous");
    install(&temp.path, 'a', BuildProfile::Debug);
    install(&temp.path, 'b', BuildProfile::Debug);
    let reason = corrupt_reason(selected(&temp.path, BuildProfile::Debug));
    assert!(reason.contains("none is recorded as current"), "{reason}");
}

#[test]
fn invalid_or_partial_records_select_nothing_and_leftover_temp_files_are_ignored() {
    let temp = TempDir::new("partial");
    let only = install(&temp.path, 'a', BuildProfile::Debug);
    let versions = version_dir(&temp.path, "demo");

    // What an interrupted atomic write leaves behind: only the temporary file.
    fs::write(versions.join(".current-debug.toml.tmp.1.2.3"), "format_ver").unwrap();
    assert_eq!(selected(&temp.path, BuildProfile::Debug).unwrap(), only);

    let full = record_text('a', "debug");
    let wrong_profile = record_text('a', "release");
    let traversal = full.replace(&digest('a'), "../../../../etc");
    for bad in ["", &full[..full.len() / 2], &wrong_profile, &traversal] {
        fs::write(versions.join("current-debug.toml"), bad).unwrap();
        corrupt_reason(selected(&temp.path, BuildProfile::Debug));
    }
}

#[test]
fn each_profile_has_its_own_current_installation() {
    let temp = TempDir::new("profiles");
    install(&temp.path, 'a', BuildProfile::Debug);
    let debug = install(&temp.path, 'b', BuildProfile::Debug);
    install(&temp.path, 'c', BuildProfile::Release);
    let release = install(&temp.path, 'd', BuildProfile::Release);
    record(&temp.path, 'b', "debug");
    record(&temp.path, 'd', "release");

    assert_eq!(selected(&temp.path, BuildProfile::Debug).unwrap(), debug);
    assert_eq!(
        selected(&temp.path, BuildProfile::Release).unwrap(),
        release
    );
    assert_eq!(selected(&temp.path, BuildProfile::Debug).unwrap(), debug);
}

// ---------------------------------------------------------------------------
// End-to-end rebuilds
// ---------------------------------------------------------------------------

/// A static C library whose `<name>_value()` returns `value`.
fn write_library(dir: &Path, name: &str, value: u32, dependencies: &str) {
    fs::create_dir_all(dir.join("include")).unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("package.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n\n[build]\nsystem = \"cmake\"\n\n[dependencies]\n{dependencies}"
        ),
    )
    .unwrap();
    fs::write(
        dir.join("include").join(format!("{name}.h")),
        format!("#pragma once\nint {name}_value(void);\n"),
    )
    .unwrap();
    set_value(dir, name, value);
    fs::write(
        dir.join("CMakeLists.txt"),
        format!(
            "cmake_minimum_required(VERSION 3.16)\nproject({name} LANGUAGES C)\nadd_library({name} STATIC src/lib.c)\ninstall(TARGETS {name} ARCHIVE DESTINATION lib)\ninstall(FILES include/{name}.h DESTINATION include)\n"
        ),
    )
    .unwrap();
}

fn set_value(dir: &Path, name: &str, value: u32) {
    fs::write(
        dir.join("src/lib.c"),
        format!("int {name}_value(void) {{ return {value}; }}\n"),
    )
    .unwrap();
}

/// An application printing `name=value` for every library in `libraries`.
fn write_app(project: &Path, dependencies: &str, libraries: &[&str]) {
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("crux.toml"),
        format!(
            "[project]\nname = \"app\"\nversion = \"0.1.0\"\nlanguage = \"c\"\n\n[dependencies]\n{dependencies}"
        ),
    )
    .unwrap();
    let mut main = String::from("#include <stdio.h>\n");
    for lib in libraries {
        main.push_str(&format!("#include <{lib}.h>\n"));
    }
    main.push_str("int main(void) {\n");
    for lib in libraries {
        main.push_str(&format!("    printf(\"{lib}=%d\\n\", {lib}_value());\n"));
    }
    main.push_str("    return 0;\n}\n");
    fs::write(project.join("src/main.c"), main).unwrap();
}

struct Workspace {
    temp: TempDir,
    project: PathBuf,
    registry: PathBuf,
}

impl Workspace {
    fn new(prefix: &str) -> Self {
        let temp = TempDir::new(prefix);
        let project = temp.path.join("app");
        let registry = init_fixture_registry(&temp.path.join("registry"));
        Self {
            temp,
            project,
            registry,
        }
    }

    /// A workspace whose application depends on the path library `greet`.
    fn with_path_dependency(prefix: &str) -> (Self, PathBuf) {
        let ws = Self::new(prefix);
        let greet = ws.temp.path.join("greet");
        write_library(&greet, "greet", 100, "");
        write_app(&ws.project, "greet = { path = \"../greet\" }\n", &["greet"]);
        (ws, greet)
    }

    /// Runs `crux`, requiring success, and returns stdout and stderr.
    fn crux(&self, args: &[&str]) -> String {
        let output = self.try_crux(args);
        assert!(output.status.success(), "{args:?}\n{}", text(&output));
        text(&output)
    }

    fn try_crux(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crux"));
        configure_registry(
            &mut command,
            &self.registry,
            &self.project.join(".crux/config.toml"),
        );
        command
            .env("CRUX_HOME", self.temp.path.join("home"))
            .args(args)
            .current_dir(&self.project)
            .output()
            .unwrap()
    }

    fn installs(&self, name: &str) -> usize {
        fs::read_dir(version_dir(&self.project, name))
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().is_dir())
            .count()
    }

    fn record(&self, name: &str, profile: &str) -> String {
        fs::read_to_string(version_dir(&self.project, name).join(format!("current-{profile}.toml")))
            .unwrap()
    }

    fn lockfile(&self) -> String {
        fs::read_to_string(self.project.join("crux.lock")).unwrap()
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_has(output: &str, needle: &str) {
    assert!(output.contains(needle), "missing {needle:?} in:\n{output}");
}

fn assert_lacks(output: &str, needle: &str) {
    assert!(
        !output.contains(needle),
        "unexpected {needle:?} in:\n{output}"
    );
}

#[test]
fn run_rebuilds_an_edited_path_dependency_and_only_then() {
    let (ws, greet) = Workspace::with_path_dependency("rebuild");
    assert_has(&ws.crux(&["run"]), "greet=100");
    let lockfile = ws.lockfile();

    // Several successive edits: whatever the fingerprints sort like, the app
    // must link the installation of the source as it is now.
    for (edits, value) in (101..=105).enumerate() {
        set_value(&greet, "greet", value);
        let output = ws.crux(&["run"]);
        assert_has(&output, "changed: greet 1.0.0 (path+../greet)");
        assert_has(&output, &format!("greet={value}"));
        assert_eq!(ws.installs("greet"), edits + 2);
    }

    let record = ws.record("greet", "debug");
    let again = ws.crux(&["run"]);
    assert_has(&again, "Dependencies:  1 installed [debug]");
    assert_has(&again, "greet=105");
    assert_lacks(&again, "changed:");
    assert_lacks(&again, "Installing");
    assert_eq!(ws.installs("greet"), 6);
    assert_eq!(ws.record("greet", "debug"), record);

    // Reverting reuses the older installation and makes it current again.
    set_value(&greet, "greet", 100);
    let reverted = ws.crux(&["run"]);
    assert_has(&reverted, "[cached]    greet 1.0.0");
    assert_has(&reverted, "greet=100");
    assert_eq!(ws.installs("greet"), 6);

    assert_eq!(ws.lockfile(), lockfile, "source edits must not relock");
}

#[test]
fn build_rebuilds_an_edited_path_dependency() {
    let (ws, greet) = Workspace::with_path_dependency("build");
    ws.crux(&["build"]);
    set_value(&greet, "greet", 200);
    assert_has(&ws.crux(&["build"]), "changed: greet 1.0.0");

    let app = ws.project.join(".crux/build/debug/bin/app");
    let app = if app.is_file() {
        app
    } else {
        ws.project.join(".crux/build/debug/app")
    };
    let output = Command::new(&app).output().unwrap();
    assert_has(&text(&output), "greet=200");
}

#[test]
fn profiles_track_source_changes_independently() {
    let (ws, greet) = Workspace::with_path_dependency("profile_switch");
    assert_has(&ws.crux(&["run"]), "greet=100");
    assert_has(&ws.crux(&["run", "--release"]), "greet=100");

    set_value(&greet, "greet", 101);
    assert_has(&ws.crux(&["run"]), "greet=101");
    let release = ws.crux(&["run", "--release"]);
    assert_has(&release, "changed: greet");
    assert_has(&release, "greet=101");

    let debug = ws.crux(&["run"]);
    assert_has(&debug, "greet=101");
    assert_lacks(&debug, "changed:");
}

#[test]
fn legacy_stale_and_corrupt_records_are_recovered_by_reinstalling() {
    let (ws, greet) = Workspace::with_path_dependency("recover");
    ws.crux(&["run"]);
    let record = version_dir(&ws.project, "greet").join("current-debug.toml");

    // One installation without a record (pre-record layout): migrated.
    fs::remove_file(&record).unwrap();
    let migrated = ws.crux(&["run"]);
    assert_has(&migrated, "unrecorded: greet");
    assert_has(&migrated, "greet=100");
    assert_has(&ws.record("greet", "debug"), "source_checksum");

    // Several installations without a record: never guessed.
    set_value(&greet, "greet", 101);
    ws.crux(&["run"]);
    fs::remove_file(&record).unwrap();
    let ambiguous = ws.crux(&["run"]);
    assert_has(&ambiguous, "none is recorded as current");
    assert_has(&ambiguous, "greet=101");

    // Truncated record.
    fs::write(&record, "format_version = 1\npack").unwrap();
    let corrupt = ws.crux(&["run"]);
    assert_has(&corrupt, "invalid current-installation record");
    assert_has(&corrupt, "greet=101");

    // Record pointing at a deleted installation.
    let current = ws.record("greet", "debug");
    let digest = current
        .lines()
        .find_map(|line| line.strip_prefix("fingerprint = \"sha256:"))
        .unwrap()
        .trim_end_matches('"');
    fs::remove_dir_all(version_dir(&ws.project, "greet").join(digest)).unwrap();
    let stale = ws.crux(&["run"]);
    assert_has(&stale, "there is no debug installation there");
    assert_has(&stale, "greet=101");
}

#[test]
fn a_failed_dependency_rebuild_does_not_run_the_old_code() {
    let (ws, greet) = Workspace::with_path_dependency("failure");
    ws.crux(&["run"]);

    fs::write(greet.join("src/lib.c"), "this is not C\n").unwrap();
    for _ in 0..2 {
        let output = ws.try_crux(&["run"]);
        assert!(!output.status.success(), "{}", text(&output));
        assert_lacks(&text(&output), "greet=100");
    }

    set_value(&greet, "greet", 101);
    assert_has(&ws.crux(&["run"]), "greet=101");
}

#[test]
fn only_edited_path_packages_and_their_dependents_are_reinstalled() {
    let ws = Workspace::new("selective");
    let repo = ws.temp.path.join("pinned-git");
    write_library(&repo, "pinned", 7, "");
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "Crux Test")
            .env("GIT_AUTHOR_EMAIL", "crux-test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Crux Test")
            .env("GIT_COMMITTER_EMAIL", "crux-test@example.invalid")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "initial"]);

    // `leaf` is reached through a nested relative path declared by `outer`.
    let leaf = ws.temp.path.join("leaf");
    write_library(&leaf, "leaf", 1, "");
    write_library(
        &ws.temp.path.join("outer"),
        "outer",
        2,
        "leaf = { path = \"../leaf\" }\n",
    );
    write_app(
        &ws.project,
        &format!(
            "outer = {{ path = \"../outer\" }}\npinned = {{ git = \"{}\", branch = \"main\" }}\n",
            repo.display()
        ),
        &["leaf", "outer", "pinned"],
    );
    assert_has(&ws.crux(&["run"]), "pinned=7");
    let pinned_record = ws.record("pinned", "debug");

    // The Git branch moves, but the lockfile still pins the first commit.
    set_value(&repo, "pinned", 8);
    git(&["commit", "-q", "-am", "moved"]);
    set_value(&leaf, "leaf", 3);

    let output = ws.crux(&["run"]);
    assert_has(
        &output,
        "Installing 2 required dependency package(s) [debug]",
    );
    assert_has(&output, "changed: leaf");
    assert_lacks(&output, "changed: outer");
    assert_has(&output, "outdated: outer 1.0.0");
    assert_lacks(&output, "pinned 1.0.0");
    assert_has(&output, "leaf=3");
    assert_has(&output, "outer=2");
    assert_has(&output, "pinned=7");
    // `outer` is unchanged but depends on `leaf`, so it has a new fingerprint.
    assert_eq!(ws.installs("outer"), 2);
    assert_eq!(ws.installs("pinned"), 1);
    assert_eq!(ws.record("pinned", "debug"), pinned_record);
}

#[test]
fn integrate_records_a_legacy_installation_and_refuses_ambiguity() {
    let (ws, greet) = Workspace::with_path_dependency("integrate");
    ws.crux(&["run"]);
    let record = version_dir(&ws.project, "greet").join("current-debug.toml");

    fs::remove_file(&record).unwrap();
    ws.crux(&["integrate", "cmake", "--profile", "debug"]);
    assert!(record.is_file());

    set_value(&greet, "greet", 101);
    ws.crux(&["run"]);
    fs::remove_file(&record).unwrap();
    let output = ws.try_crux(&["integrate", "cmake", "--profile", "debug"]);
    assert!(!output.status.success());
    assert_has(&text(&output), "none is recorded as current");
}

/// The reproduction from the 2026-11-27 architecture assessment, section 3.1.
#[test]
fn explicit_lock_and_install_cycles_run_the_latest_source() {
    let (ws, greet) = Workspace::with_path_dependency("assessment");
    for value in 100..=105 {
        set_value(&greet, "greet", value);
        ws.crux(&["lock"]);
        ws.crux(&["install", "--profile", "debug"]);
        let output = ws.crux(&["run"]);
        assert_has(&output, &format!("greet={value}"));
        assert_lacks(&output, "changed:");
    }
}
