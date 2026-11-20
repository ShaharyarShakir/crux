# Crux

**Crux** is a source-first, deterministic C/C++ package manager designed to bring modern dependency management, reproducible builds, and first-class semantic versioning to the C and C++ ecosystems.

> **Status**: `v0.1.0` (Pre-Release / Evaluation). Crux is actively tested on Linux (x86_64).

---

## Documentation Index

- [Documentation Index](docs/README.md) — Navigate all guides, references, release material, and engineering reports.
- [Quickstart Guide (5 Minutes)](docs/guides/quickstart.md) — Create, build, and run your first C or C++ application with Crux.
- [Official Package Catalog](docs/guides/official-package-catalog.md) — 50 curated packages with source URLs, exact versions, and verification status.
- [Package Ecosystem Expansion Report](docs/reports/package-ecosystem-expansion-report.md) — Package-by-package evidence, reference apps, and honest platform scope.
- [Installation Guide](docs/guides/installation.md) — Nix Flake, Cargo build, and prebuilt binaries.
- [CLI Reference](docs/reference/cli-reference.md) — Full reference for all commands, flags, environment variables, and exit codes.
- [Security Model & Threat Assessment](docs/reference/security-model.md) — Trust boundaries, process privileges, and archive protections.
- [Architecture Specification](docs/reference/architecture.md) — Modular design, subsystem boundaries, and data flow.
- [Release Readiness Audit](docs/reports/release-readiness-audit.md) — Quality baseline, audit findings, and release criteria.
- [Maintainer Release Checklist](docs/releases/release-checklist.md) — Pre-release verification gates for maintainers.
- [Project Roadmap](ROADMAP.md) — Implemented milestones and upcoming features.

---

## Key Features & Status

| Feature | Status | Description |
|---|---|---|
| **Source-First Acquisition** | Ready | Streaming HTTPS download with SHA-256 integrity verification. |
| **Strict SemVer 2.0.0** | Ready | Caret (`^`), tilde (`~`), wildcard (`*`), and comparison ranges (`>=`, `<`). |
| **C/C++ ODR Safety** | Ready | Single-version-per-package resolution prevents duplicate symbol collisions. |
| **Deterministic Lockfiles** | Ready | Canonical, versioned `crux.lock` format (TOML) ensuring byte-for-byte reproducibility. |
| **Dependency Management** | Ready | `crux add` / `crux remove` update the manifest and lockfile transactionally, including transitives. |
| **Git Package Registries** | Ready | Clones and queries Git-backed package definitions and metadata; the official registry is configured automatically with 50 curated packages. |
| **CMake Build Engine** | Ready | Isolated out-of-source CMake builds with deterministic fingerprinting. Non-CMake registry entries are explicitly rejected as catalog-only. |
| **Application Builds** | Ready | `crux build` and `crux run` automatically install, link, and execute local C/C++ applications with their dependencies. |
| **Staged Package Installation** | Ready | Safe staging directory, symlink escape detection, and atomic publication. |
| **CMake Consumer Integration** | Ready | Generates relocatable `.crux/cmake/CruxDependencies.cmake` for `find_package`. |
| **Safe Extraction** | Ready | Zip Slip, decompression bomb, and symlink traversal defenses. |

---

## Installation & Setup

### Prerequisites

Crux requires Rust 1.85+ (Edition 2024). A reproducible [Nix flake](flake.nix) is provided with all necessary build tools (`rustc`, `cargo`, `rustfmt`, `clippy`, `gcc`, `cmake`, `pkg-config`).

### Using Nix Flake (Recommended)

```bash
# Enter development shell
nix develop

# Build the Crux binary
cargo build --release

# Run Crux
./target/release/crux --help
```

### Standard Cargo Build

```bash
cargo build --release
```

---

## CLI Usage

### 1. Initialize a Project or Package (`crux init`)

Initialize a standardized C, C++, or Other project convention with `crux.toml` and initialized `crux.lock`:

```bash
# Initialize a C++ project (default standard c++20)
crux init my-app --language cpp

# Initialize a C project (default standard c17)
crux init my-c-lib --language c

# Initialize an Other project (clean conventions without C/C++ compiler assumptions)
crux init my-tool --language other

# Override language standard
crux init my-app --language c --standard c11

# Interactive prompt (asks for C, C++, or Other when stdin is a terminal)
crux init my-project
```

Standard Crux Project Conventions (Minimal Default Structure):
```text
my-app/
├── crux.toml            # Project configuration & dependencies
├── crux.lock            # Exact resolved dependency versions
└── src/                 # Application sources
    └── main.cpp         # Working starter entrypoint
```

> **Start small. Let projects grow naturally**: Crux generates only the essentials upon initialization. Optional directories (`src/modules/`, `include/`, `tests/`, `.crux/`) can be added by users as needed or are created on demand by Crux. Crux recursively discovers application source files under `src/` only.

> **Crux-Managed CMake (Option A)**: For ordinary Crux-managed projects, Crux is the user-facing project and build manager; CMake is an internal build backend. Users configure projects via `crux.toml` and do **not** need to manually maintain a root-level `CMakeLists.txt`. Internal CMake files are generated automatically under `.crux/build/generated/`.

Generated `crux.toml`:
```toml
[project]
name = "my-app"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
```

### 2. Add and Remove Dependencies (`crux add` / `crux remove`)

Manage direct dependencies without hand-editing `crux.toml` or `crux.lock`:

```bash
# Add a dependency (any version; resolves to the highest available)
crux add fmt

# Add with an explicit SemVer constraint
crux add fmt@^11.0
crux add zlib@">=1.2"

# Remove a direct dependency
crux remove fmt
```

What happens:

- `crux add <package>` sets the constraint to `"*"` (any version) when none is supplied.
- Crux resolves the complete graph — including transitive dependencies — updates **only** the `[dependencies]` table, and writes `crux.toml` plus `crux.lock` transactionally.
- The new dependency is installed for the Debug profile immediately, so `crux build` / `crux run` work without another command.
- `crux remove <package>` removes only that direct dependency, re-resolves the remaining graph, preserves shared transitive dependencies in `crux.lock`, and leaves cached/installed files untouched.
- Comments, ordering, and unrelated formatting in `crux.toml` are preserved.
- Duplicate `add` and unknown `remove` operations fail with actionable diagnostics without writing files.

> **No registry setup required**: On first use, Crux automatically configures the official Crux registry that ships in the repository's `registry/` directory. Custom registries configured with `crux registry add` are never overwritten. See [`crux registry`](#6-registry-management-crux-registry).

### 3. Validate Project Manifest & Toolchain (`crux check`)

Validates the syntactic and semantic correctness of `crux.toml`, verifies language and standard settings, checks compiler availability, and validates dependency version requirements:

```bash
crux check
```

Output:
```text
Checking Crux project...
Manifest: crux.toml
Package: my-app
Version: 0.1.0
Language: C++
Standard: c++20

Toolchain:
  Compiler: GCC
  Version: 15.3.0
  Target: x86_64-unknown-linux-gnu
  Status: Available

Direct dependencies: 2
  fmt      ^11.1
  zlib     ^1.3

Status: Valid
```

### 4. Preview Dependency Resolution (`crux resolve`)

Resolves direct and transitive dependencies against the package catalog without modifying disk state:

```bash
crux resolve
```

Output:
```text
Resolving dependencies for 'my-library' (0.1.0)...
Using registry 'default'

Resolved 2 dependencies:
  fmt 11.1.1 (registry+default)
  zlib 1.3.1 (registry+default)

Status: Success
```

### 5. Lockfile Management (`crux lock`)

Crux provides deterministic lockfile generation, synchronization, and integrity verification:

```bash
# Generate or synchronize crux.lock with crux.toml
# Reuses existing locked versions whenever they remain compatible with manifest constraints
crux lock

# Upgrade all dependencies to their highest compatible versions
# (ignores existing locked version preferences)
crux lock --update

# Check lockfile validity and synchronization without modifying any files
# Exits with an error if crux.lock is missing, corrupted, or out of sync with crux.toml
crux lock --check
```

Output for `crux lock`:
```text
Lockfile synchronized: 2 packages locked
  fmt 11.1.1
  zlib 1.3.1
```

Output for `crux lock --check`:
```text
Lockfile: /path/to/crux.lock
Packages: 2
Status: Valid (synchronized)
```

### 6. Registry Management (`crux registry`)

Crux supports Git-backed package registries. **Fresh projects use the official Crux registry automatically** — no setup step is required. The official registry URL is `https://github.com/ShaharyarShakir/crux.git`, and its registry root is the `registry/` subdirectory of that repository. Crux reuses an existing checkout (including the development tree's bundled `registry/` directory) and clones into the user cache only when needed, so cached and offline runs work after the first successful bootstrap.

Manage configured registries using:

```bash
# Add and clone a new Git package registry (local path or remote URL)
crux registry add default /path/to/crux-registry

# List all configured package registries and their status
crux registry list

# Update a configured registry from upstream via Git fast-forward pull
crux registry update default

# Print registry subcommand help
crux registry --help
```

Output for `crux registry list`:
```text
Configured registries (1):
  default
    URL:      /path/to/crux-registry
    Path:     /home/user/.crux/registries/default
    Packages: 3
    Versions: 6
    Status:   Valid
```

When the official registry is bootstrapped instead, `crux registry list` also reports the subdirectory used to locate `registry.toml`:

```text
Configured registries (1):
  default
    URL:      https://github.com/ShaharyarShakir/crux.git
    Path:     /home/user/.crux/registries/official
    Subdir:   registry
    Packages: 3
    Versions: 6
    Status:   Valid
```

`CRUX_OFFICIAL_REGISTRY_URL` can point at a mirror for air-gapped environments; `CRUX_REGISTRIES_DIR`, `CRUX_HOME`, and `XDG_CACHE_HOME` control where the official checkout is cached.

### 7. Source Acquisition & Verification (`crux fetch`)

Crux securely downloads, cryptographically verifies, caches, and extracts package source archives:

```bash
# Fetch and extract a specific package (picks highest compatible stable version)
crux fetch fmt

# Fetch an exact version
crux fetch fmt@11.1.1

# Fetch all project dependencies declared in crux.lock
crux fetch

# Force re-download and re-extraction, ignoring cached files
crux fetch fmt --force

# Print fetch subcommand help
crux fetch --help
```

Output for `crux fetch fmt@11.1.1`:
```text
Fetching fmt 11.1.1 from registry 'default'...

Successfully acquired fmt 11.1.1:
  Archive: /home/user/.cache/crux/archives/sha256/a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45
  Source:  /home/user/.cache/crux/sources/sha256/a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45
  SHA-256: sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45
```

### 8. Application & Package Build Engine (`crux build`)

Crux builds complete local C and C++ applications directly from the sources under `src/`. CMake is an internal backend: Crux generates and maintains the build configuration under `.crux/build/generated/`, and ordinary projects never need a root-level `CMakeLists.txt`.

```bash
# Build the local application (Debug by default)
crux build

# Build with an explicit configuration
crux build --debug
crux build --release

# Show detected toolchain paths and full CMake/compiler output
crux build --verbose

# Force a clean rebuild of the selected configuration
crux build --force

# Build a registry package instead of the local application
crux build fmt
crux build fmt@11.1.1 --profile debug

# Build all locked package dependencies (legacy package projects)
crux build

# Print build subcommand help
crux build --help
```

Output for a local application build:
```text
Crux Application Build

Project:       demo
Language:      C++
Standard:      C++20
Configuration: Debug

Toolchain:
  Compiler:    GCC 15.3.0
  CMake:       CMake 4.1.6
  Generator:   Ninja

Source files:  3
Dependencies:  1

Generating build configuration...
Configuring...
Building...

Build successful!

Executable:
  .crux/build/debug/demo

Finished in 0.40s
```

Application build layout:
```text
demo/
├── crux.toml
├── crux.lock
├── src/
│   ├── main.cpp
│   └── modules/
│       └── calculator.cpp
└── .crux/
    └── build/
        ├── generated/
        │   └── CMakeLists.txt   # generated by Crux; do not edit
        ├── debug/
        │   └── demo             # Debug executable
        ├── release/
        │   └── demo             # Release executable
        └── logs/
            ├── configure-debug.log
            └── build-debug.log
```

**Dispatch rule**:
- `crux build <package>` always builds that package from the configured registry.
- `crux build` with a `[project]` manifest builds the local application.
- `crux build` with a legacy `[package]` manifest (or only a `crux.lock`) retains v0.1.0 behavior and builds locked dependencies.

**Application dependencies**: For `[project]` builds, Crux checks every declared dependency (and its transitives) against `.crux/installed/` before configuring CMake. Missing or incomplete installations are installed automatically for the selected profile. Validated include directories, library search paths, static/shared libraries, and runtime search paths are injected into the generated `.crux/build/generated/CMakeLists.txt`; no root `CMakeLists.txt` is created. Static libraries are preferred when a package installs both variants, and dependencies are linked dependents-first so single-pass linkers resolve symbols correctly. `crux build --release` and `crux run --release` use Release-profile installations and fingerprints.

Output for `crux build fmt@11.1.1`:
```text
Acquiring source for fmt 11.1.1...
Building fmt 11.1.1 [release] with CMake...

Successfully built fmt 11.1.1 (compiled successfully):
  Profile:     release
  Fingerprint: sha256:7b56f8f537dbf2597b830d1737e90dc88ba278fce0d62c161d9a5b6db76a30c5
  Build Dir:   /path/to/project/.crux/builds/fmt/11.1.1/7b56f8f5.../build
  Logs:        /path/to/project/.crux/builds/fmt/11.1.1/7b56f8f5.../logs
  Artifacts (2):
    - libfmt.a (static library, 1245180 bytes)
    - fmt.pc (other artifact, 612 bytes)
```

### 9. Run (`crux run`)

Build and run the current C/C++ application from the project root:

```bash
# Build (Debug, default) and run
crux run

# Build and run Release
crux run --release

# Pass arguments to the application (after --)
crux run -- --name "Alice" --count 5

# Verbose CMake/build output
crux run --verbose
```

Behavior:
- Reuses the M19 application build engine; compilation failures prevent execution.
- Before building, `crux run` ensures declared dependencies are locked and installed for the selected profile, then launches the executable with dependency runtime directories available.
- `crux run` defaults to Debug (`--release` for Release). Debug and Release executables are isolated under `.crux/build/`.
- Application runs with working directory = project root (relative file paths resolve in the project root).
- Environment is inherited; stdin/stdout/stderr are inherited. For shared-library dependencies, Crux prepends the relevant installed library directories to the platform loader path.
- Crux exits with the application's exit status.

Ad-hoc source mode (no `crux.toml` or registry required):

```bash
# One C or C++ source file
crux run --bin hello.c
crux run --bin hello.cpp --release

# Multiple translation units, headers, and include directories
crux run --sources main.cpp,math.cpp --sources util.cpp -Iinclude
crux run --sources main.c --sources util.c --include-dir ../shared
```

Ad-hoc inputs are copied into an isolated `.crux/run/<name>-<hash>/` workspace and compiled with the same CMake engine. Source extensions are `.c` (C) and `.cpp`/`.cc`/`.cxx` (C++); headers listed in `--sources` are copied but not compiled. A pure C consumer of verified zlib is available at `examples/m22/c-zlib-roundtrip`.


### 10. Package Installation Engine (`crux install`)

Crux installs compiled packages into project-local staging and publication trees using CMake's install mechanism (`cmake --install`), isolated staging directories, symlink traversal confinement checks, atomic directory swaps, and persistent file inventory tracking:

```bash
# Install a specific package and its dependencies (Release profile by default)
crux install fmt

# Install an exact version
crux install fmt@11.1.1

# Install with Debug profile
crux install fmt@11.1.1 --profile debug

# Force re-installation, replacing existing installed artifacts
crux install fmt@11.1.1 --force

# Install all project dependencies declared in crux.lock
crux install

# Strictly require crux.lock synchronization without modifying lockfile
crux install --locked

# Print install subcommand help
crux install --help
```

Installed Directory Structure:
```text
.crux/installed/<package>/<version>/<fingerprint>/
├── include/
│   └── fmt/
│       ├── core.h
│       ├── format.h
│       └── ...
├── lib64/ (or lib/)
│   └── libfmt.a
├── share/
│   └── pkgconfig/
│       └── fmt.pc
└── installed.toml
```

Output for `crux install fmt@11.1.1`:
```text
Installing fmt 11.1.1 [release]...
Installed fmt 11.1.1 to /path/to/project/.crux/installed/fmt/11.1.1/7b56f8f537dbf259...
  Files: 20
  Size:  821.1 kB
```

### 11. CMake Consumer Integration (`crux integrate cmake`)

Crux generates a deterministic CMake integration file that makes installed packages discoverable through standard CMake mechanisms without manually writing library search paths or include directories:

```bash
# Generate CMake integration file for all project dependencies
crux integrate cmake

# Integrate a specific package and its transitive dependencies
crux integrate cmake fmt

# Integrate using Debug profile
crux integrate cmake --profile debug

# Require crux.lock to be present and strictly synchronized
crux integrate cmake --locked

# Print integrate subcommand help
crux integrate cmake --help
```

Generated Integration File:
```text
.crux/
├── installed/
│   ├── Catch2/3.8.0/d96e4c70.../
│   ├── fmt/11.1.1/7b56f8f5.../
│   └── zlib/1.3.1/32a1e05a.../
└── cmake/
    └── CruxDependencies.cmake
```

Consumer `CMakeLists.txt` Integration Example:
```cmake
cmake_minimum_required(VERSION 3.20)
project(my_app LANGUAGES C CXX)

# Include Crux-managed dependencies
include("${CMAKE_CURRENT_SOURCE_DIR}/.crux/cmake/CruxDependencies.cmake")

# Standard CMake package discovery
find_package(fmt CONFIG REQUIRED)           # Targets: fmt::fmt
find_package(Catch2 3 CONFIG REQUIRED)      # Targets: Catch2::Catch2, Catch2::Catch2WithMain
find_package(ZLIB REQUIRED)                 # Targets: ZLIB::ZLIB

add_executable(my_app main.cpp)
target_link_libraries(my_app PRIVATE fmt::fmt ZLIB::ZLIB)
```

Key Integration Features:
- **Project-Relocatable Paths**: Uses `CMAKE_CURRENT_LIST_DIR` to dynamically resolve package prefixes; moving or copying the project requires no file regeneration.
- **System-Library Isolation**: Prepends Crux installation prefixes to `CMAKE_PREFIX_PATH` and `CMAKE_MODULE_PATH`, guaranteeing that consumer builds link against Crux-managed dependencies rather than unversioned system libraries.
- **Preserves Consumer Paths**: Retains pre-existing search paths and prevents duplicate entries.
- **Idempotent Inclusions**: Built-in inclusion guards allow safe multiple `include()` calls from subdirectories.
- **Safe Path Handling**: Fully quotes paths and safely escapes special characters, backslashes, and spaces.


### 12. Compiler & Toolchain Inspection (`crux toolchain`)

Detects installed C and C++ compilers on the host system, identifies compiler family and version, inspects target platform triples, and tests compiler capabilities against supported language standards:

```bash
# Inspect all detected toolchains
crux toolchain

# Filter by project language
crux toolchain --language c
crux toolchain --language cpp
```

Output:
```text
Crux Toolchain Information

Platform:
  OS: linux
  Architecture: x86_64

C Toolchain:
  Compiler: GCC
  Version: 15.3.0
  Target: x86_64-unknown-linux-gnu
  Executable: /usr/bin/gcc
  Standards: C17 (supported), C23 (supported)

C++ Toolchain:
  Compiler: GCC
  Version: 15.3.0
  Target: x86_64-unknown-linux-gnu
  Executable: /usr/bin/g++
  Standards: C++20 (supported), C++23 (supported)
```

Environment Overrides:
- Set `CC` to specify a custom C compiler executable (e.g. `CC=clang crux toolchain --language c`).
- Set `CXX` to specify a custom C++ compiler executable (e.g. `CXX=clang++ crux toolchain --language cpp`).


---

## Manifest Specification (`crux.toml`)

Dependencies are declared under `[dependencies]` with standard SemVer requirements:

```toml
[package]
name = "cpp-network-service"
version = "1.0.0"
description = "High-performance networking library"

[dependencies]
fmt = "^11.1"
spdlog = "^1.17"
catch2 = ">=3.8.0, <4.0.0"
```

### Supported Requirement Expressions

| Requirement | Meaning | Example Match |
|---|---|---|
| `^1.2.3` | Compatible update within major version | `1.2.3`, `1.3.0`, `1.9.9` (excludes `2.0.0`) |
| `~1.2.3` | Patch updates only within minor version | `1.2.3`, `1.2.9` (excludes `1.3.0`) |
| `=1.2.3` | Exact version match | `1.2.3` |
| `>=1.0, <1.4` | Comparator range expression | `1.0.0`, `1.3.1` (excludes `1.4.0`) |
| `*` | Any stable version | Highest available stable version |

---

## Lockfile Specification (`crux.lock`)

Crux uses a deterministic, canonical TOML lockfile format:

```toml
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"
dependencies = []

[[package]]
name = "spdlog"
version = "1.17.0"
source = "registry+default"
checksum = "sha256:d8862955c6d74e5846b3f580b1605d2428b11d97a410d86e2fb13e857cd3a744"
dependencies = [
    "fmt 11.1.1",
]

[[package]]
name = "zlib"
version = "1.3.1"
source = "registry+default"
checksum = "sha256:9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"
dependencies = []
```

- Packages are sorted canonically by name then version.
- Dependencies are sorted lexicographically by package ID (`"<name> <version>"`).
- Serialization is 100% deterministic with no environment-dependent data or timestamps.

---

## Package Registry Specification

Crux uses a versioned, human-readable Git repository format for package registries:

```text
crux-registry/
├── registry.toml
└── packages/
    ├── catch2/
    │   ├── 3.7.1/
    │   │   └── package.toml
    │   └── 3.8.0/
    │       └── package.toml
    ├── fmt/
    │   ├── 11.0.2/
    │   │   └── package.toml
    │   └── 11.1.1/
    │       └── package.toml
    └── zlib/
        ├── 1.3.0/
        │   └── package.toml
        └── 1.3.1/
            └── package.toml
```

### Registry Manifest (`registry.toml`)

```toml
format_version = 1
name = "default"
description = "Official Crux package registry"
```

### Package Definition (`package.toml`)

```toml
[package]
name = "fmt"
version = "11.1.1"
description = "A modern formatting library"
license = "MIT"
homepage = "https://fmt.dev"
repository = "https://github.com/fmtlib/fmt"

[source]
type = "archive"
url = "https://github.com/fmtlib/fmt/releases/download/11.1.1/fmt-11.1.1.zip"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"

[build]
system = "cmake"

[dependencies]
# e.g. zlib = "^1.3"
```

### Catalog and Registry Subsystem Distinctions

Crux strictly separates registry-backed production resolution from test-only fixtures:

1. **Configured Git Registries (Production)**: `crux lock`, `crux resolve`, `crux add`, `crux remove`, and other catalog-backed commands resolve only through real registries. Crux selects a configured primary registry, `CRUX_REGISTRY_PATH`, a local `registry/registry.toml` checkout, or bootstraps the official Crux registry for a fresh project. There is no in-memory production fallback.
2. **Official Registry Bootstrap**: Fresh projects automatically configure and reuse the official Crux registry. In release environments Crux clones `https://github.com/ShaharyarShakir/crux.git` (subdir `registry/`) into managed cache storage; in development/source builds it may reuse the bundled in-repo registry. `CRUX_OFFLINE=1` prevents network bootstrap and produces an actionable error when no local registry is available.
3. **Custom and Local Git Registries**: You can add and use custom registries (e.g. `crux registry add default /path/to/repo` or a Git URL). Custom registry configuration is preserved and honored by production resolution.
4. **Source Acquisition Subsystem (Milestone 10)**: Package definitions declare source archive URLs and SHA-256 checksums. Crux securely downloads, cryptographically verifies, caches, and extracts archives into content-addressed directories with defense-in-depth safety checks.
5. **In-Memory Development Fixture (Tests Only)**: `crux::catalog::default_dev_catalog` / `InMemoryCatalog` remain available as deterministic Rust test fixtures. CLI integration tests use isolated Git registries, not production fallback behavior.

---

## Project Architecture

```text
src/
├── core/           # Foundational utilities (paths, atomic filesystem operations)
├── package/        # Package domain models (PackageName, PackageId, PackageMetadata, Dependency)
├── project/        # Manifest model (crux.toml), project init, and check verification
├── resolution/     # Catalog abstraction, backtracking Resolver, and Lockfile model
├── registry/       # Git-backed package registry index and CruxConfig management
├── source/         # Source acquisition, streaming download, SHA-256 verification, and archive cache
├── build/          # CMake build engine, out-of-source compilation, fingerprints, and diagnostics
├── install/        # Project-local staging, file inventory, and atomic package installation
├── integrate/      # CMake consumer integration and CruxDependencies.cmake generation
├── cli/            # CLI argument parsing (args.rs) and subcommand handlers (commands/)
├── lib.rs          # Library root and public API exports
└── main.rs         # CLI binary entry point
```

---

## Testing & Quality Assurance

Crux enforces strict code quality standards:

```bash
# Run all unit and integration tests
nix develop --command cargo test

# Verify code formatting
nix develop --command cargo fmt --check

# Run Clippy with warnings denied
nix develop --command cargo clippy --all-targets -- -D warnings

# Compiler sanity check
nix develop --command cargo check
```

---

## Security & Process Privileges

Crux prioritizes transparent execution and defense-in-depth:
- **Subprocess Safety**: Crux invokes CMake and external tools using discrete argument vectors via `std::process::Command`. It never invokes shell interpreters (`sh -c` or `cmd.exe /C`), preventing shell injection.
- **Process Privileges**: Crux currently runs external build scripts and CMake with ambient user privileges. No OS-level containerization or namespace sandbox is active in v0.1.0. See [docs/reference/security-model.md](docs/reference/security-model.md) for complete details.
- **Archive Extraction Defenses**: Zip Slip path traversal defenses, symlink escape detection, and decompression bomb ceilings are strictly enforced.

---

## Supported Platforms

- **Linux (x86_64)**: Fully supported and validated in CI.
- **Linux (aarch64)**: Planned.
- **macOS / Windows**: Planned for post-v0.1.0 releases.

---

## License

Licensed under the MIT License. See [LICENSE](LICENSE) for details.
