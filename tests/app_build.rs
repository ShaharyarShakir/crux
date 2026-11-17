use std::fs;
use std::path::PathBuf;
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
        let path = std::env::temp_dir().join(format!("crux_integ_app_build_{prefix}_{nanos}"));
        fs::create_dir_all(&path).expect("failed to create temporary test directory");
        Self { path }
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

fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn write_manifest(project_dir: &std::path::Path, contents: &str) {
    fs::write(project_dir.join("crux.toml"), contents).unwrap();
}

fn read_stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn read_stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn test_app_build_c_single_file() {
    let test_dir = TestDir::new("c_single");

    // 1. Initialize project
    let init_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "c-app", "--language", "c"])
        .current_dir(&test_dir.path)
        .output()
        .expect("crux init execution");
    assert!(init_out.status.success());

    let proj_dir = test_dir.path.join("c-app");

    // 2. Build application
    let build_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build execution");

    assert!(build_out.status.success());
    let stdout = String::from_utf8(build_out.stdout).unwrap();
    assert!(stdout.contains("Crux Application Build"));
    assert!(stdout.contains("Project:       c-app"));
    assert!(stdout.contains("Language:      C"));
    assert!(stdout.contains("Configuration: Debug"));
    assert!(stdout.contains("Build successful!"));

    // 3. Verify no root CMakeLists.txt was created
    assert!(!proj_dir.join("CMakeLists.txt").exists());

    // 4. Verify generated CMake and build artifacts
    let generated_cmake = proj_dir
        .join(".crux")
        .join("build")
        .join("generated")
        .join("CMakeLists.txt");
    assert!(generated_cmake.is_file());

    let exe_path = proj_dir
        .join(".crux")
        .join("build")
        .join("debug")
        .join("c-app");
    assert!(exe_path.is_file());

    // 5. Execute generated binary and verify output
    let run_out = Command::new(&exe_path)
        .output()
        .expect("execution of built C application binary");
    assert!(run_out.status.success());
    let run_stdout = String::from_utf8(run_out.stdout).unwrap();
    assert!(run_stdout.contains("Hello from c-app!"));
}

#[test]
fn test_app_build_c_multi_file_nested() {
    let test_dir = TestDir::new("c_multi");
    let proj_dir = test_dir.path.join("c-calc");
    fs::create_dir_all(proj_dir.join("src").join("modules")).unwrap();

    let manifest = r#"
[project]
name = "c-calc"
version = "0.1.0"
language = "c"
standard = "c17"
"#;
    fs::write(proj_dir.join("crux.toml"), manifest).unwrap();

    let calc_c = r#"
int add_nums(int a, int b) {
    return a + b;
}
"#;
    fs::write(proj_dir.join("src").join("modules").join("calc.c"), calc_c).unwrap();

    let main_c = r#"
#include <stdio.h>

int add_nums(int a, int b);

int main(void) {
    int res = add_nums(40, 2);
    printf("Result is %d\n", res);
    return 0;
}
"#;
    fs::write(proj_dir.join("src").join("main.c"), main_c).unwrap();

    let build_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build execution");

    assert!(build_out.status.success());
    let stdout = String::from_utf8(build_out.stdout).unwrap();
    assert!(stdout.contains("Source files:  2"));
    assert!(stdout.contains("Build successful!"));

    let exe_path = proj_dir
        .join(".crux")
        .join("build")
        .join("debug")
        .join("c-calc");
    assert!(exe_path.is_file());

    let run_out = Command::new(&exe_path)
        .output()
        .expect("execution of built binary");
    assert!(run_out.status.success());
    let run_stdout = String::from_utf8(run_out.stdout).unwrap();
    assert!(run_stdout.contains("Result is 42"));
}

#[test]
fn test_app_build_cpp_single_file() {
    let test_dir = TestDir::new("cpp_single");

    let init_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "cpp-app", "--language", "cpp"])
        .current_dir(&test_dir.path)
        .output()
        .expect("crux init execution");
    assert!(init_out.status.success());

    let proj_dir = test_dir.path.join("cpp-app");

    let build_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build execution");

    assert!(build_out.status.success());
    let stdout = String::from_utf8(build_out.stdout).unwrap();
    assert!(stdout.contains("Crux Application Build"));
    assert!(stdout.contains("Project:       cpp-app"));
    assert!(stdout.contains("Language:      C++"));
    assert!(stdout.contains("Configuration: Debug"));
    assert!(stdout.contains("Build successful!"));

    assert!(!proj_dir.join("CMakeLists.txt").exists());

    let exe_path = proj_dir
        .join(".crux")
        .join("build")
        .join("debug")
        .join("cpp-app");
    assert!(exe_path.is_file());

    let run_out = Command::new(&exe_path)
        .output()
        .expect("execution of built C++ application binary");
    assert!(run_out.status.success());
    let run_stdout = String::from_utf8(run_out.stdout).unwrap();
    assert!(run_stdout.contains("Hello from cpp-app!"));
}

#[test]
fn test_app_build_cpp_multi_file_nested() {
    let test_dir = TestDir::new("cpp_multi");
    let proj_dir = test_dir.path.join("cpp-service");
    fs::create_dir_all(proj_dir.join("src").join("utils")).unwrap();

    let manifest = r#"
[project]
name = "cpp-service"
version = "0.2.0"
language = "cpp"
standard = "c++20"
"#;
    fs::write(proj_dir.join("crux.toml"), manifest).unwrap();

    let helper_cpp = r#"
#include <string>

std::string get_greeting(const std::string& target) {
    return "Greetings, " + target + "!";
}
"#;
    fs::write(
        proj_dir.join("src").join("utils").join("helper.cpp"),
        helper_cpp,
    )
    .unwrap();

    let main_cpp = r#"
#include <iostream>
#include <string>

std::string get_greeting(const std::string& target);

int main() {
    std::cout << get_greeting("Crux Engineer") << std::endl;
    return 0;
}
"#;
    fs::write(proj_dir.join("src").join("main.cpp"), main_cpp).unwrap();

    let build_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build execution");

    assert!(build_out.status.success());
    let stdout = String::from_utf8(build_out.stdout).unwrap();
    assert!(stdout.contains("Source files:  2"));
    assert!(stdout.contains("Build successful!"));

    let exe_path = proj_dir
        .join(".crux")
        .join("build")
        .join("debug")
        .join("cpp-service");
    assert!(exe_path.is_file());

    let run_out = Command::new(&exe_path)
        .output()
        .expect("execution of built binary");
    assert!(run_out.status.success());
    let run_stdout = String::from_utf8(run_out.stdout).unwrap();
    assert!(run_stdout.contains("Greetings, Crux Engineer!"));
}

#[test]
fn test_app_build_debug_and_release_isolation() {
    let test_dir = TestDir::new("isolation");
    let proj_dir = test_dir.path.join("iso-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    let manifest = r#"
[project]
name = "iso-app"
version = "0.1.0"
language = "cpp"
standard = "c++20"
"#;
    fs::write(proj_dir.join("crux.toml"), manifest).unwrap();
    fs::write(
        proj_dir.join("src").join("main.cpp"),
        "#include <iostream>\nint main() { std::cout << \"iso ok\\n\"; return 0; }",
    )
    .unwrap();

    // 1. Build Debug
    let debug_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build debug");
    assert!(debug_out.status.success());
    let debug_exe = proj_dir
        .join(".crux")
        .join("build")
        .join("debug")
        .join("iso-app");
    assert!(debug_exe.is_file());

    // 2. Build Release
    let release_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["build", "--release"])
        .current_dir(&proj_dir)
        .output()
        .expect("crux build release");
    assert!(release_out.status.success());
    let release_exe = proj_dir
        .join(".crux")
        .join("build")
        .join("release")
        .join("iso-app");
    assert!(release_exe.is_file());

    // Both executables exist simultaneously in their respective profile directories
    assert!(debug_exe.is_file());
    assert!(release_exe.is_file());
}

#[test]
fn test_app_build_incremental_rebuild() {
    let test_dir = TestDir::new("incremental");
    let proj_dir = test_dir.path.join("inc-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    let manifest = r#"
[project]
name = "inc-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#;
    fs::write(proj_dir.join("crux.toml"), manifest).unwrap();

    let main_c = proj_dir.join("src").join("main.c");
    fs::write(
        &main_c,
        "#include <stdio.h>\nint main(void) { printf(\"v1\\n\"); return 0; }",
    )
    .unwrap();

    // Build v1
    let out1 = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("initial build");
    assert!(out1.status.success());

    let exe_path = proj_dir
        .join(".crux")
        .join("build")
        .join("debug")
        .join("inc-app");
    let run1 = Command::new(&exe_path).output().unwrap();
    assert!(String::from_utf8(run1.stdout).unwrap().contains("v1"));

    // Modify source to v2
    fs::write(
        &main_c,
        "#include <stdio.h>\nint main(void) { printf(\"v2\\n\"); return 0; }",
    )
    .unwrap();

    // Build v2 incrementally
    let out2 = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("incremental rebuild");
    assert!(out2.status.success());

    let run2 = Command::new(&exe_path).output().unwrap();
    assert!(String::from_utf8(run2.stdout).unwrap().contains("v2"));
}

#[test]
fn test_app_build_rejects_other_language() {
    let test_dir = TestDir::new("reject_other");
    let proj_dir = test_dir.path.join("other-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    let manifest = r#"
[project]
name = "other-app"
version = "0.1.0"
language = "other"
"#;
    fs::write(proj_dir.join("crux.toml"), manifest).unwrap();

    let build_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build execution");

    assert!(!build_out.status.success());
    assert_eq!(build_out.status.code(), Some(1));
    let stderr = String::from_utf8(build_out.stderr).unwrap();
    assert!(stderr.contains("language 'other' cannot be compiled"));
}

#[test]
fn test_app_build_missing_compiler_fails() {
    let test_dir = TestDir::new("missing_cc");
    let proj_dir = test_dir.path.join("cc-fail");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    let manifest = r#"
[project]
name = "cc-fail"
version = "0.1.0"
language = "c"
standard = "c17"
"#;
    fs::write(proj_dir.join("crux.toml"), manifest).unwrap();
    fs::write(
        proj_dir.join("src").join("main.c"),
        "int main(void) { return 0; }",
    )
    .unwrap();

    let build_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .env("CC", "/nonexistent/path/to/compiler_xyz")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build execution");

    assert!(!build_out.status.success());
    assert_eq!(build_out.status.code(), Some(1));
    let stderr = String::from_utf8(build_out.stderr).unwrap();
    assert!(stderr.contains("compiler") || stderr.contains("not found"));
}

#[test]
fn test_app_build_compilation_error_diagnostics() {
    let test_dir = TestDir::new("syntax_error");
    let proj_dir = test_dir.path.join("bad-syntax");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    let manifest = r#"
[project]
name = "bad-syntax"
version = "0.1.0"
language = "c"
standard = "c17"
"#;
    fs::write(proj_dir.join("crux.toml"), manifest).unwrap();
    fs::write(
        proj_dir.join("src").join("main.c"),
        "int main(void) { THIS IS INVALID SYNTAX; return 0; }",
    )
    .unwrap();

    let build_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build execution");

    assert!(!build_out.status.success());
    assert_eq!(build_out.status.code(), Some(1));
    let stderr = String::from_utf8(build_out.stderr).unwrap();
    assert!(stderr.contains("Build failed!") || stderr.contains("error"));
}

#[test]
fn test_app_build_discovers_added_and_removed_sources() {
    let test_dir = TestDir::new("add_remove");
    let proj_dir = test_dir.path.join("addrm-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "addrm-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
    );

    let main_c = proj_dir.join("src").join("main.c");
    fs::write(
        &main_c,
        "#include <stdio.h>\nint main(void) { printf(\"initial\\n\"); return 0; }\n",
    )
    .unwrap();

    let first = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("initial build");
    assert!(first.status.success(), "{}", read_stderr(&first));
    assert!(read_stdout(&first).contains("Source files:  1"));

    // Add a new source file and reference it from main.
    fs::write(
        proj_dir.join("src").join("extra.c"),
        "int extra_value(void) { return 42; }\n",
    )
    .unwrap();
    fs::write(
        &main_c,
        "#include <stdio.h>\nint extra_value(void);\nint main(void) { printf(\"value=%d\\n\", extra_value()); return 0; }\n",
    )
    .unwrap();

    let second = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("build after adding source");
    assert!(second.status.success(), "{}", read_stderr(&second));
    assert!(read_stdout(&second).contains("Source files:  2"));

    let generated = proj_dir
        .join(".crux")
        .join("build")
        .join("generated")
        .join("CMakeLists.txt");
    let generated_content = fs::read_to_string(&generated).unwrap();
    assert!(generated_content.contains("extra.c"));

    let exe_path = proj_dir
        .join(".crux")
        .join("build")
        .join("debug")
        .join("addrm-app");
    let run = Command::new(&exe_path).output().unwrap();
    assert!(read_stdout(&run).contains("value=42"));

    // Remove the extra source and stop referencing it.
    fs::remove_file(proj_dir.join("src").join("extra.c")).unwrap();
    fs::write(
        &main_c,
        "#include <stdio.h>\nint main(void) { printf(\"removed\\n\"); return 0; }\n",
    )
    .unwrap();

    let third = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("build after removing source");
    assert!(third.status.success(), "{}", read_stderr(&third));
    assert!(read_stdout(&third).contains("Source files:  1"));

    let generated_content = fs::read_to_string(&generated).unwrap();
    assert!(!generated_content.contains("extra.c"));

    let run = Command::new(&exe_path).output().unwrap();
    assert!(read_stdout(&run).contains("removed"));
}

#[test]
fn test_app_build_reconfigures_after_standard_change() {
    let test_dir = TestDir::new("standard_change");
    let proj_dir = test_dir.path.join("std-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "std-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
    );
    fs::write(
        proj_dir.join("src").join("main.c"),
        "int main(void) { return 0; }\n",
    )
    .unwrap();

    let first = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("build with c17");
    assert!(first.status.success(), "{}", read_stderr(&first));

    let generated = proj_dir
        .join(".crux")
        .join("build")
        .join("generated")
        .join("CMakeLists.txt");
    assert!(
        fs::read_to_string(&generated)
            .unwrap()
            .contains("set(CMAKE_C_STANDARD 17)")
    );

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "std-app"
version = "0.1.0"
language = "c"
standard = "c11"
"#,
    );

    let second = crux()
        .args(["build", "--verbose"])
        .current_dir(&proj_dir)
        .output()
        .expect("build with c11");
    assert!(second.status.success(), "{}", read_stderr(&second));
    let stdout = read_stdout(&second);
    assert!(stdout.contains("Standard:      C11"));
    assert!(
        fs::read_to_string(&generated)
            .unwrap()
            .contains("set(CMAKE_C_STANDARD 11)")
    );
}

#[test]
fn test_app_build_verbose_output() {
    let test_dir = TestDir::new("verbose");
    let proj_dir = test_dir.path.join("verbose-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "verbose-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
    );
    fs::write(
        proj_dir.join("src").join("main.c"),
        "int main(void) { return 0; }\n",
    )
    .unwrap();

    let out = crux()
        .args(["build", "--verbose"])
        .current_dir(&proj_dir)
        .output()
        .expect("verbose build");
    assert!(out.status.success(), "{}", read_stderr(&out));

    let stdout = read_stdout(&out);
    assert!(stdout.contains("Compiler Exe:"));
    assert!(stdout.contains("CMake Exe:"));
    assert!(stdout.contains("CMake configure output"));
    assert!(stdout.contains("CMake build output"));
    assert!(stdout.contains("src/main.c"));

    let logs_dir = proj_dir.join(".crux").join("build").join("logs");
    assert!(logs_dir.join("configure-debug.log").is_file());
    assert!(logs_dir.join("build-debug.log").is_file());
}

#[test]
fn test_app_build_force_wipes_profile_directory() {
    let test_dir = TestDir::new("force");
    let proj_dir = test_dir.path.join("force-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "force-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
    );
    fs::write(
        proj_dir.join("src").join("main.c"),
        "int main(void) { return 0; }\n",
    )
    .unwrap();

    let first = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("initial build");
    assert!(first.status.success(), "{}", read_stderr(&first));

    let profile_dir = proj_dir.join(".crux").join("build").join("debug");
    let marker = profile_dir.join("force-rebuild-marker");
    fs::write(&marker, "stale").unwrap();
    assert!(marker.is_file());

    let second = crux()
        .args(["build", "--force"])
        .current_dir(&proj_dir)
        .output()
        .expect("forced build");
    assert!(second.status.success(), "{}", read_stderr(&second));
    assert!(
        !marker.exists(),
        "forced build must discard the existing profile build tree"
    );
    assert!(profile_dir.join("force-app").is_file());
}

#[test]
fn test_app_build_writes_only_under_crux_directory() {
    let test_dir = TestDir::new("isolation_fs");
    let proj_dir = test_dir.path.join("clean-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "clean-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
    );
    let source_path = proj_dir.join("src").join("main.c");
    let source_content = "int main(void) { return 0; }\n";
    fs::write(&source_path, source_content).unwrap();

    let out = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("build");
    assert!(out.status.success(), "{}", read_stderr(&out));

    // Source files must never be modified.
    assert_eq!(fs::read_to_string(&source_path).unwrap(), source_content);

    // No CMake or build artifacts may appear in the project root.
    for forbidden in [
        "CMakeLists.txt",
        "CMakeCache.txt",
        "CMakeFiles",
        "build",
        "cmake_install.cmake",
        "Makefile",
    ] {
        assert!(
            !proj_dir.join(forbidden).exists(),
            "unexpected root-level artifact: {forbidden}"
        );
    }

    let mut root_entries: Vec<String> = fs::read_dir(&proj_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    root_entries.sort();
    assert_eq!(root_entries, vec![".crux", "crux.toml", "src"]);
}

#[test]
fn test_app_build_rejects_mixed_c_and_cpp_sources() {
    let test_dir = TestDir::new("mixed_sources");
    let proj_dir = test_dir.path.join("mixed-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "mixed-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
    );
    fs::write(
        proj_dir.join("src").join("main.c"),
        "int main(void) { return 0; }\n",
    )
    .unwrap();
    fs::write(
        proj_dir.join("src").join("helper.cpp"),
        "int helper() { return 1; }\n",
    )
    .unwrap();

    let out = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("mixed language build");
    assert!(!out.status.success());
    let stderr = read_stderr(&out);
    assert!(
        stderr.contains("mixing C and C++"),
        "unexpected diagnostic: {stderr}"
    );
    assert!(
        !proj_dir
            .join(".crux")
            .join("build")
            .join("generated")
            .join("CMakeLists.txt")
            .exists()
    );
}

#[test]
fn test_app_build_missing_source_directory_diagnostic() {
    let test_dir = TestDir::new("missing_src");
    let proj_dir = test_dir.path.join("no-src-app");
    fs::create_dir_all(&proj_dir).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "no-src-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
    );

    let out = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("build without src directory");
    assert!(!out.status.success());
    let stderr = read_stderr(&out);
    assert!(
        stderr.contains("source directory not found"),
        "unexpected diagnostic: {stderr}"
    );
}

#[test]
fn test_app_build_missing_cmake_diagnostic() {
    #[cfg(not(unix))]
    {
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let test_dir = TestDir::new("missing_cmake");
        let proj_dir = test_dir.path.join("no-cmake-app");
        fs::create_dir_all(proj_dir.join("src")).unwrap();

        write_manifest(
            &proj_dir,
            r#"
[project]
name = "no-cmake-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
        );
        fs::write(
            proj_dir.join("src").join("main.c"),
            "int main(void) { return 0; }\n",
        )
        .unwrap();

        // A fake compiler keeps toolchain detection hermetic while PATH is
        // reduced to a directory that contains no CMake.
        let fake_cc = test_dir.path.join("fake_cc");
        fs::write(
            &fake_cc,
            "#!/bin/sh\n\
             case \"$1\" in\n\
             --version) echo \"gcc (Fake) 13.0.0\";;\n\
             -dumpversion) echo \"13.0.0\";;\n\
             -dumpmachine) echo \"x86_64-unknown-linux-gnu\";;\n\
             *) exit 0;;\n\
             esac\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake_cc).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_cc, permissions).unwrap();

        let empty_path = test_dir.path.join("empty_path");
        fs::create_dir_all(&empty_path).unwrap();

        let out = crux()
            .arg("build")
            .env("CC", &fake_cc)
            .env("PATH", &empty_path)
            .current_dir(&proj_dir)
            .output()
            .expect("build without cmake");
        assert!(!out.status.success());
        let stderr = read_stderr(&out);
        assert!(
            stderr.contains("CMake is not available"),
            "unexpected diagnostic: {stderr}"
        );
    }
}

#[test]
fn test_app_build_respects_explicit_cxx() {
    let cxx = match find_tool("clang++").or_else(|| find_tool("g++")) {
        Some(path) => path,
        None => return,
    };
    let expected_family = if cxx
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains("clang"))
    {
        "Clang"
    } else {
        "GCC"
    };

    let test_dir = TestDir::new("explicit_cxx");
    let proj_dir = test_dir.path.join("cxx-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "cxx-app"
version = "0.1.0"
language = "cpp"
standard = "c++20"
"#,
    );
    fs::write(
        proj_dir.join("src").join("main.cpp"),
        "int main() { return 0; }\n",
    )
    .unwrap();

    let out = crux()
        .arg("build")
        .env("CXX", &cxx)
        .current_dir(&proj_dir)
        .output()
        .expect("explicit CXX build");
    assert!(out.status.success(), "{}", read_stderr(&out));
    let stdout = read_stdout(&out);
    assert!(
        stdout.contains(&format!("Compiler:    {expected_family}")),
        "unexpected compiler output: {stdout}"
    );

    let cache = fs::read_to_string(
        proj_dir
            .join(".crux")
            .join("build")
            .join("debug")
            .join("CMakeCache.txt"),
    )
    .unwrap();
    let compiler_name = cxx.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        cache.contains(&compiler_name),
        "CMake cache does not record the explicit CXX compiler"
    );
}

#[test]
fn test_app_build_reconfigures_when_compiler_changes() {
    let gcc = match find_tool("gcc") {
        Some(path) => path,
        None => return,
    };
    let clang = match find_tool("clang") {
        Some(path) => path,
        None => return,
    };

    let test_dir = TestDir::new("compiler_switch");
    let proj_dir = test_dir.path.join("switch-app");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[project]
name = "switch-app"
version = "0.1.0"
language = "c"
standard = "c17"
"#,
    );
    fs::write(
        proj_dir.join("src").join("main.c"),
        "int main(void) { return 0; }\n",
    )
    .unwrap();

    let first = crux()
        .arg("build")
        .env("CC", &gcc)
        .current_dir(&proj_dir)
        .output()
        .expect("gcc build");
    assert!(first.status.success(), "{}", read_stderr(&first));
    assert!(read_stdout(&first).contains("Compiler:    GCC"));

    let second = crux()
        .arg("build")
        .env("CC", &clang)
        .current_dir(&proj_dir)
        .output()
        .expect("clang build");
    assert!(second.status.success(), "{}", read_stderr(&second));
    assert!(read_stdout(&second).contains("Compiler:    Clang"));

    let cache = fs::read_to_string(
        proj_dir
            .join(".crux")
            .join("build")
            .join("debug")
            .join("CMakeCache.txt"),
    )
    .unwrap();
    assert!(
        cache.contains("clang"),
        "CMake cache was not invalidated after the compiler changed"
    );
}

#[test]
fn test_app_build_preserves_legacy_package_dispatch() {
    let test_dir = TestDir::new("legacy_dispatch");
    let proj_dir = test_dir.path.join("legacy-pkg");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    write_manifest(
        &proj_dir,
        r#"
[package]
name = "legacy-pkg"
version = "0.1.0"
"#,
    );
    fs::write(proj_dir.join("crux.lock"), "lockfile_version = 1\n").unwrap();
    // An application source file exists, but a legacy [package] manifest must
    // still build locked dependencies instead of the local application.
    fs::write(
        proj_dir.join("src").join("main.c"),
        "int main(void) { return 0; }\n",
    )
    .unwrap();

    let out = crux()
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("legacy package build");
    assert!(out.status.success(), "{}", read_stderr(&out));
    let stdout = read_stdout(&out);
    assert!(!stdout.contains("Crux Application Build"));
    assert!(stdout.contains("No dependencies to build in lockfile."));
    assert!(
        !proj_dir
            .join(".crux")
            .join("build")
            .join("generated")
            .exists()
    );
}

#[test]
fn test_app_build_in_path_with_spaces() {
    let parent = TestDir::new("space_parent");
    let proj_dir = parent
        .path
        .join("my folder with spaces")
        .join("spaced_proj");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    let manifest = r#"
[project]
name = "spaced_proj"
version = "0.1.0"
language = "cpp"
standard = "c++20"
"#;
    fs::write(proj_dir.join("crux.toml"), manifest).unwrap();
    fs::write(
        proj_dir.join("src").join("main.cpp"),
        "#include <iostream>\nint main() { std::cout << \"spaced ok\\n\"; return 0; }",
    )
    .unwrap();

    let build_out = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("build")
        .current_dir(&proj_dir)
        .output()
        .expect("crux build in spaced path");

    assert!(build_out.status.success());
    let exe_path = proj_dir
        .join(".crux")
        .join("build")
        .join("debug")
        .join("spaced_proj");
    assert!(exe_path.is_file());

    let run_out = Command::new(&exe_path).output().unwrap();
    assert!(
        String::from_utf8(run_out.stdout)
            .unwrap()
            .contains("spaced ok")
    );
}
