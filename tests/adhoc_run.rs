//! Integration tests for manifest-free `crux run --bin` / `--sources`.

use std::fs;
use std::path::PathBuf;
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
        let path = std::env::temp_dir().join(format!("crux_adhoc_run_{prefix}_{nanos}"));
        fs::create_dir_all(&path).expect("failed to create temporary test directory");
        Self { path }
    }

    fn write(&self, name: &str, contents: &str) {
        let path = self.path.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create source parent");
        }
        fs::write(path, contents).expect("failed to write source file");
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_crux"))
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("failed to execute crux binary")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_adhoc_c_single_source_without_manifest() {
    let dir = TestDir::new("c_single");
    dir.write(
        "hello.c",
        "#include <stdio.h>\nint main(void) { printf(\"Hello from C\\n\"); return 0; }\n",
    );

    let output = dir.run(&["run", "--bin", "hello.c"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Crux Run (ad-hoc)"));
    assert!(stdout.contains("Hello from C"));
    assert!(!dir.path.join("crux.toml").exists());

    let run_root = dir.path.join(".crux").join("run");
    assert!(run_root.is_dir(), "ad-hoc artifacts were not isolated");
    let entries: Vec<_> = fs::read_dir(&run_root).unwrap().flatten().collect();
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0]
            .file_name()
            .to_string_lossy()
            .starts_with("hello-")
    );
}

#[test]
fn test_adhoc_cpp_single_source_without_manifest() {
    let dir = TestDir::new("cpp_single");
    dir.write(
        "hello.cpp",
        "#include <iostream>\nint main() { std::cout << \"Hello from C++\" << std::endl; }\n",
    );

    let output = dir.run(&["run", "--bin", "hello.cpp"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Language:      C++"));
    assert!(stdout.contains("Hello from C++"));
}

#[test]
fn test_adhoc_multi_file_with_header() {
    let dir = TestDir::new("multi_header");
    dir.write(
        "mathops.hpp",
        "#pragma once\nint add_numbers(int left, int right);\n",
    );
    dir.write(
        "mathops.cpp",
        "#include \"mathops.hpp\"\nint add_numbers(int left, int right) { return left + right; }\n",
    );
    dir.write(
        "main.cpp",
        "#include <iostream>\n#include \"mathops.hpp\"\n\
         int main() { std::cout << \"SUM=\" << add_numbers(19, 23) << std::endl; }\n",
    );

    let output = dir.run(&[
        "run",
        "--sources",
        "main.cpp,mathops.cpp,mathops.hpp",
        "--include-dir",
        ".",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Sources:       2"));
    assert!(stdout.contains("SUM=42"));
}

#[test]
fn test_adhoc_space_separated_sources_with_bin() {
    let dir = TestDir::new("space_sources");
    dir.write(
        "calculator.hpp",
        "#pragma once\nint add_numbers(int left, int right);\n",
    );
    dir.write(
        "calculator.cpp",
        "#include \"calculator.hpp\"\nint add_numbers(int left, int right) { return left + right; }\n",
    );
    dir.write(
        "main.cpp",
        "#include <iostream>\n#include \"calculator.hpp\"\n\
         int main() { std::cout << \"CALC=\" << add_numbers(19, 23) << std::endl; }\n",
    );

    let output = dir.run(&[
        "run",
        "--bin",
        "main.cpp",
        "--sources",
        "calculator.cpp",
        "calculator.hpp",
        "--include-dir",
        ".",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Sources:       2"));
    assert!(stdout.contains("CALC=42"));
}

#[test]
fn test_adhoc_missing_source_fails() {
    let dir = TestDir::new("missing_source");
    let output = dir.run(&["run", "--bin", "missing.c"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("ad-hoc source 'missing.c' was not found"));
    assert!(!dir.path.join(".crux").join("run").exists());
}

#[test]
fn test_adhoc_compile_error_fails_without_stale_execution() {
    let dir = TestDir::new("compile_error");
    dir.write(
        "flaky.c",
        "#include <stdio.h>\nint main(void) { printf(\"FIRST RUN\\n\"); return 0; }\n",
    );

    let first = dir.run(&["run", "--bin", "flaky.c"]);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("FIRST RUN"));

    dir.write("flaky.c", "int main(void) { this is not valid C }\n");
    let second = dir.run(&["run", "--bin", "flaky.c"]);
    assert!(!second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(!stdout.contains("Running "), "stale binary was executed");
    assert!(!stdout.contains("FIRST RUN"), "stale binary output leaked");
    assert!(
        stderr.contains("error: package compilation failed"),
        "{stderr}"
    );
    // The compiler's own diagnostic (file:line:column) is preserved verbatim.
    assert!(stderr.contains("flaky.c:1:"), "{stderr}");
    assert!(stderr.contains("--- original output"), "{stderr}");

    let run_root = dir.path.join(".crux").join("run");
    let mut stale_executables = Vec::new();
    for scratch in fs::read_dir(&run_root).unwrap().flatten() {
        let executable = scratch
            .path()
            .join(".crux")
            .join("build")
            .join("debug")
            .join("flaky");
        if executable.exists() {
            stale_executables.push(executable);
        }
    }
    assert!(
        stale_executables.is_empty(),
        "stale ad-hoc executable survived a failed rebuild: {stale_executables:?}"
    );
}

#[test]
fn test_adhoc_unsupported_extension_fails() {
    let dir = TestDir::new("unsupported");
    dir.write("notes.txt", "int main(void) { return 0; }\n");

    let output = dir.run(&["run", "--bin", "notes.txt"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unsupported ad-hoc source extension '.txt'"));
}

#[test]
fn test_adhoc_argument_forwarding_and_exit_status() {
    let dir = TestDir::new("args");
    dir.write(
        "args.c",
        "#include <stdio.h>\n#include <string.h>\n\
         int main(int argc, char **argv) {\n\
             if (argc > 1) {\n\
                 printf(\"ARG=%s\\n\", argv[1]);\n\
                 if (strcmp(argv[1], \"fail\") == 0) return 7;\n\
             }\n\
             return 0;\n\
         }\n",
    );

    let success = dir.run(&["run", "--bin", "args.c", "--", "--mode", "ok"]);
    assert!(
        success.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&success.stderr)
    );
    assert!(String::from_utf8_lossy(&success.stdout).contains("ARG=--mode"));

    let failure = dir.run(&["run", "--bin", "args.c", "--", "fail"]);
    assert_eq!(failure.status.code(), Some(7));
    assert!(String::from_utf8_lossy(&failure.stdout).contains("ARG=fail"));
}

#[test]
fn test_adhoc_project_run_still_requires_manifest_without_sources() {
    let dir = TestDir::new("project_mode");
    let output = dir.run(&["run"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("manifest not found"));
}
