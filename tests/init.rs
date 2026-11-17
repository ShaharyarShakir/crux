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
        let path = std::env::temp_dir().join(format!("crux_integ_{prefix}_{nanos}"));
        fs::create_dir_all(&path).expect("failed to create temporary test directory");
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_init_in_empty_directory_creates_manifest() {
    let test_dir = TestDir::new("empty_dir");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("init")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Initialized Crux package"));

    let manifest_path = test_dir.path.join("crux.toml");
    assert!(manifest_path.exists());

    let content = fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("[project]"));
    assert!(content.contains("version = \"0.1.0\""));
    assert!(content.contains("language = \"cpp\""));
    assert!(content.contains("standard = \"c++20\""));

    assert!(test_dir.path.join("crux.lock").is_file());
    assert!(test_dir.path.join("src").join("main.cpp").is_file());
    assert!(!test_dir.path.join(".crux").exists());
    assert!(!test_dir.path.join("include").exists());
    assert!(!test_dir.path.join("tests").exists());
    assert!(!test_dir.path.join("CMakeLists.txt").exists());
}

#[test]
fn test_init_with_custom_name() {
    let test_dir = TestDir::new("custom_name");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--name", "cpp-math-engine"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Initialized Crux package 'cpp-math-engine'"));

    let manifest_path = test_dir.path.join("crux.toml");
    let content = fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("name = \"cpp-math-engine\""));
}

#[test]
fn test_init_rejects_when_manifest_already_exists() {
    let test_dir = TestDir::new("existing_manifest");
    let manifest_path = test_dir.path.join("crux.toml");
    fs::write(&manifest_path, "# existing manifest content\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("init")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("manifest already exists"));

    // Verify existing file was not overwritten
    let content = fs::read_to_string(&manifest_path).unwrap();
    assert_eq!(content, "# existing manifest content\n");
}

#[test]
fn test_init_rejects_invalid_custom_name() {
    let test_dir = TestDir::new("invalid_name");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--name", "invalid*name"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid character '*'"));

    assert!(!test_dir.path.join("crux.toml").exists());
}

#[test]
fn test_init_in_path_with_spaces() {
    let test_dir = TestDir::new("space_parent");
    let spaced_dir = test_dir.path.join("folder with spaces").join("my_cpp_lib");
    fs::create_dir_all(&spaced_dir).expect("failed to create directory with spaces");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("init")
        .current_dir(&spaced_dir)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Initialized Crux package 'my_cpp_lib'"));

    let manifest_path = spaced_dir.join("crux.toml");
    assert!(manifest_path.exists());

    let content = fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("name = \"my_cpp_lib\""));
}

#[test]
fn test_init_path_with_spaces_and_custom_name() {
    let test_dir = TestDir::new("space_custom");
    let spaced_dir = test_dir.path.join("folder with spaces");
    fs::create_dir_all(&spaced_dir).expect("failed to create directory with spaces");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--name", "explicit_name"])
        .current_dir(&spaced_dir)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());

    let manifest_path = spaced_dir.join("crux.toml");
    assert!(manifest_path.exists());
    let content = fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("name = \"explicit_name\""));
}

#[test]
fn test_init_c_project_via_cli() {
    let test_dir = TestDir::new("cli_c_proj");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--name", "c-sample", "--language", "c"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Initialized Crux package 'c-sample'"));

    let manifest = test_dir.path.join("crux.toml");
    assert!(manifest.is_file());
    let content = fs::read_to_string(&manifest).unwrap();
    assert!(content.contains("[project]"));
    assert!(content.contains("language = \"c\""));
    assert!(content.contains("standard = \"c17\""));

    let lockfile = test_dir.path.join("crux.lock");
    assert!(lockfile.is_file());

    let main_c = test_dir.path.join("src").join("main.c");
    assert!(main_c.is_file());
    let main_content = fs::read_to_string(&main_c).unwrap();
    assert!(main_content.contains("Hello from c-sample!"));

    assert!(!test_dir.path.join(".crux").exists());
    assert!(!test_dir.path.join("include").exists());
    assert!(!test_dir.path.join("tests").exists());
    assert!(!test_dir.path.join("src").join("modules").exists());
    assert!(!test_dir.path.join("CMakeLists.txt").exists());
}

#[test]
fn test_init_cpp_project_via_cli() {
    let test_dir = TestDir::new("cli_cpp_proj");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--name", "cpp-sample", "--language", "cpp"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Initialized Crux package 'cpp-sample'"));

    let manifest = test_dir.path.join("crux.toml");
    let content = fs::read_to_string(&manifest).unwrap();
    assert!(content.contains("[project]"));
    assert!(content.contains("language = \"cpp\""));
    assert!(content.contains("standard = \"c++20\""));

    assert!(test_dir.path.join("crux.lock").is_file());
    let main_cpp = test_dir.path.join("src").join("main.cpp");
    assert!(main_cpp.is_file());
    let main_content = fs::read_to_string(&main_cpp).unwrap();
    assert!(main_content.contains("Hello from cpp-sample!"));

    assert!(!test_dir.path.join(".crux").exists());
    assert!(!test_dir.path.join("include").exists());
    assert!(!test_dir.path.join("tests").exists());
    assert!(!test_dir.path.join("src").join("modules").exists());
    assert!(!test_dir.path.join("CMakeLists.txt").exists());
}

#[test]
fn test_init_other_project_via_cli() {
    let test_dir = TestDir::new("cli_other_proj");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--name", "other-sample", "--language", "other"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());

    let manifest = test_dir.path.join("crux.toml");
    let content = fs::read_to_string(&manifest).unwrap();
    assert!(content.contains("[project]"));
    assert!(content.contains("language = \"other\""));
    assert!(!content.contains("standard ="));

    assert!(test_dir.path.join("crux.lock").is_file());
    assert!(test_dir.path.join("src").is_dir());
    assert!(!test_dir.path.join("include").exists());
    assert!(!test_dir.path.join("tests").exists());
    assert!(!test_dir.path.join(".crux").exists());

    assert!(!test_dir.path.join("src").join("main.c").exists());
    assert!(!test_dir.path.join("src").join("main.cpp").exists());
    assert!(!test_dir.path.join("CMakeLists.txt").exists());
}

#[test]
fn test_init_with_positional_path_argument() {
    let test_dir = TestDir::new("cli_pos_arg");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "nested-app", "--language", "c"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());

    let nested_dir = test_dir.path.join("nested-app");
    assert!(nested_dir.is_dir());
    assert!(nested_dir.join("crux.toml").is_file());
    assert!(nested_dir.join("crux.lock").is_file());
    assert!(nested_dir.join("src").join("main.c").is_file());
    assert!(!nested_dir.join("include").exists());
    assert!(!nested_dir.join("tests").exists());
    assert!(!nested_dir.join(".crux").exists());
    assert!(!nested_dir.join("CMakeLists.txt").exists());
}

#[test]
fn test_init_with_standard_flag() {
    let test_dir = TestDir::new("cli_std_flag");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--language", "c", "--standard", "c11"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());

    let content = fs::read_to_string(test_dir.path.join("crux.toml")).unwrap();
    assert!(content.contains("standard = \"c11\""));
}

#[test]
fn test_init_rejects_invalid_language() {
    let test_dir = TestDir::new("cli_bad_lang");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--language", "rust"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unsupported project language 'rust'"));
}

#[test]
fn test_init_rejects_incompatible_standard() {
    let test_dir = TestDir::new("cli_bad_std");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--language", "c", "--standard", "c++20"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unsupported standard 'c++20' for language 'c'"));
}

#[test]
fn test_init_rejects_standard_on_other() {
    let test_dir = TestDir::new("cli_other_std");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--language", "other", "--standard", "c17"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("does not support a standard"));
}

#[test]
fn test_init_interactive_piped_selection() {
    use std::io::Write;
    use std::process::Stdio;

    let test_dir = TestDir::new("cli_piped_input");

    let mut child = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("init")
        .current_dir(&test_dir.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn crux binary");

    // Pipe "1\n" to select C
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(b"1\n").expect("failed to write to stdin");
    }

    let output = child
        .wait_with_output()
        .expect("failed to wait for process");
    assert!(output.status.success());

    let manifest_content = fs::read_to_string(test_dir.path.join("crux.toml")).unwrap();
    assert!(manifest_content.contains("language = \"c\""));
    assert!(test_dir.path.join("src").join("main.c").is_file());
}

#[test]
fn test_init_rejects_existing_lockfile_via_cli() {
    let test_dir = TestDir::new("cli_existing_lock");
    fs::write(test_dir.path.join("crux.lock"), "# existing lock").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["init", "--language", "cpp"])
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("file already exists"));
}
