//! Shared integration-test helpers.
//!
//! These helpers create real, isolated Git registries containing the historical
//! fixture package versions. They keep CLI tests deterministic and offline
//! without allowing production code to fall back to an in-memory catalog.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type FixtureDependency<'a> = (&'a str, &'a str);
type FixturePackage<'a> = (&'a str, &'a str, &'a [FixtureDependency<'a>]);

const CHECKSUMLESS_GIT_SOURCE: &str = "https://example.invalid/crux-test-fixtures.git";

/// Creates a Git-backed registry containing the legacy development fixture
/// package versions used by lock/resolve integration tests.
pub fn init_fixture_registry(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).expect("failed to create fixture registry directory");
    fs::write(
        dir.join("registry.toml"),
        "format_version = 1\nname = \"default\"\ndescription = \"Isolated CLI test registry\"\n",
    )
    .expect("failed to write fixture registry manifest");

    let packages: &[FixturePackage<'_>] = &[
        ("fmt", "10.0.0", &[]),
        ("fmt", "11.0.0", &[]),
        ("fmt", "11.1.0", &[]),
        ("fmt", "11.1.1", &[]),
        ("zlib", "1.2.11", &[]),
        ("zlib", "1.2.13", &[]),
        ("zlib", "1.3.0", &[]),
        ("zlib", "1.3.1", &[]),
        ("openssl", "3.0.0", &[("zlib", "^1.2")]),
        ("openssl", "3.3.0", &[("zlib", "^1.2")]),
        ("openssl", "3.4.0", &[("zlib", "^1.3")]),
        ("spdlog", "1.12.0", &[("fmt", "^10.0")]),
        ("spdlog", "1.13.0", &[("fmt", "^11.1")]),
        ("catch2", "3.4.0", &[]),
        ("catch2", "3.5.0", &[]),
    ];

    for (name, version, dependencies) in packages {
        let package_dir = dir.join("packages").join(name).join(version);
        fs::create_dir_all(&package_dir).expect("failed to create fixture package directory");

        let mut content = format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\n\
             description = \"Isolated CLI test fixture\"\nlicense = \"MIT\"\n\n\
             [source]\ntype = \"git\"\nurl = \"{CHECKSUMLESS_GIT_SOURCE}\"\n\n\
             [build]\nsystem = \"cmake\"\n\n[dependencies]\n"
        );
        for (dep_name, requirement) in *dependencies {
            content.push_str(&format!("{dep_name} = \"{requirement}\"\n"));
        }
        fs::write(package_dir.join("package.toml"), content)
            .expect("failed to write fixture package definition");
    }

    let git_init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir)
        .output()
        .expect("failed to initialize fixture registry Git repository");
    assert!(
        git_init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );

    dir.to_path_buf()
}

/// Configures a subprocess command to use the supplied isolated registry.
pub fn configure_registry(command: &mut Command, registry_dir: &Path, config_path: &Path) {
    command
        .env("CRUX_REGISTRY_PATH", registry_dir)
        .env("CRUX_CONFIG_PATH", config_path)
        .env_remove("CRUX_OFFLINE")
        .env_remove("CRUX_HOME")
        .env_remove("CRUX_REGISTRIES_DIR");
}
