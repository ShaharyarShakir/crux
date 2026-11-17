use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
            "crux_m20_run_{prefix}_{nanos}_{}",
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

fn crux_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crux"))
}

fn write_cpp_main(project_dir: &Path, body: &str) {
    fs::write(project_dir.join("src").join("main.cpp"), body).expect("failed to write main.cpp");
}

fn write_c_main(project_dir: &Path, body: &str) {
    fs::write(project_dir.join("src").join("main.c"), body).expect("failed to write main.c");
}

#[test]
fn run_cpp_application_debug_and_release() {
    let temp = TestDir::new("cpp_debug_release");
    let root = temp.path();

    let status = crux_cmd()
        .args(["init", "demo-cpp", "--language", "cpp"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success());
    let proj = root.join("demo-cpp");

    let out = crux_cmd().arg("run").current_dir(&proj).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Hello from demo-cpp!"));
    assert!(
        proj.join(".crux/build/debug/demo-cpp").is_file()
            || proj.join(".crux/build/debug/demo-cpp.exe").is_file()
    );

    let out_rel = crux_cmd()
        .args(["run", "--release"])
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(
        out_rel.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out_rel.stderr)
    );
    assert!(
        proj.join(".crux/build/release/demo-cpp").is_file()
            || proj.join(".crux/build/release/demo-cpp.exe").is_file()
    );
}

#[test]
fn run_c_application_with_args_and_spaces() {
    let temp = TestDir::new("c_args");
    let root = temp.path();

    let status = crux_cmd()
        .args(["init", "demo-c", "--language", "c"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success());
    let proj = root.join("demo-c");

    write_c_main(
        &proj,
        r#"#include <stdio.h>
int main(int argc, char** argv) {
  for (int i = 1; i < argc; i++) {
    printf("%s\n", argv[i]);
  }
  return 0;
}
"#,
    );

    let out = crux_cmd()
        .args([
            "run",
            "--",
            "--name",
            "Alice Smith",
            "--count",
            "5",
            "-v",
            "--flag",
        ])
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--name\nAlice Smith\n--count\n5\n-v\n--flag"));
}

#[test]
fn run_propagates_nonzero_exit_status() {
    let temp = TestDir::new("nonzero");
    let root = temp.path();
    assert!(
        crux_cmd()
            .args(["init", "failapp", "--language", "cpp"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let proj = root.join("failapp");
    write_cpp_main(&proj, "int main(){return 7;}\n");

    let out = crux_cmd().arg("run").current_dir(&proj).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(7),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_executes_from_project_root_and_preserves_stdin() {
    let temp = TestDir::new("cwd_stdin");
    let root = temp.path();
    assert!(
        crux_cmd()
            .args(["init", "ioapp", "--language", "cpp"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let proj = root.join("ioapp");
    write_cpp_main(
        &proj,
        r#"#include <fstream>
#include <iostream>
#include <string>
int main() {
  std::ofstream out("output.txt");
  out << "ok";
  std::string line;
  if (std::getline(std::cin, line)) std::cout << "stdin:" << line;
  return 0;
}
"#,
    );

    let mut child = crux_cmd()
        .arg("run")
        .current_dir(&proj)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(b"hello\n").unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("stdin:hello"));
    assert_eq!(fs::read_to_string(proj.join("output.txt")).unwrap(), "ok");
    assert!(!proj.join(".crux/build/debug/output.txt").exists());
}

#[test]
fn run_inherits_environment() {
    let temp = TestDir::new("env_inherit");
    let root = temp.path();
    assert!(
        crux_cmd()
            .args(["init", "envapp", "--language", "cpp"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let proj = root.join("envapp");
    write_cpp_main(
        &proj,
        r#"#include <iostream>
#include <cstdlib>
int main() {
  const char* val = std::getenv("CRUX_TEST_VAR");
  if (val) std::cout << "env:" << val << std::endl;
  return 0;
}
"#,
    );

    let out = crux_cmd()
        .arg("run")
        .env("CRUX_TEST_VAR", "test_env_token_42")
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("env:test_env_token_42"));
}

#[test]
fn run_stdout_and_stderr_forwarding() {
    let temp = TestDir::new("stdio_fwd");
    let root = temp.path();
    assert!(
        crux_cmd()
            .args(["init", "stdioapp", "--language", "cpp"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let proj = root.join("stdioapp");
    write_cpp_main(
        &proj,
        r#"#include <iostream>
int main() {
  std::cout << "app_stdout_line" << std::endl;
  std::cerr << "app_stderr_line" << std::endl;
  return 0;
}
"#,
    );

    let out = crux_cmd().arg("run").current_dir(&proj).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.contains("app_stdout_line"));
    assert!(stderr.contains("app_stderr_line"));
}

#[test]
fn run_incremental_rebuild_behavior() {
    let temp = TestDir::new("incremental");
    let root = temp.path();
    assert!(
        crux_cmd()
            .args(["init", "incapp", "--language", "c"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let proj = root.join("incapp");

    // First run builds and executes
    let out1 = crux_cmd().arg("run").current_dir(&proj).output().unwrap();
    assert!(out1.status.success());
    assert!(String::from_utf8_lossy(&out1.stdout).contains("Hello from incapp!"));

    // Second run reuses incremental build without error
    let out2 = crux_cmd().arg("run").current_dir(&proj).output().unwrap();
    assert!(out2.status.success());
    assert!(String::from_utf8_lossy(&out2.stdout).contains("Hello from incapp!"));
}

#[test]
fn run_paths_with_spaces_and_nested_sources() {
    let temp = TestDir::new("spaces_nested");
    let root = temp.path().join("path with spaces");
    fs::create_dir_all(&root).unwrap();

    assert!(
        crux_cmd()
            .args(["init", "nested-app", "--language", "cpp"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    let proj = root.join("nested-app");

    // Add nested helper source
    let modules_dir = proj.join("src").join("modules");
    fs::create_dir_all(&modules_dir).unwrap();
    fs::write(
        modules_dir.join("helper.cpp"),
        r#"#include <string>
std::string get_greeting() {
  return "greeting from helper in nested dir";
}
"#,
    )
    .unwrap();

    write_cpp_main(
        &proj,
        r#"#include <iostream>
#include <string>
std::string get_greeting();
int main() {
  std::cout << get_greeting() << std::endl;
  return 0;
}
"#,
    );

    let out = crux_cmd().arg("run").current_dir(&proj).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("greeting from helper in nested dir"));
}

#[test]
fn run_rejects_other_language_project() {
    let temp = TestDir::new("other_lang");
    let root = temp.path();
    assert!(
        crux_cmd()
            .args(["init", "otherapp", "--language", "other"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let proj = root.join("otherapp");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(proj.join("src/main.c"), "int main(){return 0;}").unwrap();

    let out = crux_cmd().arg("run").current_dir(&proj).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("language 'other'") || stderr.contains("C and C++"));
}

#[test]
fn run_missing_manifest_fails_with_clear_error() {
    let temp = TestDir::new("no_manifest");
    let root = temp.path();

    let out = crux_cmd().arg("run").current_dir(root).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("manifest not found") || stderr.contains("crux.toml"));
}

#[test]
fn build_failure_prevents_execution() {
    let temp = TestDir::new("build_fail");
    let root = temp.path();
    assert!(
        crux_cmd()
            .args(["init", "badapp", "--language", "cpp"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let proj = root.join("badapp");

    let out_ok = crux_cmd().arg("build").current_dir(&proj).output().unwrap();
    assert!(
        out_ok.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out_ok.stderr)
    );
    let exe = if proj.join(".crux/build/debug/badapp").is_file() {
        proj.join(".crux/build/debug/badapp")
    } else {
        proj.join(".crux/build/debug/badapp.exe")
    };
    assert!(exe.is_file());

    write_cpp_main(&proj, "int main( { this is broken }\n");
    let out = crux_cmd().arg("run").current_dir(&proj).output().unwrap();
    assert!(!out.status.success(), "run should fail on build error");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !combined.contains("Running badapp"),
        "must not run on failed build: {combined}"
    );
    assert!(exe.is_file());
}

#[test]
fn existing_package_build_commands_still_work() {
    let out = crux_cmd().args(["build", "--help"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("crux-build"));
}
