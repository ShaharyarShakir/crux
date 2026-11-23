//! `[build]` manifest settings: custom compiler/linker flags, sanitizers,
//! unsupported-toolchain reporting, and build-cache invalidation.
//!
//! Manifest validation tests need no toolchain. End-to-end tests use the
//! host's real C compiler and CMake (skipped when absent) plus a wrapper
//! script to simulate a toolchain without sanitizer support.

#![cfg(unix)]

use crux::Manifest;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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
        let path = std::env::temp_dir().join(format!("crux_build_settings_{prefix}_{nanos}"));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn toolchain_available() -> bool {
    find_tool("cmake").is_some() && find_tool("gcc").is_some()
}

fn manifest(build_section: &str) -> String {
    format!(
        "[project]\nname = \"flagged\"\nversion = \"0.1.0\"\nlanguage = \"c\"\nstandard = \"c17\"\n\n[dependencies]\n\n{build_section}"
    )
}

fn write_project(dir: &Path, build_section: &str, main_c: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("crux.toml"), manifest(build_section)).unwrap();
    fs::write(dir.join("src/main.c"), main_c).unwrap();
}

fn crux_build(dir: &Path, extra: &[&str], env: &[(&str, &Path)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_crux"));
    command.arg("build").args(extra).current_dir(dir);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("crux build")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn executable(dir: &Path) -> PathBuf {
    dir.join(".crux/build/debug/flagged")
}

fn fingerprint(dir: &Path) -> String {
    fs::read_to_string(dir.join(".crux/build/debug/crux-build.fingerprint")).unwrap()
}

// ---- manifest parsing and validation (no toolchain needed) ----------------

#[test]
fn parses_full_build_section() {
    let manifest = Manifest::from_toml_str(&manifest(
        "[build]\nc_flags = [\"-Wall\", \"-Wextra\"]\ncxx_flags = [\"-Wpedantic\"]\n\
         linker_flags = [\"-Wl,--as-needed\"]\n\n[build.sanitizers]\naddress = true\nundefined = true\n",
    ))
    .unwrap();
    assert_eq!(manifest.build.c_flags, ["-Wall", "-Wextra"]);
    assert_eq!(manifest.build.cxx_flags, ["-Wpedantic"]);
    assert_eq!(manifest.build.linker_flags, ["-Wl,--as-needed"]);
    assert!(manifest.build.address_sanitizer && manifest.build.undefined_sanitizer);
}

#[test]
fn absent_build_section_is_the_default() {
    let manifest = Manifest::from_toml_str(&manifest("")).unwrap();
    assert!(manifest.build.is_default());
}

#[test]
fn malformed_build_sections_are_rejected() {
    let cases = [
        ("[build]\nc_flags = \"-Wall\"\n", "c_flags"),
        ("[build]\nc_flags = [1, 2]\n", "c_flags"),
        ("[build]\nc_flags = [\"\"]\n", "empty"),
        ("[build]\nc_flags = [\"-Wall \"]\n", "whitespace"),
        ("[build]\ncxx_flags = [\"-W\\nall\"]\n", "control character"),
        ("[build]\nlinker_flags = [\"  \"]\n", "empty"),
        ("[build]\nunknown_key = true\n", "unknown_key"),
        ("[build.sanitizers]\nthread = true\n", "thread"),
        ("[build.sanitizers]\naddress = \"yes\"\n", "address"),
    ];
    for (section, expected) in cases {
        let err = Manifest::from_toml_str(&manifest(section))
            .expect_err(section)
            .to_string();
        assert!(err.contains(expected), "{section:?}: {err}");
    }
}

#[test]
fn build_section_is_rejected_for_package_manifests() {
    let text = "[package]\nname = \"lib\"\nversion = \"0.1.0\"\n\n[build]\nc_flags = [\"-Wall\"]\n";
    let err = Manifest::from_toml_str(text).unwrap_err().to_string();
    assert!(err.contains("[project]"), "{err}");
}

#[test]
fn serialization_round_trips_build_settings() {
    let original = Manifest::from_toml_str(&manifest(
        "[build]\nc_flags = [\"-DNAME=a b\"]\n\n[build.sanitizers]\nundefined = true\n",
    ))
    .unwrap();
    let reparsed = Manifest::from_toml_str(&original.to_toml_string()).unwrap();
    assert_eq!(original.build, reparsed.build);
}

// ---- end-to-end builds ---------------------------------------------------

const PRINT_VALUE: &str =
    "#include <stdio.h>\nint main(void) { printf(\"%d\\n\", CRUX_VALUE); return 0; }\n";

#[test]
fn custom_c_flags_reach_the_compiler_with_argument_boundaries_intact() {
    if !toolchain_available() {
        return;
    }
    let dir = TestDir::new("cflags");
    // The second entry contains spaces and must stay one argument.
    write_project(
        &dir.path,
        "[build]\nc_flags = [\"-Wall\", \"-DCRUX_VALUE=1 + 2 * 3\"]\n",
        PRINT_VALUE,
    );
    let output = crux_build(&dir.path, &[], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let run = Command::new(executable(&dir.path)).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "7");
}

#[test]
fn flags_are_not_executed_by_a_shell() {
    if !toolchain_available() {
        return;
    }
    let dir = TestDir::new("noshell");
    let marker = dir.path.join("pwned");
    write_project(
        &dir.path,
        &format!(
            "[build]\nc_flags = [\"-DCRUX_VALUE=0\", \"-DNOTE=$(touch {m}) `touch {m}` ; touch {m}\"]\n",
            m = marker.display()
        ),
        PRINT_VALUE,
    );
    let output = crux_build(&dir.path, &[], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!marker.exists(), "a flag was interpreted by a shell");
}

#[test]
fn language_specific_flags_apply_only_to_the_projects_language() {
    if !toolchain_available() {
        return;
    }
    let dir = TestDir::new("langscope");
    // `cxx_flags` must be ignored for a C project, so CRUX_VALUE stays undefined
    // from c_flags and the build uses only the C value.
    write_project(
        &dir.path,
        "[build]\nc_flags = [\"-DCRUX_VALUE=5\"]\ncxx_flags = [\"-DCRUX_VALUE=9\"]\n",
        PRINT_VALUE,
    );
    let output = crux_build(&dir.path, &[], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let run = Command::new(executable(&dir.path)).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "5");
}

#[test]
#[cfg(target_os = "linux")]
fn linker_flags_reach_the_link_step() {
    if !toolchain_available() {
        return;
    }
    let dir = TestDir::new("ldflags");
    write_project(
        &dir.path,
        "[build]\nc_flags = [\"-DCRUX_VALUE=0\"]\nlinker_flags = [\"-Wl,-Map=crux-link.map\"]\n",
        PRINT_VALUE,
    );
    let output = crux_build(&dir.path, &[], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(dir.path.join(".crux/build/debug/crux-link.map").is_file());
}

#[test]
fn compiler_errors_from_custom_flags_are_preserved() {
    if !toolchain_available() {
        return;
    }
    let dir = TestDir::new("werror");
    write_project(
        &dir.path,
        "[build]\nc_flags = [\"-Werror=unused-variable\", \"-Wunused-variable\", \"-DCRUX_VALUE=0\"]\n",
        "#include <stdio.h>\nint main(void) { int unused_thing; printf(\"%d\\n\", CRUX_VALUE); return 0; }\n",
    );
    let output = crux_build(&dir.path, &["--color", "never"], &[]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("error: package compilation failed"), "{text}");
    assert!(
        text.contains("unused_thing"),
        "compiler output lost: {text}"
    );
    assert!(text.contains("main.c"), "source location lost: {text}");
}

#[test]
fn address_and_undefined_sanitizers_instrument_the_binary() {
    if !toolchain_available() {
        return;
    }
    let dir = TestDir::new("asan");
    write_project(
        &dir.path,
        "[build]\n\n[build.sanitizers]\naddress = true\nundefined = true\n",
        "#include <stdlib.h>\nint main(int argc, char **argv) {\n  (void)argv;\n  int *p = malloc(4 * sizeof(int));\n  int v = p[argc + 3];\n  free(p);\n  return v;\n}\n",
    );
    let output = crux_build(&dir.path, &[], &[]);
    assert!(output.status.success(), "{}", stderr(&output));

    let run = Command::new(executable(&dir.path))
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .unwrap();
    assert!(!run.status.success(), "ASan should abort the overflow");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("AddressSanitizer"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn unsupported_sanitizer_toolchain_stops_the_build_with_a_diagnostic() {
    let (Some(_), Some(gcc)) = (find_tool("cmake"), find_tool("gcc")) else {
        return;
    };
    let dir = TestDir::new("unsupported");
    let wrapper = dir.path.join("cc-without-sanitizers");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\ncase \"$*\" in *-fsanitize*) echo 'fake-cc: sanitizer runtime not installed' >&2; exit 1;; esac\nexec {} \"$@\"\n",
            gcc.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();

    let project = dir.path.join("proj");
    write_project(
        &project,
        "[build]\n\n[build.sanitizers]\naddress = true\n",
        PRINT_VALUE,
    );
    let output = crux_build(&project, &["--color", "never"], &[("CC", &wrapper)]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("error: requested sanitizer is not supported"),
        "{text}"
    );
    assert!(text.contains("AddressSanitizer"), "{text}");
    assert!(
        text.contains("fake-cc: sanitizer runtime not installed"),
        "compiler output must be preserved: {text}"
    );
    assert!(
        !project.join(".crux/build/debug/flagged").exists(),
        "no binary may be produced when the sanitizer is unavailable"
    );
}

// ---- cache correctness ---------------------------------------------------

#[test]
fn unchanged_settings_reuse_the_build_tree_and_changes_invalidate_it() {
    if !toolchain_available() {
        return;
    }
    let dir = TestDir::new("cache");
    write_project(
        &dir.path,
        "[build]\nc_flags = [\"-DCRUX_VALUE=1\"]\n",
        PRINT_VALUE,
    );
    assert!(crux_build(&dir.path, &[], &[]).status.success());
    let first = fingerprint(&dir.path);

    // A sentinel proves whether the build directory was discarded.
    let sentinel = dir.path.join(".crux/build/debug/sentinel");
    fs::write(&sentinel, "x").unwrap();

    assert!(crux_build(&dir.path, &[], &[]).status.success());
    assert!(sentinel.exists(), "identical settings must reuse the tree");
    assert_eq!(first, fingerprint(&dir.path));

    // Changing a flag discards the tree and produces a new fingerprint.
    fs::write(
        dir.path.join("crux.toml"),
        manifest("[build]\nc_flags = [\"-DCRUX_VALUE=2\"]\n"),
    )
    .unwrap();
    let output = crux_build(&dir.path, &[], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!sentinel.exists(), "changed flags must discard the tree");
    assert_ne!(first, fingerprint(&dir.path));
    let run = Command::new(executable(&dir.path)).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "2");

    // Enabling a sanitizer changes the fingerprint again.
    let before_sanitizer = fingerprint(&dir.path);
    fs::write(
        dir.path.join("crux.toml"),
        manifest(
            "[build]\nc_flags = [\"-DCRUX_VALUE=2\"]\n\n[build.sanitizers]\nundefined = true\n",
        ),
    )
    .unwrap();
    let output = crux_build(&dir.path, &[], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_ne!(before_sanitizer, fingerprint(&dir.path));
}

#[test]
fn build_trees_without_a_fingerprint_are_rebuilt_not_trusted() {
    if !toolchain_available() {
        return;
    }
    let dir = TestDir::new("legacy");
    write_project(&dir.path, "", &PRINT_VALUE.replace("CRUX_VALUE", "3"));
    assert!(crux_build(&dir.path, &[], &[]).status.success());

    // Simulate a tree created by an older Crux: cache present, no record.
    let record = dir.path.join(".crux/build/debug/crux-build.fingerprint");
    fs::remove_file(&record).unwrap();
    let sentinel = dir.path.join(".crux/build/debug/sentinel");
    fs::write(&sentinel, "x").unwrap();

    assert!(crux_build(&dir.path, &[], &[]).status.success());
    assert!(!sentinel.exists());
    assert!(record.is_file());
}

#[test]
fn dependency_edits_preserve_the_build_section() {
    use crux::package::Dependency;
    use crux::project::manifest::upsert_dependency_in_toml;

    let build = "[build]\nc_flags = [\"-Wall\"]\n\n[build.sanitizers]\naddress = true\n";
    // [dependencies] before [build], and no [dependencies] at all.
    for content in [
        format!(
            "[project]\nname = \"flagged\"\nversion = \"0.1.0\"\nlanguage = \"c\"\n\n[dependencies]\nfmt = \"^10\"\n\n{build}"
        ),
        format!("[project]\nname = \"flagged\"\nversion = \"0.1.0\"\nlanguage = \"c\"\n\n{build}"),
    ] {
        let dep = Dependency::try_new("zlib", "*").unwrap();
        let updated = upsert_dependency_in_toml(&content, &dep).unwrap();
        let before = Manifest::from_toml_str(&content).unwrap();
        let after = Manifest::from_toml_str(&updated).unwrap();
        assert_eq!(before.build, after.build, "{updated}");
        assert!(
            after
                .dependencies
                .iter()
                .any(|d| d.name().as_str() == "zlib")
        );
    }
}
