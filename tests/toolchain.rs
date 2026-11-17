use std::process::Command;

#[test]
fn test_cli_toolchain_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["toolchain", "--help"])
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Display detected compiler and toolchain information"));
    assert!(stdout.contains("--language"));
}

#[test]
fn test_cli_toolchain_detect_both_by_default() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("toolchain")
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Crux Toolchain Information"));
    assert!(stdout.contains("Platform:"));
    assert!(stdout.contains("OS:"));
    assert!(stdout.contains("Architecture:"));
    assert!(stdout.contains("C Toolchain:"));
    assert!(stdout.contains("C++ Toolchain:"));
}

#[test]
fn test_cli_toolchain_detect_c() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["toolchain", "--language", "c"])
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Crux Toolchain Information"));
    assert!(stdout.contains("Platform:"));
    assert!(stdout.contains("C Toolchain:"));
    assert!(stdout.contains("Compiler:"));
    assert!(stdout.contains("Version:"));
    assert!(stdout.contains("Target:"));
    assert!(stdout.contains("Executable:"));
    assert!(stdout.contains("Standards:"));
}

#[test]
fn test_cli_toolchain_detect_cpp() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["toolchain", "--language", "cpp"])
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Crux Toolchain Information"));
    assert!(stdout.contains("Platform:"));
    assert!(stdout.contains("C++ Toolchain:"));
    assert!(stdout.contains("Compiler:"));
    assert!(stdout.contains("Version:"));
    assert!(stdout.contains("Target:"));
    assert!(stdout.contains("Executable:"));
    assert!(stdout.contains("Standards:"));
}

#[test]
fn test_cli_toolchain_detect_other() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["toolchain", "--language", "other"])
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Crux Toolchain Information"));
    assert!(stdout.contains("Other Language:"));
    assert!(stdout.contains("Compiler detection not applicable"));
}

#[test]
fn test_cli_toolchain_missing_compiler_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["toolchain", "--language", "c"])
        .env("CC", "/nonexistent/path/to/cc_xyz_999")
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("compiler") || stderr.contains("not found"));
}

#[test]
fn test_cli_toolchain_invalid_language_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["toolchain", "--language", "rust"])
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unsupported project language 'rust'"));
}
