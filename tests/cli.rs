use std::process::Command;

#[test]
fn test_cli_no_args_shows_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Crux — A source-first C/C++ package manager"));
    assert!(stdout.contains("Usage:"));
}

#[test]
fn test_cli_help_flag() {
    for flag in ["-h", "--help"] {
        let output = Command::new(env!("CARGO_BIN_EXE_crux"))
            .arg(flag)
            .output()
            .expect("failed to execute crux binary");

        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Crux — A source-first C/C++ package manager"));
    }
}

#[test]
fn test_cli_version_flag() {
    let expected_version = format!("crux {}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["-V", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_crux"))
            .arg(flag)
            .output()
            .expect("failed to execute crux binary");

        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stdout, expected_version);
    }
}

#[test]
fn test_cli_unrecognized_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("--unsupported")
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("error: unexpected argument '--unsupported'"));
}
