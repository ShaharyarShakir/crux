# Crux M21 — Dependency Management & Application Integration Completion Report

**Release:** v0.2.0  
**Milestone:** M21  
**Status:** Complete & Verified  
**Date:** November 18, 2026  
**Auditor / Engineer:** Autonomous Senior Rust Engineer  
**Repository:** `/home/shaharyar/01__git_repos/products/crux`  

---

## 1. Implementation Summary

Milestone 21 delivers Crux's first complete, end-to-end dependency-management and application integration workflow. It connects package manifest declarations (`crux.toml`), deterministic resolution (`Resolver`), cryptographic lockfile synchronization (`crux.lock`), source acquisition, sandboxed compilation, staged package installation, and internal CMake application target linking.

Developers can initialize a C or C++ application, declare dependencies from the official Git registry, compile, and execute with zero manual CMake configuration:

```bash
crux init demo --language cpp
cd demo
crux add fmt
crux build
crux run
```

All existing Crux commands (`init`, `check`, `lock`, `resolve`, `registry`, `fetch`, `build`, `install`, `integrate cmake`, `run`, `toolchain`) remain completely backward compatible and operational.

---

## 2. Files Created and Modified

### Source Files Created
- `src/cli/commands/add.rs`: Implementation of `crux add <package>[@<constraint>]`.
- `src/cli/commands/remove.rs`: Implementation of `crux remove <package>`.
- `src/cli/commands/dependency_edit.rs`: Shared transactional manifest/lockfile commit and rollback helper (`commit_manifest_and_lockfile`).
- `src/build/dependency.rs`: Application dependency link planner (`plan_app_dependencies`, `AppDependencyPlan`).
- `tests/dependency_management.rs`: Dedicated integration test suite covering dependency management and application linking.
- `docs/M21_DEPENDENCY_MANAGEMENT_REPORT.md`: This completion report.

### Source Files Modified
- `src/cli/args.rs`: Added `AddArgs`, `RemoveArgs`, `Action::Add`, `Action::AddHelp`, `Action::Remove`, `Action::RemoveHelp`, parser logic, and help strings.
- `src/cli/mod.rs`: Added dispatch arms for `add` and `remove`, `load_catalog_for_dependency_edits`, and unit tests for argument parsing.
- `src/cli/commands/mod.rs`: Re-exported `add`, `remove`, and `dependency_edit` modules.
- `src/cli/commands/build.rs`: Added `ensure_app_dependencies` to automatically synchronize and install missing locked dependencies before application builds.
- `src/cli/commands/run.rs`: Linked application execution to runtime directories and dependency environment paths (`LD_LIBRARY_PATH`, `DYLD_LIBRARY_PATH`, `PATH`).
- `src/build/app.rs`: Integrated dependency link planning into `AppBuildModel::prepare_with_cmake`.
- `src/build/generator.rs`: Added CMake generator rules emitting `target_include_directories`, `target_link_directories`, `target_link_libraries`, and `BUILD_RPATH` for application targets.
- `src/build/mod.rs`: Re-exported `AppDependency`, `AppDependencyPlan`, and `plan_app_dependencies`.
- `src/project/manifest.rs`: Added `upsert_dependency_in_toml` and `remove_dependency_from_toml` preserving TOML comments, ordering, and unrelated sections.
- `src/registry/config.rs`: Added official registry auto-configuration (`ensure_official_registry`), subdirectory support (`subdir = "registry"`), in-tree bundled registry fallback, and local cache reuse.
- `src/registry/git.rs`: Added `open_with_subdir` and path validation for subdirectory registries.
- `src/lib.rs`: Exposed public M21 APIs for registry configuration, dependency planning, and manifest helpers.

### Documentation Modified
- `README.md`: Documented `crux add`, `crux remove`, transactional semantics, and the official registry.
- `docs/CLI_REFERENCE.md`: Added reference entries and argument details for `crux add` and `crux remove`.
- `docs/QUICKSTART.md`: Added full dependency management and application workflow walkthrough.
- `docs/ARCHITECTURE.md`: Documented dependency link planning, CMake property injection, and official registry auto-bootstrap.
- `crux-site/src/content/docs/reference/cli.md`: Added website CLI reference for `crux add` and `crux remove`.
- `crux-site/src/content/docs/guides/getting-started.md`: Added dependency management guide.
- `crux-site/src/content/docs/registry/packages.md`: Added seed package usage guidance with `crux add`.

---

## 3. `crux add` Behavior

### Command Syntax
```bash
crux add <package>[@<constraint>]
```

### Execution Flow
1. **Manifest Validation**: Loads and parses `crux.toml`. If the manifest is missing, returns an actionable error directing the user to `crux init`.
2. **Spec Parsing & Defaults**: Parses `<package>[@<constraint>]`. If no constraint is provided, defaults to `"*"` (wildcard), matching the existing resolver convention to resolve the highest available compatible version.
3. **Duplicate Detection**: Verifies that the package is not already declared in `crux.toml`. Duplicate declarations are rejected with an actionable error (`dependency 'x' is already declared in crux.toml; use 'crux remove x' first...`).
4. **Graph Resolution**: Evaluates the package against the active catalog using `Resolver::resolve_with_preferences`. Existing locked package versions are preserved as preferences to avoid unnecessary churn.
5. **In-Memory Lockfile Construction**: Constructs the new `Lockfile` in memory using `graph.to_lockfile_with_existing` before modifying any files.
6. **Manifest Updating**: Updates only the `[dependencies]` table of `crux.toml` via `upsert_dependency_in_toml`, preserving comments, table order, and unrelated configurations.
7. **Transactional Commitment**: Uses `commit_manifest_and_lockfile`:
   - Writes `crux.toml` atomically.
   - Writes `crux.lock` atomically.
   - If the lockfile write fails, `crux.toml` is automatically restored to its previous contents.
8. **Immediate Installation**: Calls `run_install` for the Debug profile, ensuring the new package is acquired, built, and installed into `.crux/installed/` immediately.

---

## 4. `crux remove` Behavior

### Command Syntax
```bash
crux remove <package>
```

### Execution Flow
1. **Direct Dependency Verification**: Checks that `<package>` is declared under `[dependencies]` in `crux.toml`. If absent, reports an error and lists all currently declared direct dependencies.
2. **Pre-Write Re-resolution**: Re-resolves the remaining dependency graph *before* making any filesystem modifications. If the remaining graph cannot resolve, disk files remain untouched.
3. **Transitive Retention**: Packages required transitively by other remaining direct dependencies are preserved in `crux.lock`.
4. **Manifest Updating**: Removes only the target key from `[dependencies]` using `remove_dependency_from_toml`.
5. **Transactional Commitment**: Atomically writes updated `crux.toml` and updated `crux.lock` with rollback on lockfile write failure.
6. **Cache Safety**: Installed files in `.crux/installed/` and global cache archives are intentionally preserved. Crux adheres to safe ownership boundaries and never removes files without explicit user intent (`crux install --force`).

---

## 5. Official Registry Source URL and Implementation

- **Canonical URL**: `https://github.com/ShaharyarShakir/crux.git` (verified from repository origin remote).
- **Subdirectory Architecture**: The official registry is embedded directly within the Crux repository at the `registry/` subdirectory:
  ```text
  crux/
  ├── registry/
  │   ├── registry.toml
  │   └── packages/
  │       ├── catch2/
  │       ├── fmt/
  │       └── zlib/
  ```
- **Subdirectory Support**: `GitRegistry::open_with_subdir` verifies that `registry.toml` exists within the designated subdirectory and enforces symlink jail boundaries within that root.
- **Auto-Configuration (`ensure_official_registry`)**:
  - Automatically invoked by `load_catalog_for_dependency_edits` when no registry is configured and `CRUX_REGISTRY_PATH` is unset.
  - Generates a registry entry named `"default"` pointing to the local cache directory with `subdir = "registry"`.
  - Development and test builds seamlessly detect and reuse the bundled in-tree registry via `bundled_official_registry_root`.

---

## 6. Registry Caching, Update, and Offline Behavior

- **Cache Location**: Cloned registries are cached in user-level storage at `~/.crux/registries/crux` (or `$CRUX_REGISTRIES_DIR/crux`).
- **Cache Reuse**: Subsequent commands check if `~/.crux/registries/crux/registry/registry.toml` exists and is a valid Git repository. If valid, Crux opens the cached checkout directly without any network operations.
- **Offline Operations**: If valid cached data exists, `crux add`, `crux remove`, `crux build`, and `crux run` execute completely offline.
- **Update Behavior**: `crux registry update default` performs a fast-forward Git pull (`pull --ff-only`) on the cached repository.
- **Preservation of User Config**: Existing custom registries and configuration in `~/.crux/config.toml` or `.crux/config.toml` are never overwritten or cleared.

---

## 7. Dependency Resolution and Installation Flow

```text
[crux add <package>]
        │
        ▼
   Load manifest & parse spec
        │
        ▼
   Resolve full graph (Resolver + catalog + preferences)
        │
        ▼
   Construct lockfile in-memory
        │
        ▼
   Write crux.toml & crux.lock (Atomic with rollback)
        │
        ▼
   Acquire source archive (SourceAcquirer + SHA-256 verify)
        │
        ▼
   Extract source (Zip Slip & Decompression Bomb defense)
        │
        ▼
   Out-of-source CMake build (CMakeBuildEngine)
        │
        ▼
   Staged installation (CMakeInstallEngine -> .crux/installed/)
```

In addition, `crux build` and `crux run` invoke `ensure_app_dependencies`, inspecting all locked packages against `validate_package_installation`. If any package is missing for the active profile (e.g. Debug vs Release), Crux automatically builds and installs the missing packages before compiling the application.

---

## 8. Application Build Integration Architecture

Crux generates internal CMake configuration under `.crux/build/generated/CMakeLists.txt` without requiring a user-authored `CMakeLists.txt` or manual `crux integrate cmake` invocation.

### Dependency Link Planning (`plan_app_dependencies`)
1. **Topological Order**: Traverses the locked graph depth-first so that dependents appear before dependencies on the linker line.
2. **Artifact Classification**: Reads `installed.toml` metadata and scans installed files:
   - Includes: directories ending in `include`.
   - Link Directories: directories ending in `lib` or `lib64`.
   - Libraries: static archives (`.a`, `.lib`) and shared libraries (`.so`, `.dylib`, `.dll`). Prefers static archives for self-contained binaries.
   - Runtime Directories: directories containing shared libraries for runtime loader paths.

### Generated CMake Structure
```cmake
# Generated by Crux for application target: demo
cmake_minimum_required(VERSION 3.20)
project(demo LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_executable(demo
    "/path/to/demo/src/main.cpp"
)

target_include_directories(demo PRIVATE
    "/path/to/demo/.crux/installed/fmt/11.1.1/<fp>/include"
)

target_link_directories(demo PRIVATE
    "/path/to/demo/.crux/installed/fmt/11.1.1/<fp>/lib"
)

target_link_libraries(demo PRIVATE
    "/path/to/demo/.crux/installed/fmt/11.1.1/<fp>/lib/libfmt.a"
)

set_target_properties(demo PROPERTIES
    BUILD_RPATH "/path/to/demo/.crux/installed/fmt/11.1.1/<fp>/lib"
)
```

---

## 9. CLI Examples

### Adding a Dependency
```bash
# Add with wildcard constraint (resolves to 11.1.1)
crux add fmt

# Add with caret constraint
crux add fmt@^11.0

# Add with comparison range
crux add zlib@">=1.2, <2.0"
```

### Removing a Dependency
```bash
crux remove fmt
```

### Building and Running
```bash
# Debug build and execution (default)
crux run

# Release build and execution (auto-installs release dependency if missing)
crux run --release

# Forwarding arguments to the application
crux run -- --verbose --input data.txt
```

---

## 10. Test Coverage and Exact Results

Automated test execution across all suites via `nix develop --command cargo test --all`:

| Test Suite | Total Tests | Passed | Failed |
|---|---|---|---|
| Unit tests (`src/lib.rs`) | 201 | 201 | 0 |
| `tests/app_build.rs` | 21 | 21 | 0 |
| `tests/build.rs` | 29 | 29 | 0 |
| `tests/check.rs` | 9 | 9 | 0 |
| `tests/init.rs` | 23 | 23 | 0 |
| `tests/install.rs` | 16 | 16 | 0 |
| `tests/integrate_cmake.rs` | 17 | 17 | 0 |
| `tests/lock.rs` | 8 | 8 | 0 |
| `tests/package_model.rs` | 14 | 14 | 0 |
| `tests/dependency_management.rs` | 7 | 7 | 0 |
| `tests/registry.rs` | 26 | 26 | 0 |
| `tests/resolve.rs` | 23 | 23 | 0 |
| `tests/run.rs` | 12 | 12 | 0 |
| `tests/source.rs` | 22 | 22 | 0 |
| `tests/toolchain.rs` | 7 | 7 | 0 |
| **Total** | **435** | **435** | **0** |

All tests pass deterministically without requiring external network access.

---

## 11. Quality-Gate Results

Executed inside the Nix development environment (`nix develop --command ...`):

1. **Code Formatting**:
   ```bash
   cargo fmt --all -- --check
   ```
   *Result*: **Passed (0 diffs)**.

2. **Clippy Static Analysis**:
   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   ```
   *Result*: **Passed (0 warnings, 0 errors)**.

3. **Compilation Check**:
   ```bash
   cargo check
   ```
   *Result*: **Passed (Finished in 0.16s)**.

4. **Test Suite**:
   ```bash
   cargo test --all
   ```
   *Result*: **Passed (435 passed, 0 failed)**.

5. **Release Build**:
   ```bash
   cargo build --release
   ```
   *Result*: **Passed (Binary generated at `target/release/crux`)**.

---

## 12. Manual End-to-End Results

Executed using `target/release/crux` in an isolated temporary directory:

```bash
# 1. Initialize project
crux init demo --language cpp
# Result: Created demo/crux.toml, demo/crux.lock, demo/src/main.cpp

# 2. Add dependency
crux add fmt
# Result: Resolved fmt 11.1.1, updated crux.toml and crux.lock, downloaded and installed fmt 11.1.1

# 3. Add fmt code to src/main.cpp
cat << 'EOF' > src/main.cpp
#include <fmt/core.h>

int main() {
    fmt::print("Hello from Crux M21!\n");
    return 0;
}
EOF

# 4. Build and Run (Debug)
crux run
# Output:
# Hello from Crux M21!

# 5. Build and Run (Release)
crux run --release
# Output:
# Installing 1 required dependency package(s) [release]...
# missing: fmt 11.1.1
# [installed] fmt 11.1.1
# Hello from Crux M21!

# 6. Remove dependency
crux remove fmt
# Result: Cleanly removed fmt from crux.toml and crux.lock
```

The acceptance workflow succeeded with zero errors.

---

## 13. Backward Compatibility Verification

- Existing commands (`crux init`, `crux check`, `crux lock`, `crux resolve`, `crux registry`, `crux fetch`, `crux build`, `crux install`, `crux integrate cmake`, `crux run`, `crux toolchain`) retain identical arguments, flags, and outputs.
- Projects without dependencies build and execute with zero regression.
- External CMake integration (`crux integrate cmake`) continues generating valid `CruxDependencies.cmake` files.
- Package builds (`crux build <package>`) remain distinct from application builds (`crux build`).

---

## 14. Known Limitations and Deferred Work

The following items are deferred to subsequent milestones in accordance with M21 scope boundaries:
1. **No External Network in Tests**: Automated tests use local Git repositories and in-tree seed packages.
2. **Single Application Executable Target**: Multi-target applications and application library targets remain deferred.
3. **No Automatic Uninstall on Disk**: `crux remove` updates the project manifest and lockfile but intentionally preserves cached/installed packages in `.crux/installed/` to prevent data loss.
4. **Standalone Registry Repository**: The official registry remains embedded in `registry/` within the main Crux repository.
