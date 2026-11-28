//! Dependency exposure to package builds, and dependency-aware fingerprints.
//!
//! The real `crux` binary builds temporary local path packages with CMake, a C
//! compiler and pkg-config, against an isolated registry. Nothing is fetched.

#![cfg(unix)]

mod common;

use common::{configure_registry, init_fixture_registry};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(1);

/// Written by every fixture package at configure time.
const RECORD_SEARCH_PATHS: &str = r#"file(WRITE "${CMAKE_BINARY_DIR}/seen.txt" "prefix=$ENV{CMAKE_PREFIX_PATH}\nmodule=${CMAKE_MODULE_PATH}\npkg=$ENV{PKG_CONFIG_PATH}\n")
"#;

struct Workspace {
    root: PathBuf,
    project: PathBuf,
    registry: PathBuf,
    env: Vec<(&'static str, &'static str)>,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Workspace {
    fn new(prefix: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "crux_exposure_{prefix}_{}_{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        Self {
            project: root.join("app"),
            registry: init_fixture_registry(&root.join("registry")),
            root,
            env: Vec::new(),
        }
    }

    /// `app -> mid -> leaf`, where `mid_value()` is `leaf_value() + 1`.
    fn chain(prefix: &str) -> Self {
        let ws = Self::new(prefix);
        ws.leaf(10);
        ws.library("mid", &["leaf"], FIND_BY_PREFIX);
        ws.app("mid", &["mid"]);
        ws
    }

    /// A static library installing a header, a CMake find module and a
    /// relocatable pkg-config file.
    fn leaf(&self, value: u32) {
        let dir = self.root.join("leaf");
        write(
            &dir.join("include/leaf.h"),
            "#pragma once\nint leaf_value(void);\n",
        );
        self.set_leaf_value(value);
        write(
            &dir.join("FindLeaf.cmake"),
            "find_path(Leaf_INCLUDE_DIR leaf.h REQUIRED)\nfind_library(Leaf_LIBRARY leaf REQUIRED)\nset(Leaf_FOUND TRUE)\n",
        );
        write(
            &dir.join("leaf.pc"),
            "prefix=${pcfiledir}/../..\nName: leaf\nDescription: leaf\nVersion: 1.0.0\nCflags: -I${prefix}/include\nLibs: -L${prefix}/lib -lleaf\n",
        );
        write(
            &dir.join("CMakeLists.txt"),
            &format!(
                "cmake_minimum_required(VERSION 3.16)\nproject(leaf LANGUAGES C)\n{RECORD_SEARCH_PATHS}add_library(leaf STATIC lib.c)\n\
                 install(TARGETS leaf ARCHIVE DESTINATION lib)\ninstall(FILES include/leaf.h DESTINATION include)\n\
                 install(FILES FindLeaf.cmake DESTINATION share/cmake/Modules)\ninstall(FILES leaf.pc DESTINATION lib/pkgconfig)\n"
            ),
        );
        self.package_toml("leaf", &[]);
    }

    fn set_leaf_value(&self, value: u32) {
        write(
            &self.root.join("leaf/lib.c"),
            &format!("int leaf_value(void) {{ return {value}; }}\n"),
        );
    }

    /// A static library whose `<name>_value()` adds one to its first
    /// dependency's, discovering `dependencies[0]` with `discovery`. A check
    /// executable makes a missing library fail the package's own build.
    fn library(&self, name: &str, dependencies: &[&str], discovery: &str) {
        let dir = self.root.join(name);
        let below = dependencies[0];
        write(
            &dir.join(format!("include/{name}.h")),
            &format!("#pragma once\nint {name}_value(void);\n"),
        );
        write(
            &dir.join("lib.c"),
            &format!(
                "#include <{below}.h>\nint {name}_value(void) {{ return {below}_value() + 1; }}\n"
            ),
        );
        write(
            &dir.join("check.c"),
            &format!("#include <{name}.h>\nint main(void) {{ return {name}_value() == 0; }}\n"),
        );
        let mut cmake = format!(
            "cmake_minimum_required(VERSION 3.16)\nproject({name} LANGUAGES C)\n{RECORD_SEARCH_PATHS}{discovery}\nadd_library({name} STATIC lib.c)\n\
             target_include_directories({name} PRIVATE include ${{DEP_INCLUDES}})\ntarget_link_libraries({name} PUBLIC ${{DEP_LIBRARIES}})\n\
             add_executable({name}_check check.c)\ntarget_include_directories({name}_check PRIVATE include)\ntarget_link_libraries({name}_check PRIVATE {name})\n\
             install(TARGETS {name} ARCHIVE DESTINATION lib)\ninstall(FILES include/{name}.h DESTINATION include)\n"
        );
        // Every dependency must be findable, not only the one that is linked.
        for dependency in dependencies {
            cmake.push_str(&format!(
                "find_library({dependency}_FOUND_LIB {dependency} REQUIRED)\n"
            ));
        }
        // Link what lies further down, found only through its own prefix.
        if dependencies.len() > 1 {
            for dependency in &dependencies[1..] {
                cmake.push_str(&format!(
                    "target_link_libraries({name} PUBLIC ${{{dependency}_FOUND_LIB}})\n"
                ));
            }
        }
        write(&dir.join("CMakeLists.txt"), &cmake);
        // Only the first dependency is declared; the rest arrive transitively.
        self.package_toml(name, &dependencies[..1]);
    }

    fn header_only(&self, name: &str) {
        let dir = self.root.join(name);
        write(
            &dir.join(format!("include/{name}.h")),
            &format!("#pragma once\nstatic inline int {name}_value(void) {{ return 5; }}\n"),
        );
        write(
            &dir.join("CMakeLists.txt"),
            &format!(
                "cmake_minimum_required(VERSION 3.16)\nproject({name} LANGUAGES NONE)\n{RECORD_SEARCH_PATHS}install(FILES include/{name}.h DESTINATION include)\n"
            ),
        );
        self.package_toml(name, &[]);
    }

    fn package_toml(&self, name: &str, dependencies: &[&str]) {
        write(
            &self.root.join(name).join("package.toml"),
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n\n[build]\nsystem = \"cmake\"\n\n[dependencies]\n{}",
                path_dependencies(dependencies)
            ),
        );
    }

    /// An application depending on `direct`, printing each value in `printed`.
    fn app(&self, direct: &str, printed: &[&str]) {
        write(
            &self.project.join("crux.toml"),
            &format!(
                "[project]\nname = \"app\"\nversion = \"0.1.0\"\nlanguage = \"c\"\n\n[dependencies]\n{}",
                path_dependencies(&[direct])
            ),
        );
        let mut main = String::from("#include <stdio.h>\n");
        for name in printed {
            main.push_str(&format!("#include <{name}.h>\n"));
        }
        main.push_str("int main(void) {\n");
        for name in printed {
            main.push_str(&format!("    printf(\"{name}=%d\\n\", {name}_value());\n"));
        }
        main.push_str("    return 0;\n}\n");
        write(&self.project.join("src/main.c"), &main);
    }

    fn try_crux(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crux"));
        configure_registry(
            &mut command,
            &self.registry,
            &self.project.join(".crux/config.toml"),
        );
        command
            .env("CRUX_HOME", self.root.join("home"))
            .env_remove("CMAKE_PREFIX_PATH")
            .env_remove("PKG_CONFIG_PATH")
            .envs(self.env.iter().copied())
            .args(args)
            .current_dir(&self.project)
            .output()
            .unwrap()
    }

    fn crux(&self, args: &[&str]) -> String {
        let output = self.try_crux(args);
        assert!(output.status.success(), "{args:?}\n{}", text(&output));
        text(&output)
    }

    fn version_dir(&self, name: &str) -> PathBuf {
        self.project
            .join(".crux/installed")
            .join(name)
            .join("1.0.0")
    }

    fn record_path(&self, name: &str) -> PathBuf {
        self.version_dir(name).join("current-debug.toml")
    }

    fn record(&self, name: &str) -> toml::Table {
        toml::from_str(&fs::read_to_string(self.record_path(name)).unwrap()).unwrap()
    }

    /// Hex digest of the current debug installation.
    fn current(&self, name: &str) -> String {
        self.record(name)["fingerprint"]
            .as_str()
            .unwrap()
            .trim_start_matches("sha256:")
            .to_string()
    }

    fn prefix(&self, name: &str) -> String {
        self.version_dir(name)
            .join(self.current(name))
            .display()
            .to_string()
    }

    /// What the current build of `name` saw at configure time.
    fn seen(&self, name: &str) -> String {
        fs::read_to_string(
            self.project
                .join(".crux/builds")
                .join(name)
                .join("1.0.0")
                .join(self.current(name))
                .join("build/seen.txt"),
        )
        .unwrap()
    }

    fn installs(&self, name: &str) -> usize {
        fs::read_dir(self.version_dir(name))
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().is_dir())
            .count()
    }
}

const FIND_BY_PREFIX: &str = r#"find_path(LEAF_INCLUDE leaf.h REQUIRED)
find_library(LEAF_LIB leaf REQUIRED)
set(DEP_INCLUDES ${LEAF_INCLUDE})
set(DEP_LIBRARIES ${LEAF_LIB})"#;

const FIND_BY_MODULE: &str = r#"find_package(Leaf MODULE REQUIRED)
set(DEP_INCLUDES ${Leaf_INCLUDE_DIR})
set(DEP_LIBRARIES ${Leaf_LIBRARY})"#;

// `PKG_CONFIG_USE_CMAKE_PREFIX_PATH` off, so only PKG_CONFIG_PATH can work.
const FIND_BY_PKG_CONFIG: &str = r#"set(PKG_CONFIG_USE_CMAKE_PREFIX_PATH OFF)
find_package(PkgConfig REQUIRED)
pkg_check_modules(LEAF REQUIRED IMPORTED_TARGET leaf)
set(DEP_INCLUDES ${LEAF_INCLUDE_DIRS})
set(DEP_LIBRARIES PkgConfig::LEAF)"#;

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn path_dependencies(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("{name} = {{ path = \"../{name}\" }}\n"))
        .collect()
}

fn text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_has(output: &str, needle: &str) {
    assert!(output.contains(needle), "missing {needle:?} in:\n{output}");
}

fn assert_lacks(output: &str, needle: &str) {
    assert!(
        !output.contains(needle),
        "unexpected {needle:?} in:\n{output}"
    );
}

#[test]
fn a_changed_leaf_rebuilds_mid_and_the_application() {
    let ws = Workspace::chain("chain");
    let first = ws.crux(&["run"]);
    assert_has(&first, "mid=11");
    assert!(first.find("[installed] leaf").unwrap() < first.find("[installed] mid").unwrap());

    let leaf = ws.prefix("leaf");
    let seen = ws.seen("mid");
    assert_has(&seen, &format!("prefix={leaf}\n"));
    assert_has(&seen, &format!("module={leaf}/share/cmake/Modules\n"));
    assert_has(&seen, &format!("pkg={leaf}/lib/pkgconfig\n"));
    // A package without dependencies is given nothing.
    assert_eq!(ws.seen("leaf"), "prefix=\nmodule=\npkg=\n");

    // The record names the installation mid was built against.
    let recorded = |ws: &Workspace| {
        ws.record("mid")["dependencies"]["leaf"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        recorded(&ws),
        format!("1.0.0@sha256:{}", ws.current("leaf"))
    );
    assert!(ws.record("leaf").get("dependencies").is_none());

    let (old_leaf, old_mid) = (ws.current("leaf"), ws.current("mid"));
    let unchanged = ws.crux(&["run"]);
    assert_has(&unchanged, "Dependencies:  2 installed [debug]");
    assert_lacks(&unchanged, "Installing");
    assert_eq!(ws.current("mid"), old_mid);

    ws.set_leaf_value(20);
    let changed = ws.crux(&["run"]);
    assert_has(&changed, "changed: leaf");
    assert_has(&changed, "outdated: mid 1.0.0 (its dependencies changed)");
    assert_has(&changed, "[installed] mid");
    assert_has(&changed, "mid=21");
    assert_ne!(ws.current("leaf"), old_leaf);
    assert_ne!(ws.current("mid"), old_mid);
    assert_has(&ws.seen("mid"), &ws.prefix("leaf"));

    // Reverting selects both older installations again, without compiling.
    ws.set_leaf_value(10);
    let reverted = ws.crux(&["run"]);
    assert_has(&reverted, "[cached]    leaf");
    assert_has(&reverted, "[cached]    mid");
    assert_has(&reverted, "mid=11");
    assert_eq!(ws.current("mid"), old_mid);
    assert_eq!((ws.installs("leaf"), ws.installs("mid")), (2, 2));
}

#[test]
fn transitive_dependencies_are_exposed_and_their_changes_propagate() {
    let ws = Workspace::new("transitive");
    ws.leaf(10);
    ws.library("mid", &["leaf"], FIND_BY_PREFIX);
    // top declares only mid, and also finds and links leaf.
    ws.library(
        "top",
        &["mid", "leaf"],
        "find_path(MID_INCLUDE mid.h REQUIRED)\nfind_library(MID_LIB mid REQUIRED)\n\
         set(DEP_INCLUDES ${MID_INCLUDE})\nset(DEP_LIBRARIES ${MID_LIB})",
    );
    ws.app("top", &["top"]);

    assert_has(&ws.crux(&["run"]), "top=12");
    let expected = format!("prefix={}:{}\n", ws.prefix("leaf"), ws.prefix("mid"));
    assert_has(&ws.seen("top"), &expected);
    let old_top = ws.current("top");

    ws.set_leaf_value(30);
    let changed = ws.crux(&["run"]);
    assert_has(&changed, "outdated: mid");
    assert_has(&changed, "outdated: top");
    assert_has(&changed, "top=32");
    assert_ne!(ws.current("top"), old_top);
}

#[test]
fn cmake_modules_of_a_dependency_are_found() {
    let ws = Workspace::new("module");
    ws.leaf(10);
    ws.library("mid", &["leaf"], FIND_BY_MODULE);
    ws.app("mid", &["mid"]);
    assert_has(&ws.crux(&["run"]), "mid=11");
}

#[test]
fn an_exported_cmake_package_of_a_dependency_is_found() {
    let ws = Workspace::new("config");
    ws.leaf(10);
    write(
        &ws.root.join("leaf/CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.16)\nproject(leaf LANGUAGES C)\nadd_library(leaf STATIC lib.c)\n\
         target_include_directories(leaf PUBLIC $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include> $<INSTALL_INTERFACE:include>)\n\
         install(TARGETS leaf EXPORT leafTargets ARCHIVE DESTINATION lib)\ninstall(FILES include/leaf.h DESTINATION include)\n\
         install(EXPORT leafTargets FILE leafConfig.cmake DESTINATION lib/cmake/leaf)\n",
    );
    ws.library(
        "mid",
        &["leaf"],
        "find_package(leaf CONFIG REQUIRED)\nset(DEP_LIBRARIES leaf)",
    );
    ws.app("mid", &["mid"]);
    assert_has(&ws.crux(&["run"]), "mid=11");
}

#[test]
fn pkg_config_files_of_a_dependency_are_found() {
    let ws = Workspace::new("pkgconfig");
    ws.leaf(10);
    ws.library("mid", &["leaf"], FIND_BY_PKG_CONFIG);
    ws.app("mid", &["mid"]);
    assert_has(&ws.crux(&["run"]), "mid=11");
}

#[test]
fn several_header_only_dependencies_are_ordered_and_keep_existing_paths() {
    let mut ws = Workspace::new("headers");
    ws.env = vec![
        ("CMAKE_PREFIX_PATH", "/crux-test/ambient"),
        ("PKG_CONFIG_PATH", "/crux-test/ambient/pc"),
    ];
    ws.header_only("zeta");
    ws.header_only("alpha");
    let top = ws.root.join("top");
    write(
        &top.join("CMakeLists.txt"),
        &format!(
            "cmake_minimum_required(VERSION 3.16)\nproject(top LANGUAGES C)\n{RECORD_SEARCH_PATHS}\
             find_path(ZETA zeta.h REQUIRED)\nfind_path(ALPHA alpha.h REQUIRED)\nadd_library(top STATIC lib.c)\n\
             target_include_directories(top PRIVATE ${{ZETA}} ${{ALPHA}})\n\
             install(TARGETS top ARCHIVE DESTINATION lib)\ninstall(FILES include/top.h DESTINATION include)\n"
        ),
    );
    write(
        &top.join("include/top.h"),
        "#pragma once\nint top_value(void);\n",
    );
    write(
        &top.join("lib.c"),
        "#include <alpha.h>\n#include <zeta.h>\nint top_value(void) { return alpha_value() + zeta_value(); }\n",
    );
    ws.package_toml("top", &["zeta", "alpha"]);
    ws.app("top", &["top"]);

    assert_has(&ws.crux(&["run"]), "top=10");
    // Crux prefixes first, by name; no module or pkg-config entries are added.
    assert_eq!(
        ws.seen("top"),
        format!(
            "prefix={}:{}:/crux-test/ambient\nmodule=\npkg=/crux-test/ambient/pc\n",
            ws.prefix("alpha"),
            ws.prefix("zeta")
        )
    );
    assert_eq!(
        ws.seen("alpha"),
        "prefix=/crux-test/ambient\nmodule=\npkg=/crux-test/ambient/pc\n"
    );
}

#[test]
fn a_failed_dependent_rebuild_is_retried_instead_of_using_the_old_build() {
    let ws = Workspace::chain("failure");
    ws.crux(&["run"]);

    // leaf still builds, but no longer provides what mid calls.
    write(
        &ws.root.join("leaf/lib.c"),
        "int renamed(void) { return 1; }\n",
    );
    for _ in 0..2 {
        let output = ws.try_crux(&["run"]);
        assert!(!output.status.success(), "{}", text(&output));
        assert_lacks(&text(&output), "mid=11");
    }

    ws.set_leaf_value(40);
    assert_has(&ws.crux(&["run"]), "mid=41");
}

#[test]
fn a_dependency_installed_on_its_own_still_rebuilds_its_dependents() {
    let ws = Workspace::chain("separate");
    ws.crux(&["run"]);

    ws.set_leaf_value(50);
    ws.crux(&["install", "leaf", "--profile", "debug"]);
    let output = ws.crux(&["run"]);
    assert_lacks(&output, "changed: leaf");
    assert_has(&output, "outdated: mid");
    assert_has(&output, "mid=51");
}

#[test]
fn an_installation_with_unknown_dependencies_is_not_trusted() {
    let ws = Workspace::chain("unknown");
    ws.crux(&["run"]);
    let mid = ws.current("mid");

    // A record from before dependencies were recorded.
    let mut record = ws.record("mid");
    record.remove("dependencies");
    fs::write(ws.record_path("mid"), toml::to_string(&record).unwrap()).unwrap();
    let output = ws.crux(&["run"]);
    assert_has(&output, "outdated: mid");
    assert_has(&output, "[cached]    mid");
    assert_eq!(ws.current("mid"), mid);
    assert!(ws.record("mid").contains_key("dependencies"));

    // No record at all, as left by a pre-record Crux.
    fs::remove_file(ws.record_path("mid")).unwrap();
    let output = ws.crux(&["run"]);
    assert_has(&output, "unrecorded: mid");
    assert_has(&output, "[cached]    mid");
    assert!(ws.record("mid").contains_key("dependencies"));
    assert_lacks(&ws.crux(&["run"]), "Installing");
}

#[test]
fn building_one_package_installs_its_dependencies_first() {
    let ws = Workspace::chain("single");

    let first = ws.crux(&["build", "mid", "--profile", "debug"]);
    assert_has(&first, "[installed] leaf 1.0.0");
    assert_has(&first, "compiled successfully");
    assert!(ws.record_path("leaf").is_file());
    // Only prerequisites are installed.
    assert!(!ws.version_dir("mid").exists());

    let again = ws.crux(&["build", "mid", "--profile", "debug"]);
    assert_has(&again, "[cached]    leaf 1.0.0");
    assert_has(&again, "reused cached build");

    // An edited dependency is reinstalled and the package rebuilt against it.
    let leaf = ws.current("leaf");
    ws.set_leaf_value(70);
    let changed = ws.crux(&["build", "mid", "--profile", "debug"]);
    assert_has(&changed, "[installed] leaf 1.0.0");
    assert_has(&changed, "compiled successfully");
    assert_ne!(ws.current("leaf"), leaf);

    ws.crux(&["install", "mid", "--profile", "debug"]);
    assert_has(&ws.seen("mid"), &ws.prefix("leaf"));
}

#[test]
fn building_one_package_verifies_locked_dependency_sources() {
    let ws = Workspace::chain("locked");
    let repo = ws.root.join("leaf");
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "Crux Test")
            .env("GIT_AUTHOR_EMAIL", "crux-test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Crux Test")
            .env("GIT_COMMITTER_EMAIL", "crux-test@example.invalid")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "initial"]);
    write(
        &ws.root.join("mid/package.toml"),
        &format!(
            "[package]\nname = \"mid\"\nversion = \"1.0.0\"\n\n[build]\nsystem = \"cmake\"\n\n\
             [dependencies]\nleaf = {{ git = \"{}\", branch = \"main\" }}\n",
            repo.display()
        ),
    );
    ws.crux(&["lock"]);

    // Record a different digest for the Git dependency only.
    let path = ws.project.join("crux.lock");
    let mut lockfile: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let leaf = lockfile["package"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|package| package["name"].as_str() == Some("leaf"))
        .unwrap();
    leaf.as_table_mut().unwrap().insert(
        "checksum".to_string(),
        format!("sha256:{}", "0".repeat(64)).into(),
    );
    fs::write(&path, toml::to_string(&lockfile).unwrap()).unwrap();

    let output = ws.try_crux(&["build", "mid", "--profile", "debug"]);
    assert!(!output.status.success(), "{}", text(&output));
    assert_has(&text(&output), "checksum");
    assert!(!ws.version_dir("leaf").exists());
    assert!(!ws.project.join(".crux/builds/mid").exists());

    // Neither is a lockfile that cannot be read skipped over.
    fs::write(&path, "not a lockfile {").unwrap();
    let output = ws.try_crux(&["build", "mid", "--profile", "debug"]);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(!ws.version_dir("leaf").exists());
}

#[test]
fn a_changed_inherited_search_path_is_not_served_from_the_cache() {
    let mut ws = Workspace::chain("ambient");
    let build = ["build", "mid", "--profile", "debug"];
    let fingerprint = |output: &str| {
        output
            .lines()
            .find_map(|line| line.trim().strip_prefix("Fingerprint:"))
            .unwrap()
            .trim()
            .to_string()
    };
    let unset = fingerprint(&ws.crux(&build));

    let mut seen = std::collections::BTreeSet::from([unset.clone()]);
    for env in [
        vec![("CMAKE_PREFIX_PATH", "")],
        vec![("CMAKE_PREFIX_PATH", "/crux-test/a:/crux-test/b")],
        vec![("CMAKE_PREFIX_PATH", "/crux-test/b:/crux-test/a")],
        vec![("PKG_CONFIG_PATH", "/crux-test/a")],
    ] {
        ws.env = env;
        let output = ws.crux(&build);
        assert_has(&output, "compiled successfully");
        assert!(seen.insert(fingerprint(&output)), "reused for {:?}", ws.env);
        assert_has(&ws.crux(&build), "reused cached build");
    }

    ws.env = Vec::new();
    assert_eq!(fingerprint(&ws.crux(&build)), unset);
}

#[test]
fn a_lockfile_build_installs_dependencies_before_their_dependents() {
    let ws = Workspace::chain("lockfile");
    write(
        &ws.project.join("crux.toml"),
        &format!(
            "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\n\n[dependencies]\n{}",
            path_dependencies(&["mid"])
        ),
    );
    ws.crux(&["lock"]);

    let first = ws.crux(&["build", "--profile", "debug"]);
    assert_has(&first, "2 compiled, 0 reused");
    assert!(ws.record_path("leaf").is_file());
    // Nothing depends on mid, so it is only built, as before.
    assert!(!ws.version_dir("mid").exists());

    assert_has(
        &ws.crux(&["build", "--profile", "debug"]),
        "0 compiled, 2 reused",
    );
    ws.set_leaf_value(60);
    assert_has(
        &ws.crux(&["build", "--profile", "debug"]),
        "2 compiled, 0 reused",
    );
}
