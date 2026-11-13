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
