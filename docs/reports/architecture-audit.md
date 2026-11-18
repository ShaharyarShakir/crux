# Crux Architecture Audit (Milestone 14)

## 1. Executive Summary

Crux has evolved through Milestones 1 to 13 from a project scaffold to a full-featured, source-first C/C++ package manager. It now incorporates manifest parsing, semantic versioning, deterministic backtracking resolution with single-version ODR safety, canonical lockfile serialization, Git package registries, cryptographic source acquisition and extraction, isolated CMake compilation, staged package installation, and CMake consumer integration.

While all functional requirements and test suites are passing (294 tests across 14 test binaries), the implementation has resided primarily in a flat `src/` directory with 16 top-level files. In particular, `src/cli.rs` grew to 2,646 lines, combining argument parsing, CLI help formatting, error translation, and complex business orchestration for nine distinct subcommands.

This audit establishes the baseline dependency structure, pinpoints architectural hotspots, and defines the modular target architecture for Milestone 14 refactoring.

---

## 2. Current Architecture & Codebase Map

### File Inventory & Metrics
| File | Lines | Primary Responsibility |
|---|---|---|
| `src/cli.rs` | 2,646 | CLI argument parsing, help texts, command execution, and workflow orchestration. |
| `src/source.rs` | 1,331 | HTTP streaming download, checksum verification, archive extraction, bomb limits, and source cache. |
| `src/lock.rs` | 1,321 | Canonical lockfile format, deterministic TOML serialization, checksums, package sources, sync validation. |
| `src/registry.rs` | 1,028 | Git-backed package catalog (`GitRegistry`), configuration, package definitions, registry manifest. |
| `src/build.rs` | 980 | Out-of-source CMake build engine, build fingerprints, toolchain detection, build profiles, metadata. |
| `src/resolver.rs` | 848 | Deterministic backtracking resolver, single-version ODR collision enforcement, BFS cycle detection. |
| `src/install.rs` | 790 | CMake install execution (`cmake --install`), staging guard, atomic directory swaps, file inventory (`installed.toml`). |
| `src/package.rs` | 707 | `PackageId`, `PackageName`, `PackageMetadata`, SemVer precedence. |
| `src/integrate.rs` | 686 | CMake consumer integration (`CruxDependencies.cmake`), path escaping, metadata validation, lockfile traversal. |
| `src/manifest.rs` | 589 | `crux.toml` schema parsing, validation, and serialization. |
| `src/catalog.rs` | 241 | `PackageCatalog` trait, `InMemoryCatalog` fixture, `default_dev_catalog`. |
| `src/init.rs` | 218 | `crux init` directory scaffolding and manifest initialization. |
| `src/dependency.rs` | 191 | `Dependency` requirement specification (`^`, `~`, comparator ranges). |
| `src/check.rs` | 153 | `crux check` manifest validation logic. |
| `src/lib.rs` | 54 | Public module declarations and re-exports. |
| `src/main.rs` | 12 | CLI binary entry point. |
| **Total** | **11,795** | **16 files at single root hierarchy** |

---

## 3. Current Module Dependency Graph

```text
                               +----------------+
                               |    main.rs     |
                               +-------+--------+
                                       |
                                       v
                               +----------------+
                               |     cli.rs     |
                               +---+----+---+---+
     +-----------------------------+    |   +--------------------------+
     |             +--------------------+----------------+             |
     |             |                    |                |             |
     v             v                    v                v             v
+----------+ +-----------+       +------------+   +------------+ +-----------+
| init.rs  | | check.rs  |       | resolve.rs |   | source.rs  | | build.rs  |
+----+-----+ +-----+-----+       +-----+------+   +-----+------+ +-----+-----+
     |             |                   |                |              |
     +------+------+                   |                v              |
            |                          |          +-----------+        |
            v                          |          | lock.rs   |<-------+
     +-------------+                   |          +-----+-----+        |
     | manifest.rs |                   |                |              |
     +------+------+                   v                v              |
            |                    +------------+   +-----------+        |
            |                    | catalog.rs |<--|package.rs |        |
            v                    +-----+------+   +-----+-----+        |
     +---------------+                 ^                ^              |
     | dependency.rs |                 |                |              |
     +-------+-------+                 |                |              v
             |                         v                |        +-----------+
             +------------------>+-------------+        +--------|install.rs |
                                 | registry.rs |                 +-----+-----+
                                 +-------------+                       |
                                        ^                              v
                                        |                        +-------------+
                                        +------------------------|integrate.rs |
                                                                 +-------------+
```

---

## 4. Architectural Hotspots & Problems Identified

1. **Monolithic CLI Orchestration (`src/cli.rs`)**:
   - `cli.rs` currently spans 2,646 lines.
   - It conflates pure CLI tasks (argument parsing, flag validation, help text generation) with substantive multi-step workflow logic (`run_fetch`, `run_build`, `run_install`, `run_integrate_cmake`).
   - Adding or modifying any subcommand requires touching a single large file.

2. **Scattered Project Subsystem**:
   - `src/init.rs`, `src/check.rs`, and `src/manifest.rs` all deal with the local Crux project lifecycle (`crux.toml`), but sit as disconnected top-level files.

3. **Domain Model Fragmentation**:
   - `src/package.rs` and `src/dependency.rs` are separated at the top level despite representing the unified package domain model (`PackageName`, `PackageId`, `Dependency`, `PackageMetadata`).

4. **Resolution Subsystem Fragmentation**:
   - `src/catalog.rs`, `src/resolver.rs`, and `src/lock.rs` together define the resolution engine, catalog abstraction, and canonical lockfile generation, but lack a cohesive module namespace.

5. **Duplicated Utility Logic**:
   - Lexical path normalization (`normalize_path`) was duplicated across `src/install.rs` and `src/integrate.rs`.
   - Atomic temporary file creation and replacement patterns are repeated in `src/lock.rs`, `src/integrate.rs`, and `src/install.rs`.
   - Epoch-based nonce generation was implemented independently in multiple modules.

6. **Clear Separation of Concerns to Preserve**:
   - The `Resolver` is cleanly decoupled from concrete Git or HTTP implementations via `PackageCatalog`. This invariant must be strictly preserved.
   - Lockfile data structures (`Lockfile`, `LockedPackage`) are separate from CLI execution.
   - CMake execution uses safe argument vectors (`std::process::Command`), never shell interpolation.

---

## 5. Target Modular Architecture

```text
src/
├── main.rs
├── lib.rs
│
├── core/                        # Genuinely shared foundational infrastructure
│   ├── mod.rs
│   ├── filesystem.rs            # Atomic file writing, directory helpers
│   └── paths.rs                 # Lexical path normalization, path traversal guards
│
├── package/                     # Package domain model & identity
│   ├── mod.rs
│   ├── identity.rs              # PackageName, PackageId, PackageError
│   ├── metadata.rs              # PackageMetadata
│   └── dependency.rs            # Dependency, DependencyError
│
├── project/                     # Project lifecycle & manifest management
│   ├── mod.rs
│   ├── manifest.rs              # Manifest, ManifestError (crux.toml)
│   ├── init.rs                  # init_project, InitSummary, InitError
│   └── check.rs                 # check_project, CheckReport, CheckError
│
├── resolution/                  # Dependency resolution & lockfile persistence
│   ├── mod.rs
│   ├── catalog.rs               # PackageCatalog trait, CatalogEntry, InMemoryCatalog
│   ├── resolver.rs              # Resolver, ResolutionError, ResolvedGraph
│   └── lockfile.rs              # Lockfile, LockedPackage, Checksum, PackageSource, sync
│
├── registry/                    # Git-backed package registry
│   ├── mod.rs                   # GitRegistry, add_registry, list_registries, update_registry
│   ├── config.rs                # CruxConfig, RegistryConfig, load_config
│   ├── git.rs                   # GitOps trait, SystemGitOps
│   └── metadata.rs              # PackageDefinition, RegistryManifest
│
├── source/                      # Source archive acquisition & extraction
│   ├── mod.rs                   # SourceAcquirer, AcquiredSource, AcquisitionError
│   ├── cache.rs                 # SourceCache
│   ├── download.rs              # HttpTransport, UreqHttpTransport, MockHttpTransport
│   └── extraction.rs            # Safe extraction, ArchiveFormat, bomb limits, zip/tar
│
├── build/                       # CMake compilation engine
│   ├── mod.rs                   # BuildEngine trait, CMakeBuildEngine, BuildRequest, BuildResult
│   ├── fingerprint.rs           # BuildFingerprint, BuildProfile
│   ├── toolchain.rs             # ToolchainInfo, toolchain detection
│   └── metadata.rs              # BuildMetadata, BuildArtifact, ArtifactType
│
├── install/                     # Staged package installation
│   ├── mod.rs                   # InstallEngine trait, CMakeInstallEngine, InstallRequest, InstallResult
│   ├── metadata.rs              # InstallMetadata, CURRENT_INSTALL_METADATA_VERSION
│   └── inventory.rs             # InstalledFile, InstalledFileType, StagingGuard
│
├── integrate/                   # Consumer build-system integration
│   ├── mod.rs                   # CMakeIntegrationGenerator, ValidatedPackageInstallation
│   ├── cmake.rs                 # generate_cmake_dependencies, escape_cmake_path
│   └── validation.rs            # validate_package_installation, collect_transitive_dependencies
│
└── cli/                         # Command-line interface & subcommand handlers
    ├── mod.rs                   # CLI entry point (`run`), Action, CliError
    ├── args.rs                  # Argument parsers, help messages
    └── commands/                # Isolated command orchestration modules
        ├── mod.rs
        ├── init.rs
        ├── check.rs
        ├── lock.rs
        ├── resolve.rs
        ├── registry.rs
        ├── fetch.rs
        ├── build.rs
        ├── install.rs
        └── integrate.rs
```

---

## 6. Proposed Incremental Migration Sequence

To prevent regressions, the refactoring will proceed in 10 sequential, verifiable phases. After each step, `cargo check` and `cargo test` will be executed:

1. **Step 1: Foundational Core (`src/core/`)**:
   - Establish `src/core/paths.rs` and `src/core/filesystem.rs`.
   - Migrate `normalize_path` and `write_atomically` into `core`.
2. **Step 2: Package Domain Model (`src/package/`)**:
   - Organize `identity.rs`, `metadata.rs`, `dependency.rs`.
   - Maintain re-exports in `src/package/mod.rs` and top-level `src/lib.rs`.
3. **Step 3: Project Management (`src/project/`)**:
   - Group `manifest.rs`, `init.rs`, and `check.rs` under `src/project/`.
4. **Step 4: Resolution & Lockfile (`src/resolution/`)**:
   - Group `catalog.rs`, `resolver.rs`, and `lockfile.rs` under `src/resolution/`.
   - Ensure the resolver remains completely decoupled from network and concrete Git implementations.
5. **Step 5: Registry Subsystem (`src/registry/`)**:
   - Group `git.rs`, `config.rs`, `metadata.rs` under `src/registry/`.
6. **Step 6: Source Acquisition (`src/source/`)**:
   - Modularize `cache.rs`, `download.rs`, `extraction.rs`.
7. **Step 7: Build Engine (`src/build/`)**:
   - Modularize `fingerprint.rs`, `toolchain.rs`, `metadata.rs`.
8. **Step 8: Package Installation (`src/install/`)**:
   - Modularize `metadata.rs`, `inventory.rs`, `engine.rs`.
9. **Step 9: CMake Integration (`src/integrate/`)**:
   - Modularize `cmake.rs`, `validation.rs`.
10. **Step 10: CLI Command Decoupling (`src/cli/`)**:
    - Split `src/cli.rs` into `args.rs` and individual command modules under `src/cli/commands/`.
    - Keep `src/main.rs` minimal.

---

## 7. Public API Compatibility & Risk Mitigation

- **Single Cargo Crate**: Crux remains a single crate `crux`. No multi-crate workspace complexity is introduced.
- **Backwards-Compatible Re-exports**: All existing public types in `src/lib.rs` (e.g. `crux::package::*`, `crux::resolver::*`, `crux::lock::*`, `crux::build::*`, `crux::install::*`, `crux::integrate::*`) will be preserved using `pub use` aliases.
- **Zero Schema Changes**: The serialization formats of `crux.toml`, `crux.lock`, `package.toml`, `installed.toml`, and `.crux/cmake/CruxDependencies.cmake` will remain 100% byte-for-byte compatible.
- **Integration Test Safety**: Existing integration tests in `tests/*.rs` will continue to compile and pass against public crate APIs.

---

## 8. Test Protection Matrix

The test suite consists of 294 tests across 14 test binaries:
- `src/lib.rs` (107 unit tests)
- `tests/build.rs` (12 tests)
- `tests/check.rs` (9 tests)
- `tests/cli.rs` (4 tests)
- `tests/cmake_integration.rs` (23 tests)
- `tests/dependency.rs` (14 tests)
- `tests/init.rs` (6 tests)
- `tests/install.rs` (15 tests)
- `tests/lock.rs` (17 tests)
- `tests/lock_sync.rs` (15 tests)
- `tests/package.rs` (10 tests)
- `tests/registry.rs` (17 tests)
- `tests/resolver.rs` (23 tests)
- `tests/source.rs` (22 tests)

Every refactoring phase must achieve zero regressions across all 294 tests.
