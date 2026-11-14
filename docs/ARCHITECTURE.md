# Crux Architecture Guide

This document describes the architectural design, core invariants, data structures, and component interactions of **Crux**.

---

## 1. System Architecture Overview

```text
+-------------------------------------------------------------+
|                          CLI Layer                          |
|  - main.rs, cli.rs (argument parsing, output formatting)     |
+-------------------------------------------------------------+
         |                                           |
         v                                           v
+-----------------------+                 +--------------------+
|  Manifest Subsystem   |                 | Lockfile Subsystem |
|  - manifest.rs        |                 | - lock.rs          |
|  - dependency.rs      |                 | - Checksum         |
|  - check.rs           |                 | - LockedPackage    |
|  - crux.toml          |                 | - crux.lock        |
+-----------------------+                 +--------------------+
         |                                           ^
         | (Dependency constraints)                  | (to_lockfile)
         v                                           |
+--------------------------------------------------------------+
|                     Resolution Engine                        |
|  - resolver.rs (backtracking solver, cycle detection)        |
|  - PackageCatalog trait & InMemoryCatalog (catalog.rs)       |
|  - ResolvedGraph & ResolvedPackage                           |
+--------------------------------------------------------------+
```

---

## 2. Core Subsystems

### A. Package Identity & Semantic Versioning (`src/package.rs`)
- **`PackageName`**: Strictly validated alphanumeric identifier with dashes and underscores (`^[a-zA-Z0-9][a-zA-Z0-9_-]*$`).
- **`PackageId`**: Pair of `(PackageName, semver::Version)`. Serializes canonically as `"<name> <version>"`.
- **`PackageMetadata`**: Model for package specifications (name, version, description, license, URLs).

### B. Manifest Specification (`src/manifest.rs`, `src/dependency.rs`)
- Parsed from `crux.toml`.
- Strongly-typed `[dependencies]` section mapped to `Vec<Dependency>`, each containing a `PackageName` and a `semver::VersionReq`.

### C. Catalog Abstraction (`src/catalog.rs`)
- Decouples resolution from registry or network storage.
- `PackageCatalog` trait returns available versions and dependency lists (`Vec<CatalogEntry>`).
- Enforces uniqueness per package identity.

### D. Dependency Resolution Engine (`src/resolver.rs`)
- Resolves manifest requirements into an acyclic `ResolvedGraph`.
- **Precedence Order**: Highest compatible stable version evaluated first; pre-releases excluded by default.
- **Single-Version Policy**: C/C++ ODR safety: at most one version per package name in the graph.
- **Cycle Detection**: Breadth-first reachability search detects directed cycles before edge addition.

### E. Lockfile Subsystem (`src/lock.rs`)
- Versioned schema (`lockfile_version = 1`).
- Records exact versions, acquisition sources, cryptographic checksums (SHA-256), and direct dependency links.
- Deterministic TOML serialization with canonical package ordering.

### F. Lockfile Synchronization & Atomic Persistence
- **Lockfile-Aware Resolution**:
  - `Resolver::resolve_with_preferences` accepts an in-memory map of `(PackageName -> semver::Version)`.
  - When re-resolving an existing project, locked versions are prioritized if they satisfy the manifest requirements.
  - `--update` flag clears preferences, resolving all dependencies to their highest compatible stable versions.
- **Synchronization Verification (`check_lockfile_sync`)**:
  - Validates `crux.lock` presence and TOML schema.
  - Validates that every manifest dependency and transitive dependency is locked.
  - Confirms locked versions satisfy current manifest constraints and detects obsolete packages or altered graphs.
- **Atomic Filesystem Safety (`write_lockfile_atomically`)**:
  - Writes to a uniquely named temporary file (`.{filename}.tmp.{pid}.{nanos}`) in the target project directory to ensure same-filesystem placement.
  - Flushes buffers and calls `sync_data()` before performing an atomic `fs::rename`.
  - Employs an RAII `TempFileGuard` to guarantee automatic cleanup of temporary files upon any intermediate failure or early exit.
  - Never mutates `crux.toml` or corrupts an existing `crux.lock` on resolution failure.

### G. Git Registry Subsystem (`src/registry.rs`)
- **Registry Layout**:
  - `registry.toml`: Defines `format_version = 1`, registry name, and optional description.
  - `packages/<name>/<version>/package.toml`: Human-readable package metadata, source archive specification (URL and SHA-256 checksum), build system specification, and dependencies.
- **`PackageCatalog` Implementation**:
  - `GitRegistry` implements the `PackageCatalog` trait directly, converting `PackageDefinition` into `CatalogEntry` with registry package sources (`registry+<name>`).
  - Completely decoupled from `Resolver`: the resolution engine interacts with `&dyn PackageCatalog` and cannot distinguish in-memory test fixtures from Git-backed or remote registries.
- **Git Execution Safety (`GitOps`)**:
  - Invokes `git` directly using `std::process::Command` with argument slices.
  - No shell is ever spawned; no shell interpolation, parameter injection, or environment escaping is possible.
  - Operations include repository validation (`rev-parse --is-inside-work-tree`), safe cloning (`clone <url> <dest>`), and fast-forward updating (`pull --ff-only`).
- **Security & Integrity Validations**:
  - **Identity Consistency**: Package definitions must declare `name` and `version` matching their directory path (`packages/<name>/<version>/`); mismatches are rejected.
  - **Symlink Jail**: Symbolic links inside `packages/` are inspected using `fs::canonicalize`; links resolving outside the registry root trigger `RegistryError::SecurityViolation`.
  - **Source-First Integrity**: Archive source definitions require valid SHA-256 hex checksums (`sha256:<64 hex>`).
  - **No Code Execution**: Package definitions are purely declarative TOML data.
- **Local Configuration (`CruxConfig`)**:
  - Stored in `.crux/config.toml` (project) or `~/.crux/config.toml` (user-global), with atomic write patterns.
  - Automatically queries the active registry when resolving or locking.
  - Fails with explicit errors if a configured registry fails to open; never silently falls back to test fixtures.
