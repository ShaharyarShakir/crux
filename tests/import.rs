use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "crux_m23_import_{prefix}_{nanos}_{}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("failed to create temporary test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn crux() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crux"))
}

fn run_crux(args: &[&str], cwd: &Path) -> Output {
    crux()
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to execute crux binary")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn write_minimal_c_project(dir: &Path) {
    write_file(
        &dir.join("src/main.c"),
        "#include <stdio.h>\nint main(void) { puts(\"imported-c-ok\"); return 0; }\n",
    );
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}\nstdout: {}",
        stderr(output),
        stdout(output)
    );
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "expected failure, stdout: {}",
        stdout(output)
    );
}

#[test]
fn import_basic_c_project_checks_builds_and_runs() {
    let temp = TestDir::new("basic_c");
    let project = temp.path();
    write_minimal_c_project(project);

    let output = run_crux(&["import"], project);
    assert_success(&output);
    let out = stdout(&output);
    assert!(out.contains("Crux Project Import"));
    assert!(out.contains("Language: C"));
    assert!(out.contains("Build system: None detected"));
    assert!(out.contains("Source directories: src"));
    assert!(out.contains("Created:"));
    assert!(out.contains("crux.toml"));
    assert!(out.contains(".crux/import.toml"));
    assert!(out.contains("Preserved:"));
    assert!(out.contains("src/"));

    let manifest = fs::read_to_string(project.join("crux.toml")).unwrap();
    assert!(manifest.contains("[project]"));
    assert!(manifest.contains("version = \"0.1.0\""));
    assert!(manifest.contains("language = \"c\""));

    let metadata = fs::read_to_string(project.join(".crux/import.toml")).unwrap();
    assert!(metadata.contains("version = 1"));
    assert!(metadata.contains("\"src\""));

    let check = run_crux(&["check"], project);
    assert_success(&check);

    let build = run_crux(&["build"], project);
    assert_success(&build);
    assert!(!project.join("CMakeLists.txt").exists());

    let generated =
        fs::read_to_string(project.join(".crux/build/generated/CMakeLists.txt")).unwrap();
    assert!(generated.contains("src/main.c"));
    assert!(!generated.contains(project.to_string_lossy().as_ref()));

    let run = run_crux(&["run"], project);
    assert_success(&run);
    assert!(stdout(&run).contains("imported-c-ok"));
}

#[test]
fn import_cpp_project_uses_detected_include_directory() {
    let temp = TestDir::new("cpp_include");
    let project = temp.path();
    write_file(
        &project.join("src/main.cpp"),
        "#include \"util.hpp\"\nint main() { return answer() == 42 ? 0 : 1; }\n",
    );
    write_file(
        &project.join("include/util.hpp"),
        "#pragma once\ninline int answer() { return 42; }\n",
    );

    let output = run_crux(&["import"], project);
    assert_success(&output);
    let out = stdout(&output);
    assert!(out.contains("Language: C++"));
    assert!(out.contains("Include directories: include"));

    let build = run_crux(&["build"], project);
    assert_success(&build);
    let generated =
        fs::read_to_string(project.join(".crux/build/generated/CMakeLists.txt")).unwrap();
    assert!(generated.contains("${CRUX_PROJECT_ROOT}/include"));
}

#[test]
fn import_detects_cmake_project_name_and_preserves_cmake_configuration() {
    let temp = TestDir::new("cmake");
    let project = temp.path();
    let cmake_content = "cmake_minimum_required(VERSION 3.20)\nproject(legacy_cmake LANGUAGES CXX)\nadd_executable(custom_target src/main.cpp)\n";
    write_file(&project.join("CMakeLists.txt"), cmake_content);
    write_file(&project.join("src/main.cpp"), "int main() { return 0; }\n");

    let output = run_crux(&["import"], project);
    assert_success(&output);
    let out = stdout(&output);
    assert!(out.contains("Project: legacy_cmake"));
    assert!(out.contains("Build system: CMake"));
    assert!(out.contains("CMakeLists.txt"));

    assert_eq!(
        fs::read_to_string(project.join("CMakeLists.txt")).unwrap(),
        cmake_content
    );
    assert!(
        fs::read_to_string(project.join("crux.toml"))
            .unwrap()
            .contains("name = \"legacy_cmake\"")
    );
}

#[test]
fn import_detects_all_makefile_names_without_executing_them() {
    for filename in ["Makefile", "makefile", "GNUmakefile"] {
        let temp = TestDir::new("make");
        let project = temp.path();
        write_file(
            &project.join(filename),
            "all:\n\t@echo executed > make-was-executed.txt\n",
        );
        write_file(
            &project.join("src/main.c"),
            "int main(void) { return 0; }\n",
        );

        let output = run_crux(&["import"], project);
        assert!(
            output.status.success(),
            "{filename}: stderr: {}\nstdout: {}",
            stderr(&output),
            stdout(&output)
        );
        let out = stdout(&output);
        assert!(out.contains("Build system: Make"));
        assert!(out.contains("Make builds are not executed"));
        assert!(out.contains(filename));
        assert!(!project.join("make-was-executed.txt").exists());
        assert_eq!(
            fs::read_to_string(project.join(filename)).unwrap(),
            "all:\n\t@echo executed > make-was-executed.txt\n"
        );
    }
}

#[test]
fn import_multiple_source_directories() {
    let temp = TestDir::new("multi_dirs");
    let project = temp.path();
    write_file(
        &project.join("source/main.c"),
        "int helper(void);\nint main(void) { return helper() == 7 ? 0 : 1; }\n",
    );
    write_file(
        &project.join("app/helper.c"),
        "int helper(void) { return 7; }\n",
    );

    let output = run_crux(&["import"], project);
    assert_success(&output);
    assert!(stdout(&output).contains("Source directories: app, source"));

    let metadata = fs::read_to_string(project.join(".crux/import.toml")).unwrap();
    assert!(metadata.contains("\"app\""));
    assert!(metadata.contains("\"source\""));

    let build = run_crux(&["build"], project);
    assert_success(&build);
    let generated =
        fs::read_to_string(project.join(".crux/build/generated/CMakeLists.txt")).unwrap();
    assert!(generated.contains("app/helper.c"));
    assert!(generated.contains("source/main.c"));
}

#[test]
fn import_root_level_sources_and_ignored_directories() {
    let temp = TestDir::new("root_sources");
    let project = temp.path();
    write_file(
        &project.join("main.c"),
        "int helper(void);\nint main(void) { return helper() == 3 ? 0 : 1; }\n",
    );
    write_file(
        &project.join("helper.c"),
        "int helper(void) { return 3; }\n",
    );
    for dir in ["build", "out", "dist", "node_modules", ".git"] {
        write_file(
            &project.join(dir).join("ignored.c"),
            "this is not valid C and must never be discovered\n",
        );
    }

    let output = run_crux(&["import"], project);
    assert_success(&output);
    assert!(stdout(&output).contains("Source directories: ."));
    let metadata = fs::read_to_string(project.join(".crux/import.toml")).unwrap();
    assert!(metadata.contains("\".\""));

    let build = run_crux(&["build"], project);
    assert_success(&build);
    let generated =
        fs::read_to_string(project.join(".crux/build/generated/CMakeLists.txt")).unwrap();
    assert!(generated.contains("main.c"));
    assert!(generated.contains("helper.c"));
    for dir in ["build", "out", "dist", "node_modules", ".git"] {
        assert!(!generated.contains(&format!("{dir}/ignored.c")));
    }
}

#[test]
fn import_multiple_executable_targets_requires_force() {
    let temp = TestDir::new("multi_target");
    let project = temp.path();
    write_file(&project.join("src/main.cpp"), "int main() { return 0; }\n");
    write_file(
        &project.join("tools/main.cpp"),
        "int main() { return 0; }\n",
    );

    let output = run_crux(&["import"], project);
    assert_failure(&output);
    let err = stderr(&output);
    assert!(err.contains("unable to confidently import project"));
    assert!(err.contains("executable entry points"));
    assert!(err.contains("src/main"));
    assert!(err.contains("tools/main"));
    assert!(!project.join("crux.toml").exists());

    let forced = run_crux(&["import", "--force"], project);
    assert_success(&forced);
    let out = stdout(&forced);
    assert!(out.contains("Force mode"));
    assert!(out.contains("only one executable target"));
    assert!(project.join("crux.toml").is_file());
}

#[test]
fn import_missing_entry_point_requires_force() {
    let temp = TestDir::new("missing_main");
    let project = temp.path();
    write_file(
        &project.join("src/util.cpp"),
        "int utility() { return 1; }\n",
    );

    let output = run_crux(&["import"], project);
    assert_failure(&output);
    assert!(stderr(&output).contains("no conventional main() entry point"));
    assert!(!project.join("crux.toml").exists());

    let forced = run_crux(&["import", "--force"], project);
    assert_success(&forced);
    assert!(stdout(&forced).contains("crux build may fail to link"));
    assert!(project.join("crux.toml").is_file());
}

#[test]
fn import_invalid_project_directory_reports_actionable_error() {
    let temp = TestDir::new("invalid_dir");
    let project = temp.path();

    let missing = run_crux(&["import", "--path", "./does-not-exist"], project);
    assert_failure(&missing);
    assert!(stderr(&missing).contains("invalid project directory"));
    assert!(!project.join("crux.toml").exists());

    write_file(&project.join("not-a-dir"), "file\n");
    let file_path = run_crux(&["import", "--path", "not-a-dir"], project);
    assert_failure(&file_path);
    assert!(stderr(&file_path).contains("path is not a directory"));
}

#[test]
fn import_existing_manifest_is_preserved_even_with_force() {
    let temp = TestDir::new("existing_manifest");
    let project = temp.path();
    write_minimal_c_project(project);
    let original = "[project]\nname = \"hand-written\"\nversion = \"9.8.7\"\nlanguage = \"c\"\nstandard = \"c17\"\n";
    write_file(&project.join("crux.toml"), original);

    for args in [
        vec!["import"],
        vec!["import", "--force"],
        vec!["import", "--dry-run", "--force"],
    ] {
        let output = run_crux(&args, project);
        assert_failure(&output);
        let err = stderr(&output);
        assert!(err.contains("already initialized"));
        assert!(err.contains("preserved"));
        assert_eq!(
            fs::read_to_string(project.join("crux.toml")).unwrap(),
            original
        );
        assert!(!project.join(".crux/import.toml").exists());
    }
}

#[test]
fn import_dry_run_and_path_option_select_target_directory() {
    let temp = TestDir::new("dry_run");
    let workspace = temp.path();
    let project = workspace.join("legacy-project");
    write_minimal_c_project(&project);

    let dry = run_crux(
        &["import", "--path", "./legacy-project", "--dry-run"],
        workspace,
    );
    assert_success(&dry);
    let out = stdout(&dry);
    assert!(out.contains("Dry run: no files were written"));
    assert!(out.contains("Would create:"));
    assert!(out.contains("Proposed crux.toml:"));
    assert!(!project.join("crux.toml").exists());
    assert!(!project.join(".crux/import.toml").exists());
    assert!(!workspace.join("crux.toml").exists());

    let real = run_crux(&["import", "--path=./legacy-project"], workspace);
    assert_success(&real);
    assert!(project.join("crux.toml").is_file());
    assert!(project.join(".crux/import.toml").is_file());
    assert!(!workspace.join("crux.toml").exists());
}

#[test]
fn import_force_refreshes_only_crux_managed_metadata() {
    let temp = TestDir::new("force_metadata");
    let project = temp.path();
    write_minimal_c_project(project);

    let first = run_crux(&["import"], project);
    assert_success(&first);
    let first_manifest = fs::read_to_string(project.join("crux.toml")).unwrap();
    fs::remove_file(project.join("crux.toml")).unwrap();
    write_file(
        &project.join(".crux/import.toml"),
        "version = 1\nsource_roots = [\"stale\"]\ninclude_dirs = []\n",
    );

    let conflict = run_crux(&["import"], project);
    assert_failure(&conflict);
    assert!(stderr(&conflict).contains("conflicting Crux import metadata"));
    assert!(!project.join("crux.toml").exists());

    let forced = run_crux(&["import", "--force"], project);
    assert_success(&forced);
    let out = stdout(&forced);
    assert!(out.contains("Updated (--force):"));
    assert!(out.contains(".crux/import.toml"));
    let metadata = fs::read_to_string(project.join(".crux/import.toml")).unwrap();
    assert!(metadata.contains("\"src\""));
    assert!(!metadata.contains("\"stale\""));
    assert_eq!(
        fs::read_to_string(project.join("crux.toml")).unwrap(),
        first_manifest
    );
}

#[cfg(unix)]
#[test]
fn import_skips_symlinks_outside_project_root() {
    use std::os::unix::fs::symlink;

    let temp = TestDir::new("symlink");
    let project = temp.path().join("project");
    let outside = temp.path().join("outside.c");
    write_minimal_c_project(&project);
    write_file(&outside, "int outside = 1;\n");
    symlink(&outside, project.join("src/escape.c")).unwrap();

    let output = run_crux(&["import", "--path", "project"], temp.path());
    assert_success(&output);
    let out = stdout(&output);
    assert!(out.contains("skipped symlink"));
    assert!(out.contains("outside the project root"));
    let metadata = fs::read_to_string(project.join(".crux/import.toml")).unwrap();
    assert!(!metadata.contains("escape.c"));
}

#[test]
fn build_rejects_unsafe_import_metadata_paths() {
    let temp = TestDir::new("unsafe_metadata");
    let project = temp.path();
    write_file(
        &project.join("crux.toml"),
        "[project]\nname = \"unsafe-metadata\"\nversion = \"0.1.0\"\nlanguage = \"c\"\nstandard = \"c17\"\n",
    );
    write_file(
        &project.join("src/main.c"),
        "int main(void) { return 0; }\n",
    );
    write_file(
        &project.join(".crux/import.toml"),
        "version = 1\nsource_roots = [\"../outside\"]\ninclude_dirs = []\n",
    );

    let build = run_crux(&["build"], project);
    assert_failure(&build);
    assert!(stderr(&build).contains("invalid Crux import metadata"));
    assert!(stderr(&build).contains("unsafe relative path"));
}

#[test]
fn regression_init_build_and_run_remain_functional() {
    let temp = TestDir::new("regression");
    let root = temp.path();

    let init = run_crux(&["init", "regression-app", "--language", "c"], root);
    assert_success(&init);
    let project = root.join("regression-app");
    write_file(
        &project.join("src/main.c"),
        "#include <stdio.h>\nint main(void) { puts(\"regression-ok\"); return 0; }\n",
    );

    let build = run_crux(&["build"], &project);
    assert_success(&build);
    let run = run_crux(&["run"], &project);
    assert_success(&run);
    assert!(stdout(&run).contains("regression-ok"));
}
