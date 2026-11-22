//! Offline integration coverage for M24 direct package sources.
//!
//! All Git repositories, archives, and local packages in this suite are created
//! inside temporary directories. No external network service is required.

mod common;

use common::{configure_registry, init_fixture_registry};
use crux::cli::{Action, CliError, parse_args};
use crux::lock::{Checksum, Lockfile, PackageSource};
use crux::package::PackageName;
use crux::registry::PackageSourceDefinition;
use crux::resolution::{DirectSource, GitRef, normalize_direct_path, validate_git_ref};
use crux::source::{MockHttpTransport, SourceAcquirer, SourceCache};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
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
            std::env::temp_dir().join(format!("crux_m24_{prefix}_{}_{id}", std::process::id()));
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

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Crux Test")
        .env("GIT_AUTHOR_EMAIL", "crux-test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Crux Test")
        .env("GIT_COMMITTER_EMAIL", "crux-test@example.invalid")
        .output()
        .expect("failed to execute git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write_manifest(project: &Path, dependencies: &str) {
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("crux.toml"),
        format!(
            "[project]\nname = \"m24-app\"\nversion = \"0.1.0\"\nlanguage = \"cpp\"\n\n[dependencies]\n{dependencies}"
        ),
    )
    .unwrap();
    fs::write(project.join("src/main.cpp"), "int main() { return 0; }\n").unwrap();
}

fn package_toml(name: &str, version: &str, dependencies: &str, cmake: bool) -> String {
    let build = if cmake {
        "\n[build]\nsystem = \"cmake\"\n"
    } else {
        ""
    };
    format!(
        "[package]\nname = \"{name}\"\nversion = \"{version}\"\n\n{build}\n[dependencies]\n{dependencies}"
    )
}

fn write_package(dir: &Path, name: &str, version: &str, dependencies: &str, cmake: bool) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("package.toml"),
        package_toml(name, version, dependencies, cmake),
    )
    .unwrap();

    if cmake {
        let target = format!("crux_{}", name.replace('-', "_"));
        fs::create_dir_all(dir.join("include")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("include").join(format!("{name}.h")),
            format!("#pragma once\nint {target}_value(void);\n"),
        )
        .unwrap();
        fs::write(
            dir.join("src/lib.c"),
            format!("int {target}_value(void) {{ return 42; }}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("CMakeLists.txt"),
            format!(
                "cmake_minimum_required(VERSION 3.16)\nproject({target} LANGUAGES C)\nadd_library({target} STATIC src/lib.c)\ntarget_include_directories({target} PUBLIC include)\ninstall(TARGETS {target} ARCHIVE DESTINATION lib)\ninstall(FILES include/{name}.h DESTINATION include/{name})\n"
            ),
        )
        .unwrap();
    }
}

fn write_git_package(
    root: &Path,
    name: &str,
    version: &str,
    dependencies: &str,
    tag: Option<&str>,
) -> (PathBuf, String) {
    let dir = root.join(format!("{name}-git"));
    write_package(&dir, name, version, dependencies, true);
    git(&dir, &["init", "-q"]);
    git(&dir, &["checkout", "-q", "-b", "main"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "initial package"]);
    let commit = git(&dir, &["rev-parse", "HEAD"]);
    if let Some(tag) = tag {
        git(&dir, &["tag", tag]);
    }
    (dir, commit)
}

fn amend_git_package(dir: &Path, version: &str) -> String {
    write_package(
        dir,
        fs::read_to_string(dir.join("package.toml"))
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("name = \""))
            .unwrap()
            .trim_end_matches('"'),
        version,
        "",
        true,
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "update package"]);
    git(dir, &["rev-parse", "HEAD"])
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

fn locked<'a>(lockfile: &'a Lockfile, name: &str) -> &'a crux::lock::LockedPackage {
    lockfile
        .packages
        .iter()
        .find(|pkg| pkg.name.as_str() == name)
        .unwrap_or_else(|| panic!("package '{name}' missing from lockfile"))
}

fn create_targz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_path(*name).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, *content).unwrap();
        }
        builder.finish().unwrap();
    }
    let mut gz_bytes = Vec::new();
    {
        let mut encoder =
            flate2::write::GzEncoder::new(&mut gz_bytes, flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap();
    }
    gz_bytes
}

#[test]
fn direct_source_parser_identity_and_validation_rules() {
    let source = PackageSource::try_git(
        "https://example.invalid/util.git",
        GitRef::Tag("v1.2.3".to_string()),
    )
    .unwrap();
    let locked = PackageSource::git_commit(
        "https://example.invalid/util.git",
        GitRef::Tag("v1.2.3".to_string()),
        "0123456789abcdef0123456789abcdef01234567",
    );
    assert_eq!(source.identity_key(), locked.identity_key());
    assert_ne!(source.as_str(), locked.as_str());

    assert_eq!(normalize_direct_path("a/./b/../c").to_string_lossy(), "a/c");
    assert!(validate_git_ref("../outside").is_err());
    assert!(validate_git_ref("--upload-pack=x").is_err());
    assert!(matches!(
        DirectSource::parse("path+../local"),
        Ok(DirectSource::Path { .. })
    ));
}

#[test]
fn local_path_source_is_locked_and_deterministic() {
    let temp = TempDir::new("path_lock");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    write_package(
        &temp.path.join("local-util"),
        "local-util",
        "1.2.3",
        "",
        false,
    );
    write_manifest(&project, "local-util = { path = \"../local-util\" }\n");

    let first = run_cli(&project, &registry, &["lock"]);
    assert_success(&first);
    let first_lock = fs::read_to_string(project.join("crux.lock")).unwrap();
    assert!(first_lock.contains("path+../local-util"));
    assert!(first_lock.contains("name = \"local-util\""));
    assert!(first_lock.contains("version = \"1.2.3\""));

    let second = run_cli(&project, &registry, &["lock"]);
    assert_success(&second);
    assert_eq!(
        first_lock,
        fs::read_to_string(project.join("crux.lock")).unwrap()
    );

    let check = run_cli(&project, &registry, &["lock", "--check"]);
    assert_success(&check);
    assert!(stdout(&check).contains("Status: Valid"));

    let resolve = run_cli(&project, &registry, &["resolve"]);
    assert_success(&resolve);
    assert!(stdout(&resolve).contains("local-util 1.2.3 (path+../local-util)"));
}

#[test]
fn invalid_and_ambiguous_direct_sources_are_rejected() {
    let temp = TempDir::new("invalid_sources");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));

    write_manifest(&project, "missing = { path = \"../does-not-exist\" }\n");
    let missing = run_cli(&project, &registry, &["lock"]);
    assert!(!missing.status.success());
    assert!(stderr(&missing).contains("cannot be resolved"));

    write_package(
        &temp.path.join("bad-package"),
        "bad-package",
        "1.0.0",
        "",
        false,
    );
    let no_manifest = temp.path.join("no-package-toml");
    fs::create_dir_all(&no_manifest).unwrap();
    write_manifest(&project, "bad = { path = \"../no-package-toml\" }\n");
    let missing_toml = run_cli(&project, &registry, &["lock"]);
    assert!(!missing_toml.status.success());
    assert!(stderr(&missing_toml).contains("package.toml"));

    write_manifest(
        &project,
        "bad = { path = \"../bad-package\", git = \"https://example.invalid/bad.git\" }\n",
    );
    let ambiguous = run_cli(&project, &registry, &["check"]);
    assert!(!ambiguous.status.success());
    assert!(stderr(&ambiguous).contains("exactly one source kind"));

    write_manifest(
        &project,
        "bad = { git = \"https://example.invalid/bad.git\", tag = \"../unsafe\" }\n",
    );
    let unsafe_ref = run_cli(&project, &registry, &["check"]);
    assert!(!unsafe_ref.status.success());
    assert!(stderr(&unsafe_ref).contains("unsafe Git revision"));
}

#[test]
fn git_tag_source_is_resolved_to_an_immutable_commit() {
    let temp = TempDir::new("git_tag");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let (repo, commit) = write_git_package(&temp.path, "git-tag-util", "1.0.0", "", Some("v1.0.0"));

    write_manifest(
        &project,
        &format!(
            "git-tag-util = {{ git = \"{}\", tag = \"v1.0.0\" }}\n",
            repo.display()
        ),
    );
    let first = run_cli(&project, &registry, &["lock"]);
    assert_success(&first);
    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    let locked = locked(&lockfile, "git-tag-util");
    assert_eq!(
        locked.source.as_str(),
        format!("git+{}#tag=v1.0.0#commit={commit}", repo.display())
    );
    assert_eq!(locked.version.to_string(), "1.0.0");
    assert!(locked.checksum.is_some());

    let first_lock = fs::read_to_string(project.join("crux.lock")).unwrap();
    let second = run_cli(&project, &registry, &["lock"]);
    assert_success(&second);
    assert_eq!(
        first_lock,
        fs::read_to_string(project.join("crux.lock")).unwrap()
    );

    let _ = amend_git_package(&repo, "9.9.9");
    let after_upstream_change = run_cli(&project, &registry, &["lock"]);
    assert_success(&after_upstream_change);
    assert_eq!(
        first_lock,
        fs::read_to_string(project.join("crux.lock")).unwrap()
    );
}

#[test]
fn git_branch_commit_is_stable_until_explicit_update() {
    let temp = TempDir::new("git_branch");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let (repo, first_commit) = write_git_package(&temp.path, "branch-util", "1.0.0", "", None);

    write_manifest(
        &project,
        &format!(
            "branch-util = {{ git = \"{}\", branch = \"main\" }}\n",
            repo.display()
        ),
    );
    let first = run_cli(&project, &registry, &["lock"]);
    assert_success(&first);
    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    assert_eq!(
        locked(&lockfile, "branch-util").source.as_str(),
        format!("git+{}#branch=main#commit={first_commit}", repo.display())
    );

    let second_commit = amend_git_package(&repo, "1.1.0");
    let stable = run_cli(&project, &registry, &["lock"]);
    assert_success(&stable);
    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    assert_eq!(
        locked(&lockfile, "branch-util").version.to_string(),
        "1.0.0"
    );
    assert!(
        locked(&lockfile, "branch-util")
            .source
            .as_str()
            .ends_with(&first_commit)
    );

    let update = run_cli(&project, &registry, &["lock", "--update"]);
    assert_success(&update);
    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    assert_eq!(
        locked(&lockfile, "branch-util").version.to_string(),
        "1.1.0"
    );
    assert!(
        locked(&lockfile, "branch-util")
            .source
            .as_str()
            .ends_with(&second_commit)
    );
}

#[test]
fn git_rev_and_commit_sources_are_locked() {
    let temp = TempDir::new("git_rev");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let (repo, commit) = write_git_package(&temp.path, "rev-util", "2.0.0", "", None);
    let short = &commit[..12];

    for (label, dependency) in [
        (
            "rev",
            format!(
                "rev-util = {{ git = \"{}\", rev = \"{short}\" }}\n",
                repo.display()
            ),
        ),
        (
            "commit",
            format!(
                "rev-util = {{ git = \"{}\", commit = \"{commit}\" }}\n",
                repo.display()
            ),
        ),
    ] {
        let project = temp.path.join(label);
        write_manifest(&project, &dependency);
        let output = run_cli(&project, &registry, &["lock"]);
        assert_success(&output);
        let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
        let source = locked(&lockfile, "rev-util").source.as_str().to_string();
        assert!(source.ends_with(&format!("#commit={commit}")), "{source}");
    }
}

#[test]
fn git_ref_failures_are_actionable() {
    let temp = TempDir::new("git_ref_fail");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let (repo, _) = write_git_package(&temp.path, "ref-util", "1.0.0", "", Some("v1.0.0"));

    write_manifest(
        &project,
        &format!(
            "ref-util = {{ git = \"{}\", tag = \"does-not-exist\" }}\n",
            repo.display()
        ),
    );
    let output = run_cli(&project, &registry, &["lock"]);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.contains("git operation failed") || message.contains("did not match any"),
        "unexpected error: {message}"
    );
}

#[test]
fn direct_sources_conflict_with_registry_and_each_other() {
    let temp = TempDir::new("conflicts");
    let registry = init_fixture_registry(&temp.path.join("registry"));

    let registry_conflict = temp.path.join("registry-conflict");
    write_package(&temp.path.join("fmt-local"), "fmt", "99.0.0", "", false);
    write_manifest(&registry_conflict, "fmt = { path = \"../fmt-local\" }\n");
    let output = run_cli(&registry_conflict, &registry, &["lock"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("also present in registry"));

    let direct_conflict = temp.path.join("direct-conflict");
    write_package(
        &temp.path.join("root-a"),
        "root-a",
        "1.0.0",
        "dup = { path = \"../dup-a\" }\n",
        false,
    );
    write_package(
        &temp.path.join("root-b"),
        "root-b",
        "1.0.0",
        "dup = { path = \"../dup-b\" }\n",
        false,
    );
    write_package(&temp.path.join("dup-a"), "dup", "1.0.0", "", false);
    write_package(&temp.path.join("dup-b"), "dup", "2.0.0", "", false);
    write_manifest(
        &direct_conflict,
        "root-a = { path = \"../root-a\" }\nroot-b = { path = \"../root-b\" }\n",
    );
    let output = run_cli(&direct_conflict, &registry, &["lock"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("conflicting direct sources"));
}

#[test]
fn path_source_cycles_are_rejected() {
    let temp = TempDir::new("cycle");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    write_package(
        &temp.path.join("cycle-a"),
        "cycle-a",
        "1.0.0",
        "cycle-b = { path = \"../cycle-b\" }\n",
        false,
    );
    write_package(
        &temp.path.join("cycle-b"),
        "cycle-b",
        "1.0.0",
        "cycle-a = { path = \"../cycle-a\" }\n",
        false,
    );
    write_manifest(&project, "cycle-a = { path = \"../cycle-a\" }\n");

    let output = run_cli(&project, &registry, &["lock"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cycle") || stderr(&output).contains("more than once"),
        "unexpected error: {}",
        stderr(&output)
    );
}

#[test]
fn mixed_direct_and_registry_sources_are_deterministic() {
    let temp = TempDir::new("mixed");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    write_package(
        &temp.path.join("mixed-root"),
        "mixed-root",
        "1.0.0",
        "zlib = \"^1.3\"\nhelper = { path = \"../helper\" }\n",
        false,
    );
    write_package(&temp.path.join("helper"), "helper", "3.2.1", "", false);
    write_manifest(&project, "mixed-root = { path = \"../mixed-root\" }\n");

    let first = run_cli(&project, &registry, &["lock"]);
    assert_success(&first);
    let first_lock = fs::read_to_string(project.join("crux.lock")).unwrap();
    assert!(first_lock.contains("path+../mixed-root"));
    assert!(first_lock.contains("path+../helper"));
    assert!(first_lock.contains("registry+default"));

    let second = run_cli(&project, &registry, &["lock"]);
    assert_success(&second);
    assert_eq!(
        first_lock,
        fs::read_to_string(project.join("crux.lock")).unwrap()
    );
}

#[test]
fn archive_source_requires_matching_sha256_before_extraction() {
    let temp = TempDir::new("archive");
    let package_toml = package_toml("archive-util", "1.0.0", "", false);
    let archive = create_targz(&[("package.toml", package_toml.as_bytes())]);
    let mut hasher = Sha256::new();
    hasher.update(&archive);
    let digest = format!("{:x}", hasher.finalize());

    let transport = MockHttpTransport::new();
    let url = "https://example.invalid/archive-util.tar.gz";
    transport.add_response(url, archive.clone());
    let cache = SourceCache::new(temp.path.join("cache")).unwrap();
    let acquirer = SourceAcquirer::with_transport(cache, std::sync::Arc::new(transport));
    let checksum = Checksum::new_sha256(&digest).unwrap();
    let id = crux::package::PackageId::new(
        PackageName::new("archive-util").unwrap(),
        semver::Version::new(1, 0, 0),
    );
    let source = PackageSourceDefinition::Archive {
        url: url.to_string(),
        checksum: checksum.clone(),
    };

    let acquired = acquirer
        .acquire_direct(&id, &source, &temp.path, false)
        .unwrap();
    assert!(acquired.source_dir.join("package.toml").is_file());
    assert_eq!(acquired.checksum, checksum);

    let bad_url = "https://example.invalid/bad.tar.gz";
    let bad_transport = MockHttpTransport::new();
    bad_transport.add_response(bad_url, archive);
    let bad_cache = SourceCache::new(temp.path.join("bad-cache")).unwrap();
    let bad_acquirer =
        SourceAcquirer::with_transport(bad_cache, std::sync::Arc::new(bad_transport));
    let bad_checksum =
        Checksum::new_sha256("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();
    let error = bad_acquirer
        .acquire_direct(
            &id,
            &PackageSourceDefinition::Archive {
                url: bad_url.to_string(),
                checksum: bad_checksum,
            },
            &temp.path,
            false,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        crux::source::AcquisitionError::ChecksumMismatch { .. }
    ));
}

#[test]
fn add_flags_support_git_and_path_sources_and_reject_ambiguous_options() {
    assert!(matches!(
        parse_args(&[
            "add",
            "util",
            "--git",
            "https://example.invalid/util.git",
            "--tag",
            "v1.0.0"
        ]),
        Ok(Action::Add(_))
    ));
    assert!(matches!(
        parse_args(&["add", "util", "--path", "../util"]),
        Ok(Action::Add(_))
    ));
    assert!(matches!(
        parse_args(&[
            "add",
            "util",
            "--url",
            "https://example.invalid/u.tar.gz",
            "--sha256",
            "0000000000000000000000000000000000000000000000000000000000000000"
        ]),
        Ok(Action::Add(_))
    ));
    assert!(matches!(
        parse_args(&["add", "util", "--git", "x", "--path", "../x"]),
        Err(CliError::UnexpectedArgument(_))
    ));
    assert!(matches!(
        parse_args(&["add", "util", "--url", "https://example.invalid/u.zip"]),
        Err(CliError::MissingArgumentValue(_))
    ));
}

#[test]
fn add_git_and_path_direct_sources_update_manifest_and_lockfile() {
    let temp = TempDir::new("add_flows");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    write_manifest(&project, "");
    write_package(&temp.path.join("path-dep"), "path-dep", "1.0.0", "", true);
    let (repo, commit) = write_git_package(&temp.path, "git-dep", "1.0.0", "", Some("v1.0.0"));

    let path_add = run_cli(
        &project,
        &registry,
        &["add", "path-dep", "--path", "../path-dep"],
    );
    assert_success(&path_add);
    let manifest = fs::read_to_string(project.join("crux.toml")).unwrap();
    assert!(manifest.contains("path-dep = { path = \"../path-dep\" }"));
    assert!(project.join("crux.lock").is_file());

    let git_add = run_cli(
        &project,
        &registry,
        &[
            "add",
            "git-dep",
            "--git",
            &repo.display().to_string(),
            "--tag",
            "v1.0.0",
        ],
    );
    assert_success(&git_add);
    let manifest = fs::read_to_string(project.join("crux.toml")).unwrap();
    assert!(manifest.contains("git-dep = { git ="));
    assert!(manifest.contains("tag = \"v1.0.0\""));
    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    assert_eq!(
        locked(&lockfile, "git-dep").source.as_str(),
        format!("git+{}#tag=v1.0.0#commit={commit}", repo.display())
    );
}

#[test]
fn moved_project_preserves_relative_path_sources() {
    let temp = TempDir::new("moved");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let original_root = temp.path.join("workspace");
    let project = original_root.join("project");
    write_package(&original_root.join("local"), "movable", "1.0.0", "", true);
    write_manifest(&project, "movable = { path = \"../local\" }\n");

    let first = run_cli(&project, &registry, &["lock"]);
    assert_success(&first);

    let moved_root = temp.path.join("moved-workspace");
    fs::rename(&original_root, &moved_root).unwrap();
    let moved_project = moved_root.join("project");
    let check = run_cli(&moved_project, &registry, &["lock", "--check"]);
    assert_success(&check);
    assert!(stdout(&check).contains("Status: Valid"));

    let build = run_cli(&moved_project, &registry, &["build"]);
    assert_success(&build);

    fs::remove_dir_all(moved_root.join("local")).unwrap();
    let broken = run_cli(&moved_project, &registry, &["install", "--locked"]);
    assert!(!broken.status.success());
    assert!(stderr(&broken).contains("cannot be resolved"));
}

#[test]
fn direct_source_build_install_and_integrate_pipeline_works() {
    let temp = TempDir::new("pipeline");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    write_package(
        &temp.path.join("pipeline-dep"),
        "pipeline-dep",
        "1.0.0",
        "",
        true,
    );
    write_manifest(&project, "pipeline-dep = { path = \"../pipeline-dep\" }\n");

    let lock = run_cli(&project, &registry, &["lock"]);
    assert_success(&lock);
    let build = run_cli(&project, &registry, &["build"]);
    assert_success(&build);
    let install = run_cli(&project, &registry, &["install", "--locked"]);
    assert_success(&install);
    let integrate = run_cli(&project, &registry, &["integrate", "cmake", "--locked"]);
    assert_success(&integrate);

    let cmake = fs::read_to_string(
        project
            .join(".crux")
            .join("cmake")
            .join("CruxDependencies.cmake"),
    )
    .unwrap();
    assert!(cmake.contains("pipeline-dep"));
    assert!(cmake.contains("1.0.0"));
}

// ---------------------------------------------------------------------------
// Remediation (2026-11-21 audit): Git refs, cache identity, lock verification,
// and local-path snapshots.
// ---------------------------------------------------------------------------

fn isolated_acquirer(root: &Path) -> SourceAcquirer {
    SourceAcquirer::new(SourceCache::new(root.join("cache")).unwrap())
}

fn placeholder_id(name: &str) -> crux::package::PackageId {
    crux::package::PackageId::new(
        PackageName::new(name).unwrap(),
        semver::Version::new(0, 0, 0),
    )
}

fn git_definition(url: &Path, requested: GitRef) -> PackageSourceDefinition {
    PackageSourceDefinition::Git {
        url: url.display().to_string(),
        requested,
        resolved_commit: None,
    }
}

/// Regression: `branch = "<non-default>"` failed because the branch only
/// exists as `refs/remotes/origin/<name>` in a `--no-checkout` clone.
#[test]
fn git_non_default_branch_is_locked_to_its_commit() {
    let temp = TempDir::new("git_dev_branch");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let (repo, main_commit) = write_git_package(&temp.path, "dev-util", "1.0.0", "", None);
    git(&repo, &["checkout", "-q", "-b", "dev"]);
    let dev_commit = amend_git_package(&repo, "1.1.0");
    git(&repo, &["checkout", "-q", "main"]);
    assert_ne!(main_commit, dev_commit);

    write_manifest(
        &project,
        &format!(
            "dev-util = {{ git = \"{}\", branch = \"dev\" }}\n",
            repo.display()
        ),
    );
    let lock = run_cli(&project, &registry, &["lock"]);
    assert_success(&lock);
    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    let locked = locked(&lockfile, "dev-util");
    assert_eq!(locked.version.to_string(), "1.1.0");
    assert_eq!(
        locked.source.as_str(),
        format!("git+{}#branch=dev#commit={dev_commit}", repo.display())
    );
}

/// Mutable refs are never served from cache: re-acquiring a branch after it
/// advances yields the new commit, and the commit is always reported.
/// Immutable commits are served from cache even when the remote is gone.
#[test]
fn git_cache_is_keyed_by_immutable_commit_only() {
    let temp = TempDir::new("git_cache_identity");
    let acquirer = isolated_acquirer(&temp.path);
    let (repo, first_commit) = write_git_package(&temp.path, "moving", "1.0.0", "", None);
    let id = placeholder_id("moving");
    let branch = git_definition(&repo, GitRef::Branch("main".into()));

    let first = acquirer
        .acquire_direct(&id, &branch, &temp.path, false)
        .unwrap();
    assert_eq!(
        first.resolved_git_commit.as_deref(),
        Some(first_commit.as_str())
    );

    let second_commit = amend_git_package(&repo, "1.1.0");
    let second = acquirer
        .acquire_direct(&id, &branch, &temp.path, false)
        .unwrap();
    assert_eq!(
        second.resolved_git_commit.as_deref(),
        Some(second_commit.as_str())
    );
    assert_ne!(first.source_dir, second.source_dir);
    assert!(
        fs::read_to_string(second.source_dir.join("package.toml"))
            .unwrap()
            .contains("1.1.0")
    );
    // The first snapshot is immutable and untouched.
    assert!(
        fs::read_to_string(first.source_dir.join("package.toml"))
            .unwrap()
            .contains("1.0.0")
    );

    // An explicit full-commit request is immutable; acquire it once online.
    let explicit_commit = git_definition(&repo, GitRef::Commit(second_commit.clone()));
    let online = acquirer
        .acquire_direct(&id, &explicit_commit, &temp.path, false)
        .unwrap();
    assert_eq!(
        online.resolved_git_commit.as_deref(),
        Some(second_commit.as_str())
    );

    // Pinned commits come from the cache without contacting the remote.
    let moved = temp.path.join("moved-away");
    fs::rename(&repo, &moved).unwrap();
    let pinned = PackageSourceDefinition::Git {
        url: repo.display().to_string(),
        requested: GitRef::Branch("main".into()),
        resolved_commit: Some(first_commit.clone()),
    };
    let cached = acquirer
        .acquire_direct(&id, &pinned, &temp.path, false)
        .expect("pinned commit must be served from the cache offline");
    assert_eq!(cached.source_dir, first.source_dir);
    assert_eq!(cached.checksum, first.checksum);
    let cached = acquirer
        .acquire_direct(&id, &explicit_commit, &temp.path, false)
        .expect("full commit refs are immutable and served from the cache offline");
    assert_eq!(cached.source_dir, online.source_dir);
    assert_eq!(
        cached.resolved_git_commit.as_deref(),
        Some(second_commit.as_str())
    );

    // A mutable ref is never answered from a stale cache entry.
    assert!(
        acquirer
            .acquire_direct(&id, &branch, &temp.path, false)
            .is_err(),
        "branch refs must be re-resolved against the remote"
    );
}

fn replace_locked_checksum(project: &Path, name: &str, checksum: &str) {
    let lock_path = project.join("crux.lock");
    let mut lockfile = Lockfile::load_from_path(&lock_path).unwrap();
    let package = lockfile
        .packages
        .iter_mut()
        .find(|pkg| pkg.name.as_str() == name)
        .unwrap();
    package.checksum = Some(Checksum::new_sha256(checksum).unwrap());
    fs::write(&lock_path, lockfile.to_toml_string().unwrap()).unwrap();
}

/// Regression: the checksum recorded in `crux.lock` was never compared with
/// the acquired content.
#[test]
fn git_lockfile_checksum_is_verified_on_fetch() {
    let temp = TempDir::new("git_lock_verify");
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let (repo, _) = write_git_package(&temp.path, "verified", "1.0.0", "", Some("v1.0.0"));
    write_manifest(
        &project,
        &format!(
            "verified = {{ git = \"{}\", tag = \"v1.0.0\" }}\n",
            repo.display()
        ),
    );
    assert_success(&run_cli(&project, &registry, &["lock"]));
    assert_success(&run_cli(&project, &registry, &["fetch"]));

    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    let original = locked(&lockfile, "verified").checksum.clone().unwrap();

    // Tampered lock checksum: fetch must refuse the content.
    replace_locked_checksum(&project, "verified", &"c".repeat(64));
    let fetch = run_cli(&project, &registry, &["fetch"]);
    assert!(!fetch.status.success(), "stdout:\n{}", stdout(&fetch));
    assert!(
        stderr(&fetch).contains("does not match crux.lock"),
        "stderr:\n{}",
        stderr(&fetch)
    );

    // Compatibility: a lockfile written by Crux 0.2.0/0.2.1 recorded the legacy v1
    // tree digest. It must still verify. The upstream work tree has the same
    // files and modes as the checkout, so its v1 digest is the released value.
    let legacy = crux::source::compute_legacy_tree_sha256_v1(&repo).unwrap();
    assert_ne!(legacy, original.digest());
    replace_locked_checksum(&project, "verified", &legacy);
    assert_success(&run_cli(&project, &registry, &["fetch"]));

    // Restoring the current digest verifies too.
    replace_locked_checksum(&project, "verified", original.digest());
    assert_success(&run_cli(&project, &registry, &["fetch"]));
}

/// Regression: path sources were hashed and copied into the cache, but the
/// live working directory was returned and built. Builds now read a verified
/// snapshot, and later edits produce a new snapshot instead of mutating it.
#[test]
fn local_path_sources_build_from_verified_cache_snapshots() {
    let temp = TempDir::new("path_snapshot");
    let acquirer = isolated_acquirer(&temp.path);
    let live = temp.path.join("live-pkg");
    write_package(&live, "live-pkg", "1.0.0", "", true);
    // A nested `build/` directory is source, not output, and must be kept.
    fs::create_dir_all(live.join("cmake/build")).unwrap();
    fs::write(live.join("cmake/build/helpers.cmake"), "# helper\n").unwrap();
    // Root-level build output is excluded from the snapshot.
    fs::create_dir_all(live.join("build")).unwrap();
    fs::write(live.join("build/CMakeCache.txt"), "local output\n").unwrap();

    let id = placeholder_id("live-pkg");
    let definition = PackageSourceDefinition::Path {
        path: "live-pkg".to_string(),
    };
    let first = acquirer
        .acquire_direct(&id, &definition, &temp.path, false)
        .unwrap();
    let canonical_live = fs::canonicalize(&live).unwrap();
    assert_ne!(first.source_dir, canonical_live);
    assert!(first.source_dir.starts_with(acquirer.cache().sources_dir()));
    assert!(first.source_dir.join("cmake/build/helpers.cmake").is_file());
    assert!(!first.source_dir.join("build").exists());
    assert_eq!(
        first.checksum.digest(),
        crux::source::compute_tree_sha256(&first.source_dir).unwrap()
    );

    // Re-acquiring an unchanged tree reuses the same snapshot.
    let again = acquirer
        .acquire_direct(&id, &definition, &temp.path, false)
        .unwrap();
    assert_eq!(again.source_dir, first.source_dir);
    assert_eq!(again.checksum, first.checksum);

    // Editing the live tree yields a new snapshot; the old one is untouched.
    fs::write(live.join("src/lib.c"), "int changed(void) { return 1; }\n").unwrap();
    let edited = acquirer
        .acquire_direct(&id, &definition, &temp.path, false)
        .unwrap();
    assert_ne!(edited.source_dir, first.source_dir);
    assert_ne!(edited.checksum, first.checksum);
    assert!(
        fs::read_to_string(first.source_dir.join("src/lib.c"))
            .unwrap()
            .contains("return 42")
    );

    // A tampered snapshot is detected and rebuilt rather than trusted.
    fs::write(edited.source_dir.join("src/lib.c"), "tampered\n").unwrap();
    let rebuilt = acquirer
        .acquire_direct(&id, &definition, &temp.path, false)
        .unwrap();
    assert!(
        fs::read_to_string(rebuilt.source_dir.join("src/lib.c"))
            .unwrap()
            .contains("changed")
    );
}

// ---------------------------------------------------------------------------
// Owner decisions 2 and 3: legacy checksum warnings/migration and re-lock
// integrity for Git sources.
// ---------------------------------------------------------------------------

fn locked_git_project(prefix: &str) -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = TempDir::new(prefix);
    let project = temp.path.join("project");
    let registry = init_fixture_registry(&temp.path.join("registry"));
    let (repo, _) = write_git_package(&temp.path, "pinned", "1.0.0", "", Some("v1.0.0"));
    write_manifest(
        &project,
        &format!(
            "pinned = {{ git = \"{}\", tag = \"v1.0.0\" }}\n",
            repo.display()
        ),
    );
    assert_success(&run_cli(&project, &registry, &["lock"]));
    (temp, project, registry, repo)
}

fn current_checksum(project: &Path, name: &str) -> String {
    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    locked(&lockfile, name)
        .checksum
        .as_ref()
        .unwrap()
        .to_string()
}

const LEGACY_WARNING: &str = "legacy-format (v1) source checksum";

#[test]
fn legacy_git_checksum_warns_once_and_lock_migrates_it() {
    let (_temp, project, registry, repo) = locked_git_project("legacy_migrate");
    let current = current_checksum(&project, "pinned");
    let legacy = crux::source::compute_legacy_tree_sha256_v1(&repo).unwrap();
    replace_locked_checksum(&project, "pinned", &legacy);

    // Acquisition accepts the legacy digest with exactly one actionable warning.
    let fetch = run_cli(&project, &registry, &["fetch"]);
    assert_success(&fetch);
    let err = stderr(&fetch);
    assert_eq!(err.matches(LEGACY_WARNING).count(), 1, "{err}");
    assert!(err.contains("'pinned 1.0.0'"), "{err}");
    assert!(err.contains("consistently across machines"), "{err}");
    assert!(err.contains("Run `crux lock` to migrate it"), "{err}");
    // The lockfile is not modified by fetch.
    assert_eq!(
        current_checksum(&project, "pinned"),
        format!("sha256:{legacy}")
    );

    // A normal `crux lock` migrates the digest without requiring --update.
    let lock = run_cli(&project, &registry, &["lock"]);
    assert_success(&lock);
    let out = stdout(&lock);
    assert!(
        out.contains("Migrated legacy checksum for 'pinned 1.0.0'"),
        "{out}"
    );
    assert!(out.contains(&format!("old: sha256:{legacy}")), "{out}");
    assert!(out.contains(&format!("new: {current}")), "{out}");
    assert_eq!(current_checksum(&project, "pinned"), current);

    // Migration is deterministic and complete: no further warnings or changes.
    let migrated = fs::read(project.join("crux.lock")).unwrap();
    let fetch = run_cli(&project, &registry, &["fetch"]);
    assert_success(&fetch);
    assert!(
        !stderr(&fetch).contains(LEGACY_WARNING),
        "{}",
        stderr(&fetch)
    );
    let relock = run_cli(&project, &registry, &["lock"]);
    assert_success(&relock);
    assert!(!stdout(&relock).contains("Migrated legacy checksum"));
    assert_eq!(migrated, fs::read(project.join("crux.lock")).unwrap());
}

#[test]
fn current_format_git_checksum_produces_no_warning() {
    let (_temp, project, registry, _repo) = locked_git_project("current_format");
    let fetch = run_cli(&project, &registry, &["fetch"]);
    assert_success(&fetch);
    assert!(!stderr(&fetch).contains("warning"), "{}", stderr(&fetch));
}

#[test]
fn git_relock_rejects_changed_tree_checksum_unless_explicitly_updated() {
    let (_temp, project, registry, _repo) = locked_git_project("git_relock");
    let current = current_checksum(&project, "pinned");
    let tampered = "d".repeat(64);
    replace_locked_checksum(&project, "pinned", &tampered);
    let before = fs::read(project.join("crux.lock")).unwrap();

    // Same commit, different recorded content: refused by a normal lock.
    let lock = run_cli(&project, &registry, &["lock"]);
    assert!(!lock.status.success(), "stdout:\n{}", stdout(&lock));
    let err = stderr(&lock);
    assert!(err.contains("integrity error"), "{err}");
    assert!(err.contains(&format!("sha256:{tampered}")), "{err}");
    assert!(err.contains(&current), "{err}");
    assert_eq!(before, fs::read(project.join("crux.lock")).unwrap());

    // And by acquisition.
    let fetch = run_cli(&project, &registry, &["fetch"]);
    assert!(!fetch.status.success());

    // An explicit update accepts and reports it.
    let update = run_cli(&project, &registry, &["lock", "--update"]);
    assert_success(&update);
    let err = stderr(&update);
    assert!(
        err.contains("accepted a changed checksum for 'pinned 1.0.0'"),
        "{err}"
    );
    assert!(err.contains(&format!("old: sha256:{tampered}")), "{err}");
    assert!(err.contains(&format!("new: {current}")), "{err}");
    assert_eq!(current_checksum(&project, "pinned"), current);
}

#[test]
fn git_source_identity_change_is_a_legitimate_relock() {
    let (_temp, project, registry, repo) = locked_git_project("git_identity");
    let first = current_checksum(&project, "pinned");
    let new_commit = amend_git_package(&repo, "1.1.0");
    git(&repo, &["tag", "v1.1.0"]);
    write_manifest(
        &project,
        &format!(
            "pinned = {{ git = \"{}\", tag = \"v1.1.0\" }}\n",
            repo.display()
        ),
    );

    let lock = run_cli(&project, &registry, &["lock"]);
    assert_success(&lock);
    assert!(!stderr(&lock).contains("integrity"), "{}", stderr(&lock));
    let lockfile = Lockfile::load_from_path(&project.join("crux.lock")).unwrap();
    let pinned = locked(&lockfile, "pinned");
    assert_eq!(pinned.version.to_string(), "1.1.0");
    assert!(pinned.source.as_str().ends_with(&new_commit));
    assert_ne!(pinned.checksum.as_ref().unwrap().to_string(), first);
}

#[test]
fn lock_check_warns_on_legacy_git_checksum_and_reports_git_drift_read_only() {
    let (_temp, project, registry, repo) = locked_git_project("check_legacy");
    let legacy = crux::source::compute_legacy_tree_sha256_v1(&repo).unwrap();
    replace_locked_checksum(&project, "pinned", &legacy);
    let lock_before = fs::read(project.join("crux.lock")).unwrap();
    let manifest_before = fs::read(project.join("crux.toml")).unwrap();

    // Legacy digest: accepted with a warning; --check still passes and does not migrate.
    let check = run_cli(&project, &registry, &["lock", "--check"]);
    assert_success(&check);
    assert!(
        stdout(&check).contains("legacy-checksum"),
        "{}",
        stdout(&check)
    );
    assert!(
        stdout(&check).contains("1 warning(s), 0 error(s)"),
        "{}",
        stdout(&check)
    );
    let err = stderr(&check);
    assert!(err.contains(LEGACY_WARNING), "{err}");
    assert!(err.contains("run `crux lock` to migrate it"), "{err}");
    assert_eq!(lock_before, fs::read(project.join("crux.lock")).unwrap());
    assert_eq!(
        manifest_before,
        fs::read(project.join("crux.toml")).unwrap()
    );

    // Tampered digest: reported as drift of the cached direct source.
    replace_locked_checksum(&project, "pinned", &"e".repeat(64));
    let lock_before = fs::read(project.join("crux.lock")).unwrap();
    let check = run_cli(&project, &registry, &["lock", "--check"]);
    assert!(!check.status.success());
    let err = stderr(&check);
    assert!(err.contains("checksum drift for 'pinned 1.0.0'"), "{err}");
    assert!(err.contains("locally cached direct source"), "{err}");
    assert_eq!(lock_before, fs::read(project.join("crux.lock")).unwrap());
    assert_eq!(
        manifest_before,
        fs::read(project.join("crux.toml")).unwrap()
    );
}
