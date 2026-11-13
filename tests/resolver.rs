use crux::catalog::{CatalogError, InMemoryCatalog, default_dev_catalog};
use crux::dependency::Dependency;
use crux::lock::Lockfile;
use crux::package::{PackageId, PackageName};
use crux::resolver::{ResolutionError, Resolver};
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
        let path = std::env::temp_dir().join(format!("crux_integ_resolve_{prefix}_{nanos}"));
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
fn test_resolver_exact_version_selection() {
    let mut catalog = InMemoryCatalog::new();
    catalog.add_package("fmt", "11.0.0", &[]).unwrap();
    catalog.add_package("fmt", "11.1.0", &[]).unwrap();
    catalog.add_package("fmt", "11.1.1", &[]).unwrap();
    catalog.add_package("fmt", "12.0.0", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("fmt", "=11.1.0").unwrap()];

    let graph = resolver.resolve(None, &deps).unwrap();
    assert_eq!(graph.package_count(), 1);
    let pkg = graph.get("fmt").unwrap();
    assert_eq!(pkg.version.to_string(), "11.1.0");
}

#[test]
fn test_resolver_caret_requirement_highest_compatible() {
    let mut catalog = InMemoryCatalog::new();
    catalog.add_package("fmt", "11.0.0", &[]).unwrap();
    catalog.add_package("fmt", "11.1.0", &[]).unwrap();
    catalog.add_package("fmt", "11.1.2", &[]).unwrap();
    catalog.add_package("fmt", "12.0.0", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("fmt", "^11.1.0").unwrap()];

    let graph = resolver.resolve(None, &deps).unwrap();
    assert_eq!(graph.package_count(), 1);
    let pkg = graph.get("fmt").unwrap();
    assert_eq!(pkg.version.to_string(), "11.1.2");
}

#[test]
fn test_resolver_tilde_requirement() {
    let mut catalog = InMemoryCatalog::new();
    catalog.add_package("zlib", "1.2.3", &[]).unwrap();
    catalog.add_package("zlib", "1.2.9", &[]).unwrap();
    catalog.add_package("zlib", "1.3.0", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("zlib", "~1.2.3").unwrap()];

    let graph = resolver.resolve(None, &deps).unwrap();
    let pkg = graph.get("zlib").unwrap();
    assert_eq!(pkg.version.to_string(), "1.2.9");
}

#[test]
fn test_resolver_comparator_requirements() {
    let mut catalog = InMemoryCatalog::new();
    catalog.add_package("openssl", "1.1.1", &[]).unwrap();
    catalog.add_package("openssl", "3.0.0", &[]).unwrap();
    catalog.add_package("openssl", "3.2.0", &[]).unwrap();
    catalog.add_package("openssl", "3.4.0", &[]).unwrap();
    catalog.add_package("openssl", "4.0.0", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("openssl", ">=3.0.0, <3.3.0").unwrap()];

    let graph = resolver.resolve(None, &deps).unwrap();
    let pkg = graph.get("openssl").unwrap();
    assert_eq!(pkg.version.to_string(), "3.2.0");
}

#[test]
fn test_resolver_highest_compatible_version_selection() {
    let mut catalog = InMemoryCatalog::new();
    catalog.add_package("catch2", "3.0.0", &[]).unwrap();
    catalog.add_package("catch2", "3.4.0", &[]).unwrap();
    catalog.add_package("catch2", "3.7.0", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("catch2", "^3.0").unwrap()];

    let graph = resolver.resolve(None, &deps).unwrap();
    assert_eq!(graph.get("catch2").unwrap().version.to_string(), "3.7.0");
}

#[test]
fn test_resolver_prerelease_exclusion_and_explicit_inclusion() {
    let mut catalog = InMemoryCatalog::new();
    catalog.add_package("boost", "1.82.0", &[]).unwrap();
    catalog.add_package("boost", "1.83.0", &[]).unwrap();
    catalog.add_package("boost", "1.84.0-alpha.1", &[]).unwrap();

    let resolver = Resolver::new(&catalog);

    // Default caret requirement should NOT select the pre-release
    let deps_stable = [Dependency::try_new("boost", "^1.82").unwrap()];
    let graph_stable = resolver.resolve(None, &deps_stable).unwrap();
    assert_eq!(
        graph_stable.get("boost").unwrap().version.to_string(),
        "1.83.0"
    );

    // Explicit requirement for pre-release allows selecting it
    let deps_prerelease = [Dependency::try_new("boost", "=1.84.0-alpha.1").unwrap()];
    let graph_prerelease = resolver.resolve(None, &deps_prerelease).unwrap();
    assert_eq!(
        graph_prerelease.get("boost").unwrap().version.to_string(),
        "1.84.0-alpha.1"
    );
}

#[test]
fn test_resolver_no_matching_version_error() {
    let mut catalog = InMemoryCatalog::new();
    catalog.add_package("fmt", "10.0.0", &[]).unwrap();
    catalog.add_package("fmt", "10.1.0", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("fmt", "^11.0").unwrap()];

    let err = resolver.resolve(None, &deps).unwrap_err();
    match err {
        ResolutionError::NoMatchingVersion(details) => {
            assert_eq!(details.package.as_str(), "fmt");
            assert_eq!(details.requirement.to_string(), "^11.0");
            assert_eq!(details.available.len(), 2);
            assert_eq!(details.available[0].to_string(), "10.0.0");
            assert_eq!(details.available[1].to_string(), "10.1.0");
            assert!(details.dependent.is_none());
        }
        other => panic!("expected NoMatchingVersion error, got: {other:?}"),
    }
}

#[test]
fn test_resolver_package_not_found_error() {
    let catalog = InMemoryCatalog::new();
    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("nonexistent", "^1.0").unwrap()];

    let err = resolver.resolve(None, &deps).unwrap_err();
    match err {
        ResolutionError::PackageNotFound { package, dependent } => {
            assert_eq!(package.as_str(), "nonexistent");
            assert!(dependent.is_none());
        }
        other => panic!("expected PackageNotFound, got: {other:?}"),
    }
}

#[test]
fn test_resolver_conflicting_constraints_error() {
    let mut catalog = InMemoryCatalog::new();
    catalog
        .add_package("app", "1.0.0", &[("zlib", "^1.3")])
        .unwrap();
    catalog
        .add_package("lib", "1.0.0", &[("zlib", "=1.2.11")])
        .unwrap();
    catalog.add_package("zlib", "1.2.11", &[]).unwrap();
    catalog.add_package("zlib", "1.3.1", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [
        Dependency::try_new("app", "^1.0").unwrap(),
        Dependency::try_new("lib", "^1.0").unwrap(),
    ];

    let err = resolver.resolve(None, &deps).unwrap_err();
    match err {
        ResolutionError::ConflictingRequirements { package, conflicts } => {
            assert_eq!(package.as_str(), "zlib");
            assert_eq!(conflicts.len(), 2);
        }
        other => panic!("expected ConflictingRequirements, got: {other:?}"),
    }
}

#[test]
fn test_resolver_transitive_linear_chain() {
    let mut catalog = InMemoryCatalog::new();
    catalog
        .add_package("app", "1.0.0", &[("layer1", "^1.0")])
        .unwrap();
    catalog
        .add_package("layer1", "1.2.0", &[("layer2", "^2.0")])
        .unwrap();
    catalog
        .add_package("layer2", "2.1.0", &[("layer3", "^3.0")])
        .unwrap();
    catalog.add_package("layer3", "3.0.0", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("app", "^1.0").unwrap()];

    let graph = resolver.resolve(None, &deps).unwrap();
    assert_eq!(graph.package_count(), 4);
    assert!(graph.get("app").is_some());
    assert!(graph.get("layer1").is_some());
    assert!(graph.get("layer2").is_some());
    assert!(graph.get("layer3").is_some());

    assert_eq!(
        graph.get("app").unwrap().dependencies,
        vec![PackageId::try_new("layer1", "1.2.0").unwrap()]
    );
}

#[test]
fn test_resolver_diamond_dependency_satisfaction() {
    let mut catalog = InMemoryCatalog::new();
    // lib_a requires zlib ^1.2 (permits >=1.2.0, <2.0.0)
    catalog
        .add_package("lib_a", "1.0.0", &[("zlib", "^1.2")])
        .unwrap();
    // lib_b requires zlib >=1.2.8, <1.3.0
    catalog
        .add_package("lib_b", "1.0.0", &[("zlib", ">=1.2.8, <1.3.0")])
        .unwrap();
    catalog.add_package("zlib", "1.2.8", &[]).unwrap();
    catalog.add_package("zlib", "1.2.11", &[]).unwrap();
    catalog.add_package("zlib", "1.3.1", &[]).unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [
        Dependency::try_new("lib_a", "^1.0").unwrap(),
        Dependency::try_new("lib_b", "^1.0").unwrap(),
    ];

    let graph = resolver.resolve(None, &deps).unwrap();
    assert_eq!(graph.package_count(), 3);
    // zlib 1.2.11 satisfies both ^1.2 and >=1.2.8, <1.3.0 (and is highest satisfying version)
    assert_eq!(graph.get("zlib").unwrap().version.to_string(), "1.2.11");
}

#[test]
fn test_resolver_missing_transitive_dependency_error() {
    let mut catalog = InMemoryCatalog::new();
    catalog
        .add_package("direct", "1.0.0", &[("missing_leaf", "^2.0")])
        .unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("direct", "^1.0").unwrap()];

    let err = resolver.resolve(None, &deps).unwrap_err();
    match err {
        ResolutionError::PackageNotFound { package, dependent } => {
            assert_eq!(package.as_str(), "missing_leaf");
            assert_eq!(
                dependent.unwrap(),
                PackageId::try_new("direct", "1.0.0").unwrap()
            );
        }
        other => panic!("expected PackageNotFound, got: {other:?}"),
    }
}

#[test]
fn test_resolver_cyclic_dependency_error() {
    let mut catalog = InMemoryCatalog::new();
    catalog
        .add_package("node_a", "1.0.0", &[("node_b", "^1.0")])
        .unwrap();
    catalog
        .add_package("node_b", "1.0.0", &[("node_c", "^1.0")])
        .unwrap();
    catalog
        .add_package("node_c", "1.0.0", &[("node_a", "^1.0")])
        .unwrap();

    let resolver = Resolver::new(&catalog);
    let deps = [Dependency::try_new("node_a", "^1.0").unwrap()];

    let err = resolver.resolve(None, &deps).unwrap_err();
    match err {
        ResolutionError::CyclicDependency { cycle } => {
            assert_eq!(cycle.first(), cycle.last());
            assert!(cycle.len() >= 3);
            assert!(cycle.contains(&PackageName::new("node_a").unwrap()));
            assert!(cycle.contains(&PackageName::new("node_b").unwrap()));
            assert!(cycle.contains(&PackageName::new("node_c").unwrap()));
        }
        other => panic!("expected CyclicDependency, got: {other:?}"),
    }
}

#[test]
fn test_catalog_rejects_duplicate_entries() {
    let mut catalog = InMemoryCatalog::new();
    catalog.add_package("fmt", "11.1.1", &[]).unwrap();

    let duplicate_res = catalog.add_package("fmt", "11.1.1", &[]);
    match duplicate_res {
        Err(CatalogError::DuplicateEntry(id)) => {
            assert_eq!(id, PackageId::try_new("fmt", "11.1.1").unwrap());
        }
        other => panic!("expected DuplicateEntry error, got: {other:?}"),
    }
}

#[test]
fn test_resolver_determinism_across_insertion_orders() {
    // Catalog 1: inserted in order A, B, C
    let mut cat1 = InMemoryCatalog::new();
    cat1.add_package("aaa", "1.0.0", &[("ccc", "^1.0")])
        .unwrap();
    cat1.add_package("bbb", "2.0.0", &[("ccc", "^1.0")])
        .unwrap();
    cat1.add_package("ccc", "1.5.0", &[]).unwrap();
    cat1.add_package("ccc", "1.8.0", &[]).unwrap();

    // Catalog 2: inserted in reversed order
    let mut cat2 = InMemoryCatalog::new();
    cat2.add_package("ccc", "1.8.0", &[]).unwrap();
    cat2.add_package("ccc", "1.5.0", &[]).unwrap();
    cat2.add_package("bbb", "2.0.0", &[("ccc", "^1.0")])
        .unwrap();
    cat2.add_package("aaa", "1.0.0", &[("ccc", "^1.0")])
        .unwrap();

    let deps = [
        Dependency::try_new("bbb", "^2.0").unwrap(),
        Dependency::try_new("aaa", "^1.0").unwrap(),
    ];

    let graph1 = Resolver::new(&cat1).resolve(None, &deps).unwrap();
    let graph2 = Resolver::new(&cat2).resolve(None, &deps).unwrap();

    assert_eq!(graph1, graph2);

    let lock1 = graph1.to_lockfile().unwrap().to_toml_string().unwrap();
    let lock2 = graph2.to_lockfile().unwrap().to_toml_string().unwrap();
    assert_eq!(lock1, lock2);
}

#[test]
fn test_resolved_graph_to_lockfile_roundtrip() {
    let catalog = default_dev_catalog();
    let resolver = Resolver::new(&catalog);

    let root_pkg = PackageId::try_new("my-project", "0.1.0").unwrap();
    let deps = [
        Dependency::try_new("openssl", "^3.4").unwrap(),
        Dependency::try_new("fmt", "^11.1").unwrap(),
    ];

    let graph = resolver.resolve(Some(root_pkg), &deps).unwrap();
    let lockfile = graph.to_lockfile().unwrap();

    assert_eq!(lockfile.package_count(), 3); // fmt, openssl, zlib
    let toml_str = lockfile.to_toml_string().unwrap();

    let deserialized = Lockfile::from_toml_str(&toml_str).unwrap();
    assert_eq!(lockfile, deserialized);
    assert_eq!(deserialized.package_count(), 3);

    // Verify deterministic package ordering in lockfile
    let packages = &deserialized.packages;
    assert_eq!(packages[0].name.as_str(), "fmt");
    assert_eq!(packages[1].name.as_str(), "openssl");
    assert_eq!(packages[2].name.as_str(), "zlib");
}

#[test]
fn test_cli_resolve_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .arg("--help")
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("crux-resolve"));
    assert!(stdout.contains("in-memory test catalog"));
}

#[test]
fn test_cli_resolve_unexpected_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .arg("--invalid-flag")
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("error: unexpected argument '--invalid-flag'"));
}

#[test]
fn test_cli_resolve_missing_manifest() {
    let test_dir = TestDir::new("cli_resolve_missing");

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("manifest not found at"));
}

#[test]
fn test_cli_resolve_project_without_dependencies() {
    let test_dir = TestDir::new("cli_resolve_no_deps");
    let manifest_content = r#"
[package]
name = "isolated-app"
version = "0.1.0"
description = "App without dependencies"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("isolated-app"));
    assert!(stdout.contains("No dependencies declared in crux.toml"));
    assert!(stdout.contains("Status: Success (0 packages resolved)"));
}

#[test]
fn test_cli_resolve_success_with_dev_fixture_catalog() {
    let test_dir = TestDir::new("cli_resolve_success");
    let manifest_content = r#"
[package]
name = "cpp-network-app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
openssl = "^3.4"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("cpp-network-app"));
    assert!(stdout.contains("Resolved 3 dependencies:"));
    assert!(stdout.contains("fmt 11.1.1 (registry+default)"));
    assert!(stdout.contains("openssl 3.4.0 (registry+default)"));
    assert!(stdout.contains("zlib 1.3.1 (registry+default)"));
    assert!(stdout.contains("Status: Success"));
}

#[test]
fn test_cli_resolve_detects_conflict_with_dev_fixture() {
    let test_dir = TestDir::new("cli_resolve_conflict");
    // In default_dev_catalog():
    // spdlog 1.12.0 requires fmt ^10.0
    // If root requests fmt ^11.1 and spdlog 1.12.0, they conflict on fmt!
    let manifest_content = r#"
[package]
name = "conflict-app"
version = "1.0.0"

[dependencies]
fmt = "^11.1"
spdlog = "=1.12.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Status: Resolution Failed"));
    assert!(stderr.contains("conflicting version requirements for package 'fmt'"));
}

#[test]
fn test_cli_resolve_unknown_dependency() {
    let test_dir = TestDir::new("cli_resolve_unknown");
    let manifest_content = r#"
[package]
name = "missing-dep-app"
version = "1.0.0"

[dependencies]
nonexistent-lib = "^1.0"
"#;
    fs::write(test_dir.path.join("crux.toml"), manifest_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crux"))
        .arg("resolve")
        .current_dir(&test_dir.path)
        .output()
        .expect("failed to execute crux binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    eprintln!("STDERR:\n{stderr}");
    assert!(stderr.contains("Status: Resolution Failed"));
    assert!(stderr.contains("package 'nonexistent-lib' not found in catalog"));
}
