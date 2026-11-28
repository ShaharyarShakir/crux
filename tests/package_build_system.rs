//! M4: how a package definition's `[build] system` selects the package build
//! system, end to end through `crux install` and `crux build`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Workspace {
    root: PathBuf,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Workspace {
    /// A project depending on one header-only path package, `leaf`, whose
    /// definition ends with `build_section`.
    fn new(name: &str, build_section: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "crux_m4_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write(
            &root.join("leaf/CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.16)\nproject(leaf LANGUAGES NONE)\ninstall(FILES leaf.h DESTINATION include)\n",
        );
        write(&root.join("leaf/leaf.h"), "#pragma once\n");
        write(
            &root.join("leaf/package.toml"),
            &format!("[package]\nname = \"leaf\"\nversion = \"1.0.0\"\n{build_section}"),
        );
        write(
            &root.join("project/crux.toml"),
            "[package]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\nleaf = { path = \"../leaf\" }\n",
        );
        Self { root }
    }

    fn crux(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_crux"))
            .args(args)
            .current_dir(self.root.join("project"))
            .env("CRUX_HOME", self.root.join("home"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .output()
            .unwrap()
    }

    fn succeed(&self, args: &[&str]) -> String {
        let output = self.crux(args);
        assert!(output.status.success(), "{args:?}\n{}", text(&output));
        text(&output)
    }
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A configured CMake tree and an installed header show CMake did the work.
fn assert_built_and_installed_by_cmake(workspace: &Workspace) {
    let crux_dir = workspace.root.join("project/.crux");
    let only_child = |dir: PathBuf| fs::read_dir(dir).unwrap().next().unwrap().unwrap().path();
    let build_root = only_child(crux_dir.join("builds/leaf/1.0.0"));
    assert!(build_root.join("build/CMakeCache.txt").is_file());

    let fingerprint = build_root.file_name().unwrap();
    let installed = crux_dir.join("installed/leaf/1.0.0").join(fingerprint);
    assert!(installed.join("include/leaf.h").is_file());
}

#[test]
fn a_definition_without_a_build_section_is_built_with_cmake() {
    let workspace = Workspace::new("omitted", "");
    workspace.succeed(&["lock"]);
    workspace.succeed(&["install"]);
    assert_built_and_installed_by_cmake(&workspace);
}

#[test]
fn an_explicit_cmake_declaration_is_built_with_cmake() {
    let workspace = Workspace::new("explicit", "\n[build]\nsystem = \"cmake\"\n");
    workspace.succeed(&["lock"]);
    workspace.succeed(&["install"]);
    assert_built_and_installed_by_cmake(&workspace);
}

#[test]
fn an_unsupported_build_system_fails_clearly_without_building() {
    let workspace = Workspace::new("unsupported", "\n[build]\nsystem = \"meson\"\n");
    workspace.succeed(&["lock"]);

    // The package has a CMakeLists.txt, so a silent fallback would succeed.
    for command in ["install", "build"] {
        let output = workspace.crux(&[command]);
        assert!(!output.status.success(), "{command}\n{}", text(&output));
        assert!(
            text(&output).contains(
                "package 'leaf 1.0.0' declares build system 'meson', but Crux currently supports only 'cmake'"
            ),
            "{command}\n{}",
            text(&output)
        );
    }
    assert!(!workspace.root.join("project/.crux/builds").exists());
}
