//! Offline integration coverage for re-lock checksum integrity
//! (2026-11-21 remediation, owner decisions 1 and 3).
//!
//! Registry archive packages are declared in an isolated fixture registry.
//! Locking never downloads archives; `crux fetch` failures asserted here occur
//! before any network access. No external network service is required.

mod common;

use common::{configure_registry, init_fixture_registry};
use crux::lock::Lockfile;
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
        let path = std::env::temp_dir().join(format!(
            "crux_lock_integrity_{prefix}_{}_{id}",
            std::process::id()
        ));
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

fn digest(c: char) -> String {
    c.to_string().repeat(64)
}

/// Writes (or overwrites) an archive-sourced package into the fixture registry.
fn write_archive_package(registry: &Path, name: &str, version: &str, digest: &str) {
    let dir = registry.join("packages").join(name).join(version);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("package.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\n\
             description = \"Integrity fixture\"\nlicense = \"MIT\"\n\n\
             [source]\ntype = \"archive\"\n\
             url = \"https://example.invalid/{name}-{version}.tar.gz\"\n\
             checksum = \"sha256:{digest}\"\n\n\
             [build]\nsystem = \"cmake\"\n\n[dependencies]\n"
        ),
    )
    .unwrap();
}

fn write_manifest(project: &Path, dependencies: &str) {
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("crux.toml"),
        format!(
            "[project]\nname = \"integrity-app\"\nversion = \"0.1.0\"\nlanguage = \"cpp\"\n\n[dependencies]\n{dependencies}"
        ),
    )
    .unwrap();
    fs::write(project.join("src/main.cpp"), "int main() { return 0; }\n").unwrap();
}

fn run_cli(project: &Path, registry: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_crux"));
    configure_registry(
        &mut command,
        registry,
        &project.join(".crux").join("config.toml"),
    );
    command.args(args).current_dir(project).output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn locked_checksum(project: &Path, name: &str) -> String {
    Lockfile::load_from_path(&project.join("crux.lock"))
        .unwrap()
        .packages
        .iter()
        .find(|pkg| pkg.name.as_str() == name)
        .and_then(|pkg| pkg.checksum.as_ref().map(ToString::to_string))
        .unwrap_or_else(|| panic!("package '{name}' has no locked checksum"))
}

fn locked_version(project: &Path, name: &str) -> String {
    Lockfile::load_from_path(&project.join("crux.lock"))
        .unwrap()
        .packages
        .iter()
        .find(|pkg| pkg.name.as_str() == name)
        .map(|pkg| pkg.version.to_string())
        .unwrap()
}

struct ArchiveFixture {
    _temp: TempDir,
    project: PathBuf,
    registry: PathBuf,
}

/// A project locked to registry archive package `arc 1.0.0` (checksum `a…`).
fn locked_archive_project(prefix: &str) -> ArchiveFixture {
    let temp = TempDir::new(prefix);
    let registry = init_fixture_registry(&temp.path.join("registry"));
    write_archive_package(&registry, "arc", "1.0.0", &digest('a'));
    write_archive_package(&registry, "arc", "1.1.0", &digest('c'));
    let project = temp.path.join("project");
    write_manifest(&project, "arc = \"=1.0.0\"\n");
    let lock = run_cli(&project, &registry, &["lock"]);
    assert_success(&lock);
    assert_eq!(
        locked_checksum(&project, "arc"),
        format!("sha256:{}", digest('a'))
    );
    ArchiveFixture {
        _temp: temp,
        project,
        registry,
    }
}

/// Test 1: a normal re-lock with an unchanged checksum is a byte-identical no-op.
#[test]
fn normal_lock_with_unchanged_checksum_is_stable() {
    let fx = locked_archive_project("unchanged");
    let before = fs::read(fx.project.join("crux.lock")).unwrap();
    let relock = run_cli(&fx.project, &fx.registry, &["lock"]);
    assert_success(&relock);
    assert_eq!(before, fs::read(fx.project.join("crux.lock")).unwrap());
    assert!(!stderr(&relock).contains("warning"), "{}", stderr(&relock));
}

/// Test 2: the registry reports a different checksum for the same version and
/// source. A normal `crux lock` must refuse and leave crux.lock untouched, and
/// fetch must keep enforcing the locked checksum.
#[test]
fn normal_lock_rejects_changed_checksum_and_leaves_lockfile_untouched() {
    let fx = locked_archive_project("changed");
    let before = fs::read(fx.project.join("crux.lock")).unwrap();
    write_archive_package(&fx.registry, "arc", "1.0.0", &digest('b'));

    let relock = run_cli(&fx.project, &fx.registry, &["lock"]);
    assert!(!relock.status.success(), "stdout:\n{}", stdout(&relock));
    let err = stderr(&relock);
    assert!(err.contains("integrity error"), "{err}");
    assert!(err.contains("arc 1.0.0"), "{err}");
    assert!(err.contains(&format!("sha256:{}", digest('a'))), "{err}");
    assert!(err.contains(&format!("sha256:{}", digest('b'))), "{err}");
    assert!(err.contains("crux lock --update"), "{err}");
    assert_eq!(before, fs::read(fx.project.join("crux.lock")).unwrap());

    // Acquisition still enforces the locked checksum (before any download).
    let fetch = run_cli(&fx.project, &fx.registry, &["fetch"]);
    assert!(!fetch.status.success());
    assert!(
        stderr(&fetch).contains("does not match crux.lock"),
        "{}",
        stderr(&fetch)
    );

    // Dependency edits re-lock too and must not smuggle the change in.
    let manifest_before = fs::read(fx.project.join("crux.toml")).unwrap();
    let add = run_cli(&fx.project, &fx.registry, &["add", "zlib@^1.3"]);
    assert!(!add.status.success(), "stdout:\n{}", stdout(&add));
    assert!(stderr(&add).contains("integrity error"), "{}", stderr(&add));
    assert_eq!(
        manifest_before,
        fs::read(fx.project.join("crux.toml")).unwrap()
    );
    assert_eq!(before, fs::read(fx.project.join("crux.lock")).unwrap());
}

/// Test 3: an explicit `crux lock --update` accepts the change and reports the
/// package, the old and new checksums, and that the update was explicit.
#[test]
fn explicit_update_accepts_and_reports_changed_checksum() {
    let fx = locked_archive_project("explicit_update");
    write_archive_package(&fx.registry, "arc", "1.0.0", &digest('b'));

    let update = run_cli(&fx.project, &fx.registry, &["lock", "--update"]);
    assert_success(&update);
    let err = stderr(&update);
    assert!(err.contains("accepted a changed checksum"), "{err}");
    assert!(err.contains("'arc 1.0.0'"), "{err}");
    assert!(
        err.contains(&format!("old: sha256:{}", digest('a'))),
        "{err}"
    );
    assert!(
        err.contains(&format!("new: sha256:{}", digest('b'))),
        "{err}"
    );
    assert!(err.contains("requested explicitly"), "{err}");
    assert_eq!(
        locked_checksum(&fx.project, "arc"),
        format!("sha256:{}", digest('b'))
    );

    // Once accepted, normal re-locks are stable again.
    let relock = run_cli(&fx.project, &fx.registry, &["lock"]);
    assert_success(&relock);
    assert!(!stderr(&relock).contains("accepted a changed checksum"));
}

/// Test 4: a legitimate version upgrade records the new version's checksum
/// with a normal `crux lock` and is not treated as an integrity change.
#[test]
fn legitimate_version_upgrade_is_accepted_by_normal_lock() {
    let fx = locked_archive_project("upgrade");
    write_manifest(&fx.project, "arc = \"^1.1\"\n");
    let relock = run_cli(&fx.project, &fx.registry, &["lock"]);
    assert_success(&relock);
    assert_eq!(locked_version(&fx.project, "arc"), "1.1.0");
    assert_eq!(
        locked_checksum(&fx.project, "arc"),
        format!("sha256:{}", digest('c'))
    );
    assert!(
        !stderr(&relock).contains("integrity"),
        "{}",
        stderr(&relock)
    );
}

fn write_path_package(dir: &Path, name: &str, version: &str, body: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("package.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n\n[dependencies]\n"),
    )
    .unwrap();
    fs::write(dir.join("src/lib.c"), body).unwrap();
}

/// Owner decision 1: local path dependencies stay mutable. Editing the live
/// source never causes a lockfile checksum failure; fetch builds from a fresh
/// snapshot of the edited tree and `crux lock` refreshes the checksum.
#[test]
fn local_path_edits_do_not_cause_lockfile_checksum_failures() {
    let temp = TempDir::new("path_mutable");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let local = temp.path.join("local-lib");
    write_path_package(&local, "local-lib", "1.0.0", "int v(void) { return 1; }\n");
    let project = temp.path.join("project");
    write_manifest(&project, "local-lib = { path = \"../local-lib\" }\n");

    assert_success(&run_cli(&project, &registry, &["lock"]));
    let first_checksum = locked_checksum(&project, "local-lib");

    // Active development: edit the dependency without re-locking.
    write_path_package(&local, "local-lib", "1.0.0", "int v(void) { return 2; }\n");

    let fetch = run_cli(&project, &registry, &["fetch"]);
    assert_success(&fetch);
    assert!(!stderr(&fetch).contains("does not match crux.lock"));
    let snapshot = stdout(&fetch)
        .lines()
        .find_map(|line| line.trim().strip_prefix("local-lib 1.0.0 -> "))
        .map(PathBuf::from)
        .expect("fetch reports the acquired snapshot directory");
    assert_ne!(
        fs::canonicalize(&snapshot).unwrap(),
        fs::canonicalize(&local).unwrap(),
        "builds use a cache snapshot, not the live directory"
    );
    assert!(
        fs::read_to_string(snapshot.join("src/lib.c"))
            .unwrap()
            .contains("return 2"),
        "the snapshot reflects the edited source"
    );

    // A normal re-lock refreshes the mutable checksum without an integrity error.
    let relock = run_cli(&project, &registry, &["lock"]);
    assert_success(&relock);
    assert!(
        !stderr(&relock).contains("integrity"),
        "{}",
        stderr(&relock)
    );
    assert_ne!(locked_checksum(&project, "local-lib"), first_checksum);
}

// ---------------------------------------------------------------------------
// `crux lock --check` registry checksum-drift reporting.
// ---------------------------------------------------------------------------

/// Byte snapshot of the files `crux lock --check` must never modify.
struct ProjectSnapshot {
    lock: Vec<u8>,
    manifest: Vec<u8>,
}

impl ProjectSnapshot {
    fn take(project: &Path) -> Self {
        Self {
            lock: fs::read(project.join("crux.lock")).unwrap(),
            manifest: fs::read(project.join("crux.toml")).unwrap(),
        }
    }

    fn assert_unchanged(&self, project: &Path) {
        assert_eq!(
            self.lock,
            fs::read(project.join("crux.lock")).unwrap(),
            "crux.lock must remain byte-identical"
        );
        assert_eq!(
            self.manifest,
            fs::read(project.join("crux.toml")).unwrap(),
            "crux.toml must remain unchanged"
        );
    }
}

/// Points the project at an isolated cache so tests can prove `--check`
/// downloads and extracts nothing.
fn isolate_cache(project: &Path) -> PathBuf {
    let cache = project.parent().unwrap().join("cache");
    fs::create_dir_all(project.join(".crux")).unwrap();
    fs::write(
        project.join(".crux/config.toml"),
        format!("cache_dir = \"{}\"\n", cache.display()),
    )
    .unwrap();
    cache
}

fn assert_cache_untouched(cache: &Path) {
    fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files_under(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    // The cache may pre-create empty layout directories; no file may appear.
    for sub in ["archives", "sources"] {
        let mut files = Vec::new();
        files_under(&cache.join(sub), &mut files);
        assert!(
            files.is_empty(),
            "--check must not download or extract: {files:?}"
        );
    }
}

fn lock_check(fx: &ArchiveFixture) -> Output {
    run_cli(&fx.project, &fx.registry, &["lock", "--check"])
}

#[test]
fn lock_check_reports_matching_registry_checksums() {
    let fx = locked_archive_project("check_match");
    let cache = isolate_cache(&fx.project);
    let snapshot = ProjectSnapshot::take(&fx.project);

    let check = lock_check(&fx);
    assert_success(&check);
    let out = stdout(&check);
    assert!(
        out.contains("Checksums: compared with registry 'default'"),
        "{out}"
    );
    assert!(out.contains("ok") && out.contains("arc 1.0.0"), "{out}");
    assert!(
        out.contains("Checksum audit: 1 ok, 0 warning(s), 0 error(s)"),
        "{out}"
    );
    assert!(
        out.contains("only locally available registry metadata"),
        "{out}"
    );
    assert!(out.contains("Status: Valid (synchronized)"), "{out}");
    snapshot.assert_unchanged(&fx.project);
    assert_cache_untouched(&cache);

    // Deterministic: a second run produces identical output.
    assert_eq!(out, stdout(&lock_check(&fx)));
}

#[test]
fn lock_check_reports_registry_checksum_drift_without_modifying_anything() {
    let fx = locked_archive_project("check_drift");
    let cache = isolate_cache(&fx.project);
    write_archive_package(&fx.registry, "arc", "1.0.0", &digest('b'));
    let snapshot = ProjectSnapshot::take(&fx.project);

    let check = lock_check(&fx);
    assert!(!check.status.success(), "stdout:\n{}", stdout(&check));
    let out = stdout(&check);
    let err = stderr(&check);
    assert!(
        out.contains("checksum-drift") && out.contains("arc 1.0.0"),
        "{out}"
    );
    assert!(out.contains("0 ok, 0 warning(s), 1 error(s)"), "{out}");
    assert!(
        err.contains("Status: Invalid (checksum audit failed)"),
        "{err}"
    );
    assert!(err.contains("checksum drift for 'arc 1.0.0'"), "{err}");
    assert!(err.contains(&format!("sha256:{}", digest('a'))), "{err}");
    assert!(err.contains(&format!("sha256:{}", digest('b'))), "{err}");
    assert!(err.contains("crux lock --update"), "{err}");
    assert!(err.contains("crux.lock was not modified"), "{err}");
    // Drift is not reported as a missing package/version or a source change.
    assert!(!err.contains("no longer provides") && !err.contains("source identity change"));
    snapshot.assert_unchanged(&fx.project);
    assert_cache_untouched(&cache);

    // --check never accepts the change; the lock still records the old digest.
    assert_eq!(
        locked_checksum(&fx.project, "arc"),
        format!("sha256:{}", digest('a'))
    );
}

#[test]
fn lock_check_distinguishes_missing_registry_version_and_package() {
    let fx = locked_archive_project("check_missing");
    isolate_cache(&fx.project);
    let snapshot = ProjectSnapshot::take(&fx.project);

    // The locked version disappeared from the registry.
    fs::remove_dir_all(fx.registry.join("packages/arc/1.0.0")).unwrap();
    let check = lock_check(&fx);
    assert!(!check.status.success());
    assert!(
        stdout(&check).contains("version-missing"),
        "{}",
        stdout(&check)
    );
    let err = stderr(&check);
    assert!(
        err.contains("does not provide version 1.0.0 (available: 1.1.0)"),
        "{err}"
    );
    assert!(err.contains("crux registry update default"), "{err}");
    assert!(!err.contains("checksum drift"), "{err}");
    snapshot.assert_unchanged(&fx.project);

    // The package disappeared entirely.
    fs::remove_dir_all(fx.registry.join("packages/arc")).unwrap();
    let check = lock_check(&fx);
    assert!(!check.status.success());
    assert!(
        stdout(&check).contains("package-missing"),
        "{}",
        stdout(&check)
    );
    let err = stderr(&check);
    assert!(err.contains("no longer provides package 'arc'"), "{err}");
    assert!(!err.contains("checksum drift"), "{err}");
    snapshot.assert_unchanged(&fx.project);
}

#[test]
fn lock_check_reports_changed_registry_source_identity() {
    let fx = locked_archive_project("check_source");
    isolate_cache(&fx.project);
    // Re-lock against a registry with a non-default identity, then rename it.
    let manifest = fs::read_to_string(fx.registry.join("registry.toml")).unwrap();
    fs::write(
        fx.registry.join("registry.toml"),
        manifest.replace("name = \"default\"", "name = \"mirror\""),
    )
    .unwrap();
    fs::remove_file(fx.project.join("crux.lock")).unwrap();
    assert_success(&run_cli(&fx.project, &fx.registry, &["lock"]));
    let manifest = fs::read_to_string(fx.registry.join("registry.toml")).unwrap();
    fs::write(
        fx.registry.join("registry.toml"),
        manifest.replace("name = \"mirror\"", "name = \"relocated\""),
    )
    .unwrap();
    let snapshot = ProjectSnapshot::take(&fx.project);

    let check = lock_check(&fx);
    assert!(!check.status.success(), "stdout:\n{}", stdout(&check));
    assert!(
        stdout(&check).contains("source-changed"),
        "{}",
        stdout(&check)
    );
    let err = stderr(&check);
    assert!(err.contains("'registry+mirror'"), "{err}");
    assert!(err.contains("'registry+relocated'"), "{err}");
    assert!(
        err.contains("source identity change, not checksum drift"),
        "{err}"
    );
    snapshot.assert_unchanged(&fx.project);
}

#[test]
fn lock_check_reports_unavailable_registry_metadata() {
    let fx = locked_archive_project("check_unavailable");
    isolate_cache(&fx.project);
    let snapshot = ProjectSnapshot::take(&fx.project);

    // Registry metadata for an individual package records no checksum.
    let temp_registry = fx.registry.clone();
    write_manifest(&fx.project, "arc = \"=1.0.0\"\nzlib = \"^1.3\"\n");
    assert_success(&run_cli(&fx.project, &temp_registry, &["lock"]));
    let per_package = ProjectSnapshot::take(&fx.project);
    let check = lock_check(&fx);
    assert_success(&check);
    assert!(
        stdout(&check).contains("checksum-unavailable"),
        "{}",
        stdout(&check)
    );
    assert!(
        stderr(&check).contains("records no checksum for 'zlib 1.3.1'"),
        "{}",
        stderr(&check)
    );
    per_package.assert_unchanged(&fx.project);

    // The whole registry is unavailable.
    fs::write(fx.project.join("crux.toml"), &snapshot.manifest).unwrap();
    fs::write(fx.project.join("crux.lock"), &snapshot.lock).unwrap();
    let missing_registry = fx.registry.parent().unwrap().join("no-such-registry");
    let check = run_cli(&fx.project, &missing_registry, &["lock", "--check"]);
    assert!(!check.status.success());
    let err = stderr(&check);
    assert!(
        err.contains("Status: Unverified (registry metadata unavailable)"),
        "{err}"
    );
    assert!(err.contains("could not be compared"), "{err}");
    assert!(!err.contains("checksum drift"), "{err}");
    snapshot.assert_unchanged(&fx.project);
}
