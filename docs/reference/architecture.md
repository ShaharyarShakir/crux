# Crux Architecture Guide

This document describes the architectural design, core invariants, data structures, and component interactions of **Crux**.

---

## 1. System Architecture Overview

Crux is organized into cohesive domain directories within a single Cargo crate:

```text
+-----------------------------------------------------------------------------------------+
|                                    CLI Subsystem (src/cli/)                             |
|  - args.rs (parsing, help text)                                                         |
|  - commands/ (init, import, check, lock, resolve, registry, fetch, build, run,          |
|    install, integrate, toolchain)                                                      |
|  - mod.rs (top-level run dispatcher, CliError)                                          |
+-----------------------------------------------------------------------------------------+
       |                  |                |                  |                 |
       v                  v                v                  v                 v
+--------------+  +---------------+  +------------+  +-----------------+  +---------------+
| Project      |  | Resolution    |  | Registry   |  | Source          |  | Build Engine  |
| (src/project)|  | (src/resolut- |  | (src/regi- |  | (src/source/)   |  | (src/build/)  |
| - manifest   |  |   ion/)       |  |   stry/)   |  | - download      |  | - toolchain   |
| - init       |  | - resolver    |  | - git ops  |  | - extraction    |  | - fingerprint |
| - check      |  | - catalog     |  | - metadata |  | - cache         |  | - engine      |
| - crux.toml  |  | - lockfile    |  | - config   |  |                 |  | - metadata    |
+--------------+  +---------------+  +------------+  +-----------------+  +---------------+
       |                  ^                ^                  ^                 ^
       |                  |                |                  |                 |
       +------------------+----------------+------------------+                 |
                          |                                                     |
                          v                                                     |
                  +---------------+                                             v
                  | Package       |                                    +-----------------+
                  | (src/package/)|                                    | Install Engine  |
                  | - identity    |                                    | (src/install/)  |
                  | - metadata    |                                    | - inventory     |
                  | - dependency  |                                    | - engine        |
                  +---------------+                                    | - metadata      |
                          ^                                            +-----------------+
                          |                                                     |
                  +---------------+                                             v
                  | Core Platform |                                    +-----------------+
                  | (src/core/)   |                                    | Consumer        |
                  | - paths       |                                    | Integration     |
                  | - filesystem  |                                    | (src/integrate/)|
                  +---------------+                                    | - validation    |
                                                                       | - cmake script  |
                                                                       +-----------------+
```

---

## 2. Modular Subsystem Directory Layout

Crux adheres to strict separation of concerns, organized into the following module hierarchy:

### A. Foundational Core (`src/core/`)
- **`paths.rs`**: Canonical path normalization (`normalize_path`) and path traversal security guards (`is_safe_relative_path`).
- **`filesystem.rs`**: Safe atomic file replacement patterns (`write_atomically`) and random nonces.
- **`process.rs`**: Safe subprocess execution with captured stdout/stderr, closed stdin, and bounded timeouts (`run_with_timeout`).

### B. Package Domain Model (`src/package/`)
- **`identity.rs`**: `PackageName` (strictly validated identifier `^[a-zA-Z0-9][a-zA-Z0-9_-]*$`) and `PackageId` (`name version` pair with SemVer 2.0.0 ordering).
- **`metadata.rs`**: Model for package specifications (name, version, description, license, URLs).
- **`dependency.rs`**: Strongly-typed `Dependency` requirements and version constraints (caret, tilde, comparator ranges).

### C. Project Management (`src/project/`)
- **`manifest.rs`**: `Manifest` specification, parsing, and validation for `crux.toml`.
- **`init.rs`**: Package initialization logic (`init_project`) and automated name derivation.
- **`check.rs`**: Manifest validation (`check_project`) and structured diagnostic reporting.
- **`discovery.rs`**: Safe, read-only existing-project discovery (`discover_project`) with deterministic file ordering, build-system detection, CMake project-name parsing, conventional source/include/test-directory detection, entry-point heuristics, ignored-directory filtering, and symlink-escape warnings.
- **`import.rs`**: `crux import` domain logic (`import_project`, `ImportMetadata`, `format_import_report`). Writes a conservative manifest and Crux-managed `.crux/import.toml`; never modifies user files or overwrites an existing manifest.

### D. Dependency Resolution Engine & Lockfile (`src/resolution/`)
- **`catalog.rs`**: Decoupled `PackageCatalog` trait and `InMemoryCatalog` fixture.
- **`lockfile.rs`**: Versioned lockfile model (`Lockfile`, `LockedPackage`, `Checksum`), lockfile synchronization (`check_lockfile_sync`), and atomic persistence.
- **`resolver.rs`**: Deterministic backtracking solver (`Resolver`), `ResolvedGraph`, C/C++ single-version ODR safety enforcement, and cycle detection.

### E. Git Registry Subsystem (`src/registry/`)
- **`metadata.rs`**: Declarative `package.toml` schema, `PackageDefinition`, and `RegistryManifest`.
- **`git.rs`**: `GitRegistry` implementation of `PackageCatalog` and safe shell-free `git` subprocess execution (`GitOps`).
- **`config.rs`**: Persistent configuration models (`CruxConfig`, `RegistryConfig`, `config.toml`).

**M22 official catalog**: the in-tree `registry/` directory now contains 50 canonical package identities and 53 concrete versions. `[build].system` is recorded as `cmake` for the verified tier and `catalog-only` for discoverable-but-unsupported entries. CLI build/install paths reject non-CMake definitions before acquisition, keeping the build engine strictly CMake-only while allowing the registry to document a broader C/C++ ecosystem. The reference applications under `examples/m22/` — five C++ applications plus the pure C `c-zlib-roundtrip` application — exercise verified packages end to end, and `tests/official_registry.rs` provides offline-safe catalog consistency checks. See `docs/guides/official-package-catalog.md` and `docs/reports/package-ecosystem-expansion-report.md`.

### F. Source Acquisition, Verification, and Extraction (`src/source/`)
- **`download.rs`**: Streaming HTTP/HTTPS download with inline SHA-256 calculation and security scheme enforcement.
- **`extraction.rs`**: Safe archive extraction (.zip, .tar, .tar.gz) with Zip Slip / symlink jail / decompression bomb protection.
- **`cache.rs`**: Content-addressed local source and archive cache (`SourceCache`).

### G. CMake Build Engine Subsystem (`src/build/`)
- **`metadata.rs`**: Build profiles (`Debug`, `Release`), configuration models, and artifact classification.
- **`toolchain.rs`**: Project-aware compiler and standard detection (`ProjectToolchain`, `CompilerInfo`, `probe_standard_support`), honoring explicit `CC`/`CXX` overrides.
- **`fingerprint.rs`**: Deterministic SHA-256 build fingerprint generation across all inputs.
- **`engine.rs`**: Out-of-source CMake build execution (`CMakeBuildEngine`), subprocess orchestration, and log capture for registry packages.
- **`discovery.rs`**: Recursive application source discovery with deterministic ordering, header filtering, language-mismatch rejection, and symlink confinement. `discover_sources` preserves canonical `src/`-only behavior; `discover_sources_with_roots` supports safe source roots recorded by `crux import` and conventional `source/`, `app/`, or root-level layouts when no `src/` exists.
- **`generator.rs`**: Deterministic internal CMake project generation for applications (`AppCMakeConfig`, `generate_app_cmakelists`), including CMake string escaping and standard selection.
- **`app.rs`**: Application build planning and execution (`AppBuildModel`, `AppBuildEngine`, `AppTarget`, `AppBuildLayout`). Separates prepare, generate, configure/compile, and report stages while reusing M17 project config and M18 toolchain detection.

### H. Package Installation Engine Subsystem (`src/install/`)
- **`metadata.rs`**: Persistent `installed.toml` metadata schema (`InstallMetadata`).
- **`inventory.rs`**: Installed file inventory scanning, deterministic sorting, and `InstalledFileType` classification.
- **`engine.rs`**: Project-local staging, symlink escape jail, and atomic publication engine (`CMakeInstallEngine`).

### I. CMake Consumer Integration Subsystem (`src/integrate/`)
- **`validation.rs`**: Installation integrity validation prior to integration (`validate_package_installation`).
- **`cmake.rs`**: Self-locating, relocatable `.crux/cmake/CruxDependencies.cmake` generator (`CMakeIntegrationGenerator`).

### J. Command-Line Interface (`src/cli/`)
- **`args.rs`**: CLI argument structures (`InitArgs`, `ImportArgs`, `LockArgs`), help text constants, and parsers.
- **`commands/`**: Modular command runners (`init`, `import`, `check`, `lock`, `resolve`, `registry`, `fetch`, `build`, `run`, `install`, `integrate`, `toolchain`).
- **`mod.rs`**: Top-level execution dispatcher (`run`) and structured `CliError`.
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
- **Local Configuration (`CruxConfig`) & Official Registry Auto-Bootstrap**:
  - Stored in `.crux/config.toml` (project) or `~/.crux/config.toml` (user-global), with atomic write patterns.
  - All catalog-backed production commands (`lock`, `resolve`, `add`, `remove`, `fetch`, `build <pkg>`, `install`, `integrate --locked`) share one registry-selection path. There is no production fallback to the in-memory test catalog.
  - Fails with explicit errors if a configured registry fails to open. `CRUX_OFFLINE=1` disables bootstrap cloning and requires an existing local registry; the error explains how to configure one.
  - **Official Registry Bootstrap (`ensure_official_registry`)**: Automatically configures the official Crux registry (`https://github.com/ShaharyarShakir/crux.git`) for fresh installations when no registry exists. Handles repository subdirectories (`subdir = "registry"`), caches checkouts under `~/.crux/registries/official`, and reuses cached checkouts without network requests for offline reliability. Development builds transparently reuse the bundled in-tree registry root. Custom user-configured registries are preserved and never overwritten.

### H. Source Acquisition, Verification and Extraction Subsystem (`src/source.rs`)
- **Decoupled Architecture**:
  - Completely decoupled from the resolution engine and registry index.
  - The resolver operates exclusively over version constraints and package identities.
  - The registry supplies declarative `PackageDefinition` metadata (`PackageSourceDefinition::Archive`).
  - `SourceAcquirer` coordinates streaming download, integrity verification, content-addressed caching, and safe extraction into `AcquiredSource`.
- **Streaming HTTP/HTTPS & Security Boundaries**:
  - Enforces safe URL schemes: only `http://` and `https://` are permitted; `file://`, `ftp://`, `ssh://`, and pseudo-schemes are rejected.
  - Streams responses chunk by chunk directly into temporary files, hashing raw bytes on the fly with `sha2::Sha256`.
  - Configurable download timeout (default 30s) and maximum archive size (default 500 MB) to prevent denial-of-service disk exhaustion attacks.
  - Safe redirect following (maximum 5 redirects, strictly confined to HTTP/HTTPS).
- **Cryptographic Integrity Verification**:
  - Strictly verifies computed SHA-256 digests against expected package definition / lockfile checksums before any archive is cached or extracted.
  - On mismatch, raises structured `AcquisitionError::ChecksumMismatch { expected, actual }`, cleans up temporary download files immediately, and never corrupts or overwrites existing valid cached archives.
- **Content-Addressed Local Cache (`SourceCache`)**:
  - Configurable root directory (`CRUX_CACHE_DIR`, `CruxConfig::cache_dir`, `XDG_CACHE_HOME`, or `~/.cache/crux`).
  - **Archive Cache**: `<cache_root>/archives/sha256/<64_char_digest>`
  - **Source Cache**: `<cache_root>/sources/sha256/<64_char_digest>/`
  - **Cache Hit Optimization**:
    1. If the extracted source directory exists and contains the completion sentinel (`.crux_complete`), acquisition returns immediately with zero disk extraction and zero network calls.
    2. If the source directory is absent but the cached archive exists and satisfies SHA-256 verification, acquisition extracts directly without downloading.
    3. If a cached archive fails cryptographic verification (corrupted on disk), it is purged and automatically re-downloaded.
  - **Atomic Replacement**: Employs atomic rename and RAII guards (`TempFileGuard`, `TempDirGuard`) to ensure temporary files are staged in `<cache_root>/tmp/` and atomically promoted. Partial files or failed extractions never leave corrupt artifacts in cache.
- **Defense-in-Depth Safe Extraction**:
  - **Archive Formats**: Supports `.zip`, `.tar.gz`, `.tgz`, and `.tar`, detected via URL extension, filename, and file magic bytes sniffing (`PK\x03\x04`, `\x1f\x8b`, `ustar`).
  - **Path Traversal ("Zip Slip") Protection**: Rejects archive entries with `..` components, absolute paths, leading slashes, Windows drive letters (`C:`), or paths resolving outside the extraction root.
  - **Symlink & Hardlink Jail**: Rejects symlinks with absolute targets or targets escaping the destination tree. Hardlinks must point to existing extracted files within the destination root.
  - **Decompression Bomb Protection**: Enforces strict maximum uncompressed size limits (default 1 GB) and maximum entry counts (default 50,000 files). Halts immediately if thresholds are crossed.
  - **Device Node Rejection**: Rejects FIFOs, sockets, block devices, and character devices.
  - **Safe Permissions**: Applies 0o755 for directories, 0o755 for executables, and 0o644 for regular files, stripping setuid, setgid, and sticky bits.
  - **Top-Level Directory Unwrapping**: If an archive wraps all contents in a single common root directory (e.g. `fmt-11.1.1/`), the directory is safely unwrapped so source files and `CMakeLists.txt` reside at the top of `<source_dir>`.
- **Command-Line Interface (`crux fetch`)**:
  - `crux fetch <package>` or `crux fetch <package>@<version>`: Looks up package definition in active registry, acquires source, verifies integrity, extracts to cache, and displays destination path.
  - `crux fetch` (no argument): Fetches and verifies all project dependencies declared in `crux.lock` (or resolves from `crux.toml`).
  - Supports `--force` flag to bypass cache and re-download and re-extract.

### I. Build Engine Subsystem (`src/build/`)

#### Application Build Engine (M19)

Crux manages local C/C++ applications end to end while treating CMake strictly as an internal backend. Applications and registry packages share toolchain detection primitives but use separate build flows and output roots.

- **Build Model & Staged Pipeline**:
  - `AppBuildModel::prepare` loads and validates `crux.toml`, resolves the effective language and standard, detects the compiler through M18, discovers sources under `src/`, detects CMake, and computes the deterministic layout. It writes nothing.
  - `AppBuildEngine::execute` generates internal CMake configuration, configures out-of-source, compiles, validates the expected executable, and returns `AppBuildResult`. `AppBuildEngine::build` is a convenience wrapper for prepare + execute.
  - `AppTarget` and `AppTargetKind` model a single executable target derived from the project name. The model is structured so future milestones can add library and multi-target support without rewriting the engine.
- **Source Discovery Rules**:
  - Only files under `<project_root>/src/` are application inputs.
  - C projects accept `.c`; C++ projects accept `.cpp`, `.cc`, and `.cxx` (case-insensitive extension matching).
  - Discovery is recursive, ignores hidden files/directories and headers, sorts deterministically, and rejects unsupported/foreign source extensions and mixed C/C++ compilation units.
  - Symlinked files and directories must resolve inside the canonical `src/` tree; escapes are rejected and symlink cycles are bounded by a canonical visited-directory set.
- **Internal CMake Generation**:
  - Generated exclusively to `.crux/build/generated/CMakeLists.txt` via atomic writes. No root-level `CMakeLists.txt` is created or modified.
  - Declares the project language, selects the configured C/C++ standard, sets `*_STANDARD_REQUIRED ON`, and disables compiler extensions (`*_EXTENSIONS OFF`).
  - Injects every discovered source as an escaped, quoted CMake path relative to the project root, and sets explicit runtime output directories for single- and multi-configuration generators.
  - CMake string values escape backslashes, quotes, `$`, `;`, newlines, carriage returns, and tabs. Source paths must remain under the project root.
- **Profiles & Layout**:
  - Debug is the default application configuration; Release is selected with `--release`.
  - `.crux/build/debug/` and `.crux/build/release/` are fully isolated build trees with separate executables.
  - Logs are written to `.crux/build/logs/configure-<profile>.log` and `.crux/build/logs/build-<profile>.log`.
  - Existing CMake caches are invalidated when the selected compiler or generator differs from the cached values, preventing stale cross-compiler configuration.
- **Compiler & Process Safety**:
  - Reuses M18 detection and honors `CC`/`CXX`; never silently switches compilers after a selected toolchain fails.
  - Verifies CMake is executable before configuration and reports missing toolchains clearly.
  - All CMake and compiler invocations use argument vectors with stdin closed, captured streams, and bounded timeouts (30s probes, 10min configure, 30min build).
  - Deterministic single-configuration generators are preferred: Ninja when available, otherwise Unix Makefiles.
- **Dispatch & Backward Compatibility**:
  - `crux build <package>` builds a registry package.
  - `crux build` with a `[project]` manifest builds the local application.
  - `crux build` with a legacy `[package]` manifest or only `crux.lock` retains v0.1.0 locked-dependency build behavior.

#### Application Execution Engine (`crux run`, M20)

`crux run` builds and executes the current local C or C++ application in a single developer command:

- **Build Engine Reuse**:
  - Reuses the M19 application build engine directly via `AppBuildEngine::prepare` and `AppBuildEngine::execute`.
  - Does not implement an independent build system or freshness cache; leverages CMake's native incremental compilation.
  - Defaults to `Debug`; supports `--debug` and `--release` configurations, executing the corresponding isolated executable from `.crux/build/debug/` or `.crux/build/release/`.
- **Pre-execution Project & Manifest Validation**:
  - Confirms `crux.toml` exists and contains a `[project]` manifest.
  - Validates that the application language is C or C++; projects with `language = "other"` receive an actionable diagnostic and execution is aborted.
- **Executable Validation & Confinement**:
  - Retrieves the exact executable path from the M19 `AppBuildResult` (no guessing or independent path layout reconstruction).
  - Confirms the binary exists, is a regular file, and has executable permissions where the platform supports this check.
  - Confines executable resolution to the project-local `.crux/build/` directory via canonicalization, preventing execution of arbitrary binaries outside the Crux build hierarchy.
  - Build failures immediately abort execution; Crux never executes a stale executable from a prior build.
- **Process Execution & Argument Forwarding**:
  - Launches the application directly via `std::process::Command` using explicit argument vectors; never spawns a shell or uses `sh -c` / `bash -c`.
  - Application arguments after `--` are forwarded unchanged with ordering, spaces, and leading dashes preserved.
  - Inherits standard input, output, and error (`Stdio::inherit()`) so interactive terminal applications and unbuffered streams work natively.
  - Sets the child process working directory to the project root (`current_dir(project_root)`), ensuring relative file paths resolve predictably in the project workspace rather than inside `.crux/build/`.
  - Inherits the parent process environment by default without clearing or modifying environment variables.
- **Exit Status & Signal Propagation**:
  - If the application exits with code 0, Crux exits successfully (code 0).
  - If the application exits with a nonzero code, Crux propagates that exact code to the shell via `CliError::AppExit`.
  - On Unix, if the application terminates due to an unhandled signal, the exit code is mapped to `128 + signal`.

#### Ad-hoc Source Execution (`crux run --bin` / `--sources`)

Manifest-free `crux run` uses the same `AppBuildEngine` and CMake generator as project applications, with a synthesized in-memory `ProjectConfig` and an empty dependency plan:

- **Input Planning (`src/build/adhoc.rs`)**:
  - `--bin hello.c` is normalized into a single source input; `--sources` accepts repeatable flags and/or comma-separated lists.
  - Explicit source and header paths are canonicalized, deduplicated, classified by extension, and copied into a Crux-managed scratch workspace under `.crux/run/<target>-<hash>/src/`.
  - `.c` selects C; `.cpp`, `.cc`, and `.cxx` select C++. Mixed compilation units and unsupported extensions are rejected before CMake runs.
  - Header files listed in `--sources` are copied but never compiled as translation units.
  - `-I`, `--include`, and `--include-dir` add canonical include roots to the generated target; relative paths resolve from the caller's working directory.
- **Deterministic Isolation**:
  - The scratch directory name combines a sanitized target name with a stable fingerprint of the working directory, sources, and include roots.
  - The scratch tree is removed before each ad-hoc run, so a failed compile cannot leave an executable that a later invocation might accidentally launch.
  - Debug and Release outputs remain isolated in `.crux/build/debug/` and `.crux/build/release/` inside the scratch workspace.
- **Execution Semantics**:
  - No `crux.toml`, lockfile, registry, or package installation is required.
  - Arguments after `--`, working-directory behavior, inherited streams, executable confinement, and exit-status propagation are identical to project `crux run`.

#### Dependency Management & Application Integration (`crux add` / `crux remove`, M21)

Crux delivers an end-to-end dependency management workflow that seamlessly connects project manifests, lockfile synchronization, package installation, and application compilation:

- **Manifest & Lockfile Transactional Edits (`commit_manifest_and_lockfile`)**:
  - `crux add <package>[@<constraint>]`:
    - Defaults to `"*"` (any compatible version) when constraint is omitted.
    - Resolves the complete graph (including transitive dependencies) using the deterministic `Resolver`, reusing existing lockfile preferences to prevent version churn.
    - Modifies only the `[dependencies]` table via `upsert_dependency_in_toml`, preserving comments, order, and unrelated TOML structure.
    - Atomically writes `crux.toml` followed by `crux.lock`. If lockfile writing fails, `crux.toml` is automatically restored to its previous state.
    - Automatically invokes `run_install` so newly declared dependencies are installed for the default Debug profile immediately.
  - `crux remove <package>`:
    - Validates that the target is a declared direct dependency; rejects nonexistent packages with actionable diagnostics listing declared dependencies.
    - Re-resolves the remaining dependency graph before writing to disk, ensuring shared transitive dependencies required by other packages remain locked.
    - Removes only the direct entry via `remove_dependency_from_toml` and commits both files transactionally.
    - Leaves installed files intact to avoid unsafe or unverified cache purges.
- **Application Dependency Link Planning (`plan_app_dependencies`)**:
  - Traverses the locked package graph topologically (dependents before dependencies) using depth-first search.
  - Validates package installation integrity via `validate_package_installation` for the selected profile (`Debug` or `Release`), checking fingerprints, installation metadata, and required files.
  - Classifies installed artifacts (`InstalledFileType`):
    - Include directories (`include/`).
    - Library search directories (`lib/`, `lib64/`).
    - Static archives (`.a`, `.lib`) and shared libraries (`.so`, `.dylib`, `.dll`).
    - Prefers static archives for portable binary output; uses shared libraries if no static archive is present.
    - Collects runtime shared-library directories for `BUILD_RPATH` injection and runtime environment setup (`LD_LIBRARY_PATH`, `DYLD_LIBRARY_PATH`, `PATH`).
- **Internal CMake Application Generation (`generate_app_cmakelists`)**:
  - Automatically emits target dependency rules for Crux-managed application targets:
    - `target_include_directories(<target> PRIVATE "<dir>" ...)`
    - `target_link_directories(<target> PRIVATE "<dir>" ...)`
    - `target_link_libraries(<target> PRIVATE "<lib>" ...)`
    - `set_target_properties(<target> PROPERTIES BUILD_RPATH "<rpath>")`
  - Relieves developers from writing or maintaining CMake dependency logic.
- **Automatic Dependency Synchronization in Build and Run**:
  - `crux build` and `crux run` invoke `ensure_app_dependencies` before CMake configuration.
  - If any declared dependency is not installed for the active profile (e.g. running `crux run --release` when only Debug is installed), Crux automatically builds and installs the missing packages.
  - No misleading success messages are printed when an application terminates with a nonzero status.

#### Existing Project Import (`crux import`, M23)

`crux import` adapts existing C/C++ projects without creating a second build engine or changing the manifest schema:

- **Read-Only Discovery (`src/project/discovery.rs`)**:
  - Canonicalizes the target directory, rejects non-directories, and scans deterministically with sorted directory entries and ordered collections.
  - Detects `CMakeLists.txt`, `Makefile`/`makefile`/`GNUmakefile`, and pre-existing `crux.toml`.
  - Parses simple top-level CMake `project()` declarations for a name hint.
  - Detects conventional source roots (`src/`, `source/`, `app/`), include roots (`include/`, `inc/`, `headers/`), root-level compilation units and headers, likely entry points (`main.*` or top-level `int main`), multiple targets, and test directories.
  - Skips `.git`, `.crux`, `build`, `out`, `dist`, `node_modules`, `target`, `vendor`, hidden paths, and symlinks resolving outside the project root.
- **Conservative Manifest Generation (`src/project/import.rs`)**:
  - Uses `Manifest`, `ProjectLanguage`, validation, and serialization from `src/project/manifest.rs`; no invented manifest keys or guessed dependency edges.
  - Derives a valid name from CMake metadata or the directory name and applies safe language-standard defaults.
  - Records observed source roots and include directories in versioned, Crux-managed `.crux/import.toml` rather than extending `crux.toml`.
  - Uses create-only writes for new files and atomic replacement only for refreshed Crux-managed metadata with explicit `--force` notice.
  - Refuses to overwrite an existing `crux.toml` under all circumstances, including `--force`.
  - Rejects projects with no C/C++ compilation units or mixed C/C++ sources; requires `--force` for missing entry points or multiple executable targets.
- **Build Engine Reuse**:
  - `AppBuildModel::prepare_with_cmake` loads `.crux/import.toml` and passes recorded source roots to `discover_sources_with_roots`.
  - Recorded include directories are validated as safe project-relative paths and emitted as `${CRUX_PROJECT_ROOT}/...` entries in the internal CMake target.
  - Existing CMake and Make files are preserved but not executed or translated; `crux build` and `crux run` continue to use the same internal CMake generator and build layout.
- **Reporting**:
  - The import report distinguishes detected facts, assumptions, limitations, created files, preserved files, and next steps.
  - Makefile and CMake compatibility limits are reported explicitly, and dry runs print the proposed manifest without writing files.

See `docs/guides/import-existing-projects.md` and `docs/reports/project-import-migration-report.md`.

#### Registry Package Build Engine (M11)

- **Out-of-Source Isolated Builds**:
  - Strict physical separation: the acquired source directory (`<source_dir>`) is never modified or polluted in-place (no `CMakeCache.txt` or `CMakeFiles/` in source tree).
  - Out-of-source build hierarchy: `.crux/builds/<package_name>/<package_version>/<fingerprint_hex>/`
    - `build/`: CMake build tree destination directory (`-B`)
    - `logs/configure.log`: Full stdout and stderr capture from CMake configuration
    - `logs/build.log`: Full stdout and stderr capture from CMake compilation
    - `metadata.toml`: Canonical serialization of `BuildMetadata`
- **Deterministic Build Fingerprint (`BuildFingerprint`)**:
  - Computes a cryptographic SHA-256 digest (`sha256:<64_hex_chars>`) across:
    1. Package identity (`<name>@<version>`)
    2. Source checksum (`sha256:<digest>`)
    3. Build profile (`debug` or `release`)
    4. Detected CMake version
    5. Detected CMake generator (`Unix Makefiles`, `Ninja`)
    6. Detected C/C++ compiler identities and versions
    7. Shared library flag (`BUILD_SHARED_LIBS`)
    8. Sorted user/package CMake arguments
    9. Sorted environment variable overrides
  - Purely functional and reproducible: contains no timestamps, random nonces, or environment-dependent absolute paths.
- **Toolchain & Compiler Detection**:
  - Safely queries system toolchain without fragile regex parsing:
    - Runs `cmake --version` to verify CMake availability and extract version string.
    - Runs compiler queries (`gcc --version`, `g++ --version`, `clang --version`, `cc --version`).
    - Detects generator availability (`Ninja` if present, falling back to `Unix Makefiles`).
- **Safe Subprocess Execution**:
  - Exclusively invokes `std::process::Command` using explicit, discrete argument arrays (`&[&str]` / `&[String]`).
  - Zero shell interpolation: never executes commands through `sh -c`, `bash -c`, or raw shell wrappers.
  - Safe against directory paths or flags containing spaces, special characters, or quotes.
- **Diagnostic Logging & Structured Failures**:
  - Captures complete stdout and stderr streams into disk log files (`configure.log`, `build.log`).
  - On configuration or compilation failure, extracts the trailing diagnostic lines (last 20 lines) into actionable `BuildError::ConfigureFailed` or `BuildError::BuildFailed` reports.
- **Idempotency & Rebuild Caching**:
  - Before invoking CMake, checks for existing valid `metadata.toml` matching the computed `BuildFingerprint`.
  - If existing artifacts are verified on disk and `--force` is not passed, immediately returns a cached `BuildResult` (`reused_cache: true`) without executing CMake.
  - Passing `--force` triggers a clean build by wiping the build tree and re-executing configure and build.
- **Command-Line Interface (`crux build`)**:
  - `crux build <package>` or `crux build <package>@<version>`: Acquires package source, executes CMake build, and reports artifacts and duration.
  - `crux build` (no argument) with a legacy `[package]` manifest or only `crux.lock`: Builds all dependencies declared in `crux.lock` in dependency order.
  - `crux build` (no argument) with a `[project]` manifest: Builds the local application through the M19 application build engine.
  - Flags:
    - `--debug` / `--release`: Selects the application configuration (default: `debug`).
    - `--profile <debug|release>`: Explicit build profile (default: `release` for packages, `debug` for applications).
    - `--verbose`: Shows detected toolchain paths and full CMake/compiler output.
    - `--force`: Rebuilds even if a cached build is present.

### J. Package Installation Engine Subsystem (`src/install.rs`)
- **Project-Local Installation Hierarchy**:
  - Deterministic location based on package identity and build environment:
    `.crux/installed/<package_name>/<package_version>/<fingerprint_hex>/`
    - `bin/`: Executables and CLI tools
    - `include/`: C/C++ public header files
    - `lib/` or `lib64/`: Static libraries (`.a`, `.lib`) and shared libraries (`.so`, `.dylib`, `.dll`)
    - `share/`: Pkg-config files (`share/pkgconfig/`), CMake target configs (`share/cmake/`), manual pages, documentation
    - `installed.toml`: Canonical persistent metadata and file inventory
- **Staging & Atomic Publication Pipeline**:
  - Direct installation into final destinations is strictly prohibited.
  - Staging directory: `.crux/staging/<package>_<version>_<pid>_<nanos>/`
  - Invokes `cmake --install` using safe argument arrays (zero shell interpolation):
    - For packages with standard relative destinations: `cmake --install <build_dir> --prefix <staging_dir> --config <Profile>`
    - For packages declaring absolute destinations (`CMAKE_ABSOLUTE_DESTINATION_FILES`, e.g. zlib): `DESTDIR=<staging_dir> cmake --install <build_dir> --prefix / --config <Profile>`, followed by safe prefix promotion (`promote_unwrapped_prefix`) to normalize `usr/local` contents into the Crux prefix root.
  - Generates comprehensive install execution logs (`install.log` in the package's build logs directory).
  - Validates all staged files against security constraints before publication.
  - Writes `installed.toml` atomically into staging root (`.installed.toml.tmp` -> rename).
  - Atomic directory replacement:
    - If target directory already exists, moves existing directory to a temporary backup alongside the destination (`<dest>.old.<unique_id>`).
    - Atomically renames staging directory to destination directory.
    - On success: permanently cleans up backup directory.
    - On failure: immediately rolls back backup to target directory and preserves previous valid installation.
  - RAII `StagingGuard` guarantees complete removal of temporary staging directories upon early error or panic.
- **Installed File Inventory & Classification (`InstalledFileType`)**:
  - Scans and indexes every non-metadata file installed into staging.
  - Deterministic alphabetical sorting by relative path.
  - File type classification:
    - `Header`: C/C++ header files (`.h`, `.hpp`, `.hxx`, `.hh`, `.inc`, `.inl`, or in `include/`)
    - `StaticLibrary`: Static archives (`.a`, `.lib`)
    - `SharedLibrary`: Shared libraries and versioned sonames (`.so`, `.dylib`, `.dll`, `.so.*`)
    - `Executable`: Binaries in `bin/` or with Unix executable mode permissions
    - `PkgConfig`: Pkg-config metadata (`.pc` files or in `pkgconfig/`)
    - `CMakeModule`: CMake package configuration and exported targets (`*Config.cmake`, `*-config.cmake`, `*.cmake`)
    - `Other`: Licenses, manuals, documentation, data files
- **Strict Filesystem & Security Boundaries**:
  - **Path Traversal Protection**: Rejects any installed path containing `..` or root components escaping the installation root.
  - **Symlink Escape Jail**: Symlinks are strictly validated; internal relative symlinks (e.g. `libz.so -> libz.so.1`) are permitted, while symlinks targeting outside the staging root trigger `InstallError::SecurityViolation`.
  - **Filesystem Object Sanitization**: Rejects FIFOs, sockets, character devices, and block devices.
  - **Scoped Deletion**: Recursive deletion is strictly restricted to Crux-managed staging and backup directories; never mutates external or parent paths.
  - **Execution Disclosure**: Package CMake install rules execute package-controlled code. This execution is out-of-source but not sandboxed.
- **Installation Caching & Idempotency**:
  - Fast-path cache validation: checks `installed.toml` matching package name, version, build fingerprint, and profile.
  - Defense-in-depth: verifies every tracked file in inventory exists on disk with exact matching byte size.
  - Corrupted metadata or missing installed files immediately trigger cache invalidation and clean reinstallation.
  - Normal execution: returns `reused_install: true` in ~0.01s without invoking CMake.
  - Forced execution (`--force`): bypasses cache, runs fresh staged install, and safely replaces existing installation.
- **Dependency-Aware Installation & Locked Behavior**:
  - `crux install <package>`: Resolves transitive dependency closure and installs dependencies in topological order prior to the requested package.
  - `crux install`: Installs all project dependencies declared in `crux.lock` in strict topological order.
  - `--locked` flag: Verifies `crux.lock` is present and synchronized with `crux.toml`; fails if missing or out of sync; never creates, updates, or modifies `crux.lock`.

### K. CMake Consumer Integration Subsystem (`src/integrate.rs`)

Crux provides first-class consumer integration for CMake projects via `crux integrate cmake`. It generates a self-locating, deterministic CMake integration script (`.crux/cmake/CruxDependencies.cmake`) that makes Crux-installed packages discoverable through standard CMake package discovery (`find_package`) without requiring consumers to configure library paths or compiler include flags manually.

- **Self-Locating & Relocatable Path Resolution**:
  - The generated script derives the project root dynamically at CMake configure time:
    ```cmake
    get_filename_component(_CRUX_PROJECT_DIR "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)
    ```
  - Package installation prefixes are resolved relative to `_CRUX_PROJECT_DIR` (e.g. `${_CRUX_PROJECT_DIR}/.crux/installed/<pkg>/<ver>/<fingerprint>`).
  - No machine-specific or environment-dependent absolute paths are written to disk. Moving, renaming, or copying the project to another path or machine requires zero file regeneration.
- **Search Path Integration & System-Library Isolation**:
  - **`CMAKE_PREFIX_PATH`**: Crux package prefixes are prepended to `CMAKE_PREFIX_PATH` in deterministic alphabetical order. Prepending ensures CMake's `find_package` finds Crux-managed headers, libraries, and package config files before unversioned host or Nix store libraries, preventing accidental host symbol leakage.
  - **`CMAKE_MODULE_PATH`**: If an installed package provides CMake modules (`share/cmake/Modules` or `cmake/`), the directory is prepended to `CMAKE_MODULE_PATH`.
  - **Preservation & Deduplication**: Pre-existing consumer-specified search paths are preserved at the end of the search list, and duplicate entries are eliminated.
  - **Inclusion Guard**: An idempotent guard (`if(DEFINED _CRUX_DEPENDENCIES_INCLUDED) return() endif()`) allows safe inclusion from root and subproject `CMakeLists.txt` files.
- **Upstream Package Discovery Compatibility**:
  - **CMake Config Packages**: Packages providing CMake config files (e.g. `fmt` -> `fmt-config.cmake`, `Catch2` -> `Catch2Config.cmake`) are discovered via `find_package(<name> CONFIG REQUIRED)` and expose their authentic upstream imported targets (`fmt::fmt`, `Catch2::Catch2`, `Catch2::Catch2WithMain`).
  - **Built-in Find Modules**: Packages without custom config packages (e.g. `zlib` providing only headers and shared/static libraries) are discoverable via CMake's built-in modules (`find_package(ZLIB REQUIRED)`) and export standard targets (`ZLIB::ZLIB`).
  - **Zero Fabricated Targets**: Crux never invents fake imported targets; it relies entirely on authentic upstream exports and official CMake discovery modules.
- **Installation Metadata & Security Validation**:
  - Before integration, Crux strictly validates the `installed.toml` metadata and disk state for each target dependency:
    - Verifies format version (`CURRENT_INSTALL_METADATA_VERSION = 1`).
    - Validates package identity (name and SemVer version).
    - Verifies build fingerprint digest matches the directory name.
    - Verifies directory is strictly confined within `.crux/installed/`.
    - Checks `success == true` and non-empty inventory.
    - Verifies every indexed file exists on disk with matching byte size.
    - Path traversal prevention: rejects relative paths containing `..`, root, or prefix components.
    - Symlink jail enforcement: validates that all symlinks resolve strictly within the package installation prefix; symlinks escaping the jail are rejected.
- **Lockfile & Transitive Dependency Integration**:
  - `crux integrate cmake`: Integrates all locked dependencies declared in `crux.lock`.
  - `crux integrate cmake <package>`: Discovers and integrates the specified package and all of its transitive dependencies via BFS lockfile traversal.
  - Missing installation error: If a required package is locked but not installed, Crux returns a clear diagnostic directing the developer to run `crux install`.
  - `--locked` mode: Strictly validates that `crux.lock` is synchronized with `crux.toml`; never creates or mutates lockfile state.
- **Atomic File Generation & CMake Syntax Safety**:
  - Generates `.crux/cmake/CruxDependencies.cmake` atomically via temporary staging files and atomic directory renames.
  - Escapes CMake string values (spaces, quotes, semicolons, dollar signs, backslashes); rejects forbidden newlines or null control characters.
  - Diagnostic error emission: If an integrated package directory is deleted post-generation, CMake configuration immediately fails with a helpful error directing the developer to `crux install`.

