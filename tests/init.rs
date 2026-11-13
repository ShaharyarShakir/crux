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
    assert!(content.contains("[package]"));
    assert!(content.contains("version = \"0.1.0\""));
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
