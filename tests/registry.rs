use crux::catalog::PackageCatalog;
use crux::cli::run;
use crux::dependency::Dependency;
use crux::package::{PackageId, PackageName};
use crux::registry::{
    CURRENT_REGISTRY_FORMAT_VERSION, GitRegistry, REGISTRY_MANIFEST_FILENAME, RegistryError,
    init_test_git_registry, load_config, seed_registry_files,
};
use crux::resolver::Resolver;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Creates an isolated temporary directory for test execution.
fn temp_test_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("crux_test_{name}_{}_{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("failed to create temp test dir");
    dir
}

#[test]
fn test_open_valid_git_registry() {
    let dir = temp_test_dir("open_valid");
    let reg = init_test_git_registry(&dir).expect("failed to initialize test registry");

    assert_eq!(reg.name(), "default");
    assert_eq!(
        reg.manifest().format_version,
        CURRENT_REGISTRY_FORMAT_VERSION
    );
    assert_eq!(reg.package_count(), 3);
    assert_eq!(reg.total_version_count(), 6);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_read_package_definitions() {
    let dir = temp_test_dir("read_defs");
    let reg = init_test_git_registry(&dir).expect("failed to initialize test registry");

    let fmt_id = PackageId::try_new("fmt", "11.1.1").unwrap();
    let def = reg
        .get_package_definition(&fmt_id)
        .expect("expected fmt 11.1.1 in registry");

    assert_eq!(def.metadata.name.as_str(), "fmt");
    assert_eq!(def.metadata.version.to_string(), "11.1.1");
    assert_eq!(
        def.metadata.description.as_deref(),
        Some("A modern formatting library")
    );
    assert_eq!(def.metadata.license.as_deref(), Some("MIT"));

    match &def.source {
        crux::registry::PackageSourceDefinition::Archive { url, checksum } => {
            assert!(url.contains("11.1.1"));
            assert_eq!(
                checksum.digest(),
                "a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"
            );
        }
        _ => panic!("expected archive source"),
    }

    assert_eq!(def.build.as_ref().map(|b| b.system.as_str()), Some("cmake"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_multiple_package_versions_and_lookup() {
    let dir = temp_test_dir("multi_versions");
    let reg = init_test_git_registry(&dir).expect("failed to initialize test registry");

    let zlib_name = PackageName::new("zlib").unwrap();
    let versions = reg.get_package_versions(&zlib_name).unwrap();

    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version.to_string(), "1.3.0");
    assert_eq!(versions[1].version.to_string(), "1.3.1");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_unknown_package_returns_empty_list() {
    let dir = temp_test_dir("unknown_pkg");
    let reg = init_test_git_registry(&dir).expect("failed to initialize test registry");

    let unknown = PackageName::new("nonexistent-lib").unwrap();
    let versions = reg.get_package_versions(&unknown).unwrap();
    assert!(versions.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_missing_registry_repository_fails() {
    let missing_path = temp_test_dir("missing_repo").join("does_not_exist");
    let res = GitRegistry::open(&missing_path);
    assert!(matches!(res, Err(RegistryError::Io(_))));
}

#[test]
fn test_invalid_git_repository_fails() {
    let dir = temp_test_dir("invalid_git");
    seed_registry_files(&dir).unwrap();

    // Directory has files, but is NOT a git repo
    let res = GitRegistry::open(&dir);
    assert!(matches!(res, Err(RegistryError::NotAGitRepository(_))));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_missing_registry_manifest_fails() {
    let dir = temp_test_dir("missing_manifest");
    // Init git repo without registry.toml
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&dir)
        .output()
        .unwrap();

    let res = GitRegistry::open(&dir);
    assert!(matches!(
        res,
        Err(RegistryError::MissingRegistryManifest(_))
    ));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_unsupported_format_version_fails() {
    let dir = temp_test_dir("unsupported_fmt");
    seed_registry_files(&dir).unwrap();
    fs::write(
        dir.join(REGISTRY_MANIFEST_FILENAME),
        "format_version = 99\nname = \"future\"\n",
    )
    .unwrap();

    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&dir)
        .output()
        .unwrap();

    let res = GitRegistry::open(&dir);
    assert!(matches!(
        res,
        Err(RegistryError::UnsupportedFormatVersion(99))
    ));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_identity_mismatch_detected() {
    let dir = temp_test_dir("identity_mismatch");
    seed_registry_files(&dir).unwrap();

    // Mutate package.toml to declare a different name
    let bad_pkg_toml = dir
        .join("packages")
        .join("fmt")
        .join("11.1.1")
        .join("package.toml");
    fs::write(
        bad_pkg_toml,
        r#"[package]
name = "wrong-name"
version = "11.1.1"

[source]
type = "archive"
url = "https://example.com/fmt.zip"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"
"#,
    )
    .unwrap();

    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&dir)
        .output()
        .unwrap();

    let res = GitRegistry::open(&dir);
    assert!(matches!(res, Err(RegistryError::IdentityMismatch { .. })));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_invalid_semantic_version_directory_fails() {
    let dir = temp_test_dir("invalid_ver_dir");
    seed_registry_files(&dir).unwrap();

    let bad_ver_dir = dir.join("packages").join("fmt").join("not-a-semver");
    fs::create_dir_all(&bad_ver_dir).unwrap();
    fs::write(
        bad_ver_dir.join("package.toml"),
        r#"[package]
name = "fmt"
version = "not-a-semver"

[source]
type = "archive"
url = "https://example.com/fmt.zip"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"
"#,
    )
    .unwrap();

    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&dir)
        .output()
        .unwrap();

    let res = GitRegistry::open(&dir);
    assert!(matches!(res, Err(RegistryError::InvalidVersion(_))));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_security_symlink_outside_registry_rejected() {
    let dir = temp_test_dir("symlink_outside");
    seed_registry_files(&dir).unwrap();

    let outside_target = temp_test_dir("outside_target");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let symlink_path = dir.join("packages").join("malicious");
        symlink(&outside_target, &symlink_path).unwrap();

        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&dir)
            .output()
            .unwrap();

        let res = GitRegistry::open(&dir);
        assert!(matches!(res, Err(RegistryError::SecurityViolation(_))));
    }

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside_target);
}

#[test]
fn test_deterministic_package_enumeration() {
    let dir = temp_test_dir("determinism");
    let reg = init_test_git_registry(&dir).expect("failed to initialize test registry");

    let names_1 = reg.package_names();
    let names_2 = reg.package_names();
    assert_eq!(names_1, names_2);
    assert_eq!(
        names_1.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
        vec!["catch2", "fmt", "zlib"]
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_resolver_integration_with_git_registry() {
    let dir = temp_test_dir("resolver_integration");
    let reg = init_test_git_registry(&dir).expect("failed to initialize test registry");

    let resolver = Resolver::new(&reg);

    let dep_fmt = Dependency::try_new("fmt", "^11.0").unwrap();
    let dep_zlib = Dependency::try_new("zlib", "^1.3").unwrap();
    let root_deps = vec![dep_fmt, dep_zlib];

    let graph = resolver
        .resolve(None, &root_deps)
        .expect("resolution with Git registry failed");

    assert_eq!(graph.package_count(), 2);
    let locked_fmt = graph
        .packages
        .get(&PackageName::new("fmt").unwrap())
        .unwrap();
    assert_eq!(locked_fmt.version.to_string(), "11.1.1");

    let locked_zlib = graph
        .packages
        .get(&PackageName::new("zlib").unwrap())
        .unwrap();
    assert_eq!(locked_zlib.version.to_string(), "1.3.1");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_cli_registry_add_list_update() {
    let _guard = ENV_LOCK.lock().unwrap();
    let base_dir = temp_test_dir("cli_registry_flow");
    let upstream_dir = base_dir.join("upstream_reg");
    let crux_home = base_dir.join("crux_home");
    let project_dir = base_dir.join("project");

    fs::create_dir_all(&upstream_dir).unwrap();
    fs::create_dir_all(&crux_home).unwrap();
    fs::create_dir_all(&project_dir).unwrap();

    init_test_git_registry(&upstream_dir).unwrap();

    // Set isolated environment variables
    unsafe {
        std::env::set_var("CRUX_HOME", &crux_home);
        std::env::set_var("CRUX_CONFIG_PATH", crux_home.join("config.toml"));
        std::env::set_var("CRUX_REGISTRIES_DIR", crux_home.join("registries"));
    }

    // 1. crux registry add default <upstream_dir>
    let mut out = Vec::new();
    let mut err = Vec::new();
    let add_res = run(
        [
            "crux",
            "registry",
            "add",
            "default",
            upstream_dir.to_str().unwrap(),
        ],
        &mut out,
        &mut err,
    );
    assert!(
        add_res.is_ok(),
        "crux registry add failed: {}",
        String::from_utf8_lossy(&err)
    );
    let add_out = String::from_utf8(out).unwrap();
    assert!(add_out.contains("Successfully added registry 'default'"));

    // Verify config saved
    let config = load_config(None).unwrap();
    assert!(config.registries.contains_key("default"));

    // 2. crux registry list
    let mut list_out = Vec::new();
    let mut list_err = Vec::new();
    let list_res = run(["crux", "registry", "list"], &mut list_out, &mut list_err);
    assert!(list_res.is_ok());
    let list_str = String::from_utf8(list_out).unwrap();
    assert!(list_str.contains("default"));
    assert!(list_str.contains("Packages: 3"));

    // 3. Add a new package version to upstream repo
    let new_pkg_dir = upstream_dir.join("packages").join("zlib").join("1.3.2");
    fs::create_dir_all(&new_pkg_dir).unwrap();
    fs::write(
        new_pkg_dir.join("package.toml"),
        r#"[package]
name = "zlib"
version = "1.3.2"
description = "A Massively Spiffy Yet Delicately Unobtrusive Compression Library"
license = "Zlib"
homepage = "https://zlib.net"
repository = "https://github.com/madler/zlib"

[source]
type = "archive"
url = "https://github.com/madler/zlib/releases/download/v1.3.2/zlib-1.3.2.tar.gz"
checksum = "sha256:9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"

[build]
system = "cmake"

[dependencies]
"#,
    )
    .unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(&upstream_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Add zlib 1.3.2"])
        .current_dir(&upstream_dir)
        .output()
        .unwrap();

    // 4. crux registry update default
    let mut update_out = Vec::new();
    let mut update_err = Vec::new();
    let update_res = run(
        ["crux", "registry", "update", "default"],
        &mut update_out,
        &mut update_err,
    );
    assert!(
        update_res.is_ok(),
        "update failed: {}",
        String::from_utf8_lossy(&update_err)
    );
    let update_str = String::from_utf8(update_out).unwrap();
    assert!(update_str.contains("Successfully updated registry 'default'"));
    assert!(update_str.contains("Versions: 7"));

    // Cleanup env
    unsafe {
        std::env::remove_var("CRUX_HOME");
        std::env::remove_var("CRUX_CONFIG_PATH");
        std::env::remove_var("CRUX_REGISTRIES_DIR");
    }

    let _ = fs::remove_dir_all(&base_dir);
}

#[test]
fn test_cli_lock_and_resolve_using_configured_git_registry() {
    let _guard = ENV_LOCK.lock().unwrap();
    let base_dir = temp_test_dir("cli_lock_registry");
    let reg_dir = base_dir.join("test_reg");
    let crux_home = base_dir.join("crux_home");
    let project_dir = base_dir.join("project");

    fs::create_dir_all(&reg_dir).unwrap();
    fs::create_dir_all(&crux_home).unwrap();
    fs::create_dir_all(&project_dir).unwrap();

    init_test_git_registry(&reg_dir).unwrap();

    // Set isolated environment variables
    unsafe {
        std::env::set_var("CRUX_HOME", &crux_home);
        std::env::set_var("CRUX_CONFIG_PATH", crux_home.join("config.toml"));
        std::env::set_var("CRUX_REGISTRIES_DIR", crux_home.join("registries"));
    }

    // Register test registry as 'default'
    let mut out = Vec::new();
    let mut err = Vec::new();
    let add_res = run(
        [
            "crux",
            "registry",
            "add",
            "default",
            reg_dir.to_str().unwrap(),
        ],
        &mut out,
        &mut err,
    );
    assert!(
        add_res.is_ok(),
        "registry add failed: {}",
        String::from_utf8_lossy(&err)
    );

    // Create crux.toml in project directory
    let crux_toml = r#"[package]
name = "my-test-app"
version = "0.1.0"

[dependencies]
fmt = "^11.0"
zlib = "^1.3"
"#;
    fs::write(project_dir.join("crux.toml"), crux_toml).unwrap();

    // Switch current dir to project_dir for CLI commands
    let orig_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project_dir).unwrap();

    // 1. crux resolve
    let mut resolve_out = Vec::new();
    let mut resolve_err = Vec::new();
    let res = run(["crux", "resolve"], &mut resolve_out, &mut resolve_err);
    assert!(
        res.is_ok(),
        "crux resolve failed: {}",
        String::from_utf8_lossy(&resolve_err)
    );
    let resolve_str = String::from_utf8(resolve_out).unwrap();
    assert!(resolve_str.contains("Using registry 'default'"));
    assert!(resolve_str.contains("fmt 11.1.1 (registry+default)"));
    assert!(resolve_str.contains("zlib 1.3.1 (registry+default)"));

    // 2. crux lock
    let mut lock_out = Vec::new();
    let mut lock_err = Vec::new();
    let lock_res = run(["crux", "lock"], &mut lock_out, &mut lock_err);
    assert!(
        lock_res.is_ok(),
        "crux lock failed: {}",
        String::from_utf8_lossy(&lock_err)
    );
    let lock_str = String::from_utf8(lock_out).unwrap();
    assert!(lock_str.contains("Using registry 'default'"));
    assert!(lock_str.contains("Generated crux.lock"));

    let lockfile_content = fs::read_to_string(project_dir.join("crux.lock")).unwrap();
    assert!(lockfile_content.contains("name = \"fmt\""));
    assert!(lockfile_content.contains("version = \"11.1.1\""));
    assert!(lockfile_content.contains(
        "checksum = \"sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45\""
    ));
    assert!(lockfile_content.contains("name = \"zlib\""));
    assert!(lockfile_content.contains("version = \"1.3.1\""));
    assert!(lockfile_content.contains(
        "checksum = \"sha256:9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23\""
    ));

    // 3. crux lock --check
    let mut check_out = Vec::new();
    let mut check_err = Vec::new();
    let check_res = run(["crux", "lock", "--check"], &mut check_out, &mut check_err);
    assert!(
        check_res.is_ok(),
        "crux lock --check failed: {}",
        String::from_utf8_lossy(&check_err)
    );
    let check_str = String::from_utf8(check_out).unwrap();
    assert!(check_str.contains("Status: Valid (synchronized)"));

    // Restore dir and env
    std::env::set_current_dir(&orig_dir).unwrap();
    unsafe {
        std::env::remove_var("CRUX_HOME");
        std::env::remove_var("CRUX_CONFIG_PATH");
        std::env::remove_var("CRUX_REGISTRIES_DIR");
    }

    let _ = fs::remove_dir_all(&base_dir);
}

#[test]
fn test_cli_broken_git_registry_fails_without_fallback() {
    let _guard = ENV_LOCK.lock().unwrap();
    let base_dir = temp_test_dir("cli_broken_reg");
    let crux_home = base_dir.join("crux_home");
    let project_dir = base_dir.join("project");

    fs::create_dir_all(&crux_home).unwrap();
    fs::create_dir_all(&project_dir).unwrap();

    // Create broken config pointing to nonexistent git repo
    let broken_config = r#"[registries.default]
path = "/path/to/definitely/nonexistent/directory"
"#;
    fs::write(crux_home.join("config.toml"), broken_config).unwrap();

    unsafe {
        std::env::set_var("CRUX_HOME", &crux_home);
        std::env::set_var("CRUX_CONFIG_PATH", crux_home.join("config.toml"));
    }

    let crux_toml = r#"[package]
name = "my-broken-test"
version = "0.1.0"

[dependencies]
fmt = "^11.0"
"#;
    fs::write(project_dir.join("crux.toml"), crux_toml).unwrap();

    let orig_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project_dir).unwrap();

    let mut out = Vec::new();
    let mut err = Vec::new();
    let res = run(["crux", "resolve"], &mut out, &mut err);

    // MUST fail with error and NEVER silently fall back to in-memory catalog
    assert!(res.is_err());
    let err_msg = String::from_utf8(err).unwrap();
    assert!(!err_msg.contains("Using in-memory test catalog"));

    std::env::set_current_dir(&orig_dir).unwrap();
    unsafe {
        std::env::remove_var("CRUX_HOME");
        std::env::remove_var("CRUX_CONFIG_PATH");
    }

    let _ = fs::remove_dir_all(&base_dir);
}

#[test]
fn test_cli_registry_add_duplicate_fails() {
    let _guard = ENV_LOCK.lock().unwrap();
    let base_dir = temp_test_dir("cli_duplicate");
    let upstream_dir = base_dir.join("upstream_reg");
    let crux_home = base_dir.join("crux_home");

    fs::create_dir_all(&upstream_dir).unwrap();
    fs::create_dir_all(&crux_home).unwrap();
    init_test_git_registry(&upstream_dir).unwrap();

    unsafe {
        std::env::set_var("CRUX_HOME", &crux_home);
        std::env::set_var("CRUX_CONFIG_PATH", crux_home.join("config.toml"));
        std::env::set_var("CRUX_REGISTRIES_DIR", crux_home.join("registries"));
    }

    let mut out1 = Vec::new();
    let mut err1 = Vec::new();
    let res1 = run(
        [
            "crux",
            "registry",
            "add",
            "myreg",
            upstream_dir.to_str().unwrap(),
        ],
        &mut out1,
        &mut err1,
    );
    assert!(res1.is_ok());

    let mut out2 = Vec::new();
    let mut err2 = Vec::new();
    let res2 = run(
        [
            "crux",
            "registry",
            "add",
            "myreg",
            upstream_dir.to_str().unwrap(),
        ],
        &mut out2,
        &mut err2,
    );
    assert!(res2.is_err());
    let err_msg = String::from_utf8(err2).unwrap();
    assert!(err_msg.contains("is already configured"));

    unsafe {
        std::env::remove_var("CRUX_HOME");
        std::env::remove_var("CRUX_CONFIG_PATH");
        std::env::remove_var("CRUX_REGISTRIES_DIR");
    }

    let _ = fs::remove_dir_all(&base_dir);
}

/// Hardening (2026-11-21): registry URLs may embed credentials, which must
/// never be printed by `crux registry list`.
#[test]
fn test_registry_list_redacts_url_credentials() {
    let root = std::env::temp_dir().join(format!(
        "crux_registry_redact_{}_{}",
        std::process::id(),
        crux::core::generate_nonce()
    ));
    let registry = root.join("registry");
    std::fs::create_dir_all(&registry).unwrap();
    std::fs::write(
        registry.join("registry.toml"),
        "format_version = 1\nname = \"private\"\ndescription = \"x\"\n",
    )
    .unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&registry)
            .status()
            .unwrap()
            .success()
    );
    let config = root.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[registries.private]\nurl = \"https://deploy:s3cr3t-token@git.example.invalid/reg.git\"\npath = \"{}\"\n",
            registry.display()
        ),
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_crux"))
        .args(["registry", "list"])
        .current_dir(&root)
        .env("CRUX_CONFIG_PATH", &config)
        .env("CRUX_HOME", &root)
        .env_remove("CRUX_REGISTRY_PATH")
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{all}");
    assert!(
        all.contains("https://***@git.example.invalid/reg.git"),
        "{all}"
    );
    assert!(!all.contains("s3cr3t-token"), "credentials leaked: {all}");
    let _ = std::fs::remove_dir_all(&root);
}
