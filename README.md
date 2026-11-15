# Crux

**Crux** is a source-first, deterministic C/C++ package manager designed to bring modern dependency management, reproducible builds, and first-class semantic versioning to the C and C++ ecosystems.

> **Status**: `v0.1.0` (Pre-Release / Evaluation). Crux is actively tested on Linux (x86_64).

---

## Documentation Index

- [Quickstart Guide (5 Minutes)](docs/QUICKSTART.md) — Create and build your first C++ project with Crux & CMake.
- [Installation Guide](docs/INSTALLATION.md) — Nix Flake, Cargo build, and prebuilt binaries.
- [CLI Reference](docs/CLI_REFERENCE.md) — Full reference for all commands, flags, environment variables, and exit codes.
- [Security Model & Threat Assessment](docs/SECURITY.md) — Trust boundaries, process privileges, and archive protections.
- [Architecture Specification](docs/ARCHITECTURE.md) — Modular design, subsystem boundaries, and data flow.
- [Release Readiness Audit](docs/RELEASE_READINESS_AUDIT.md) — Quality baseline, audit findings, and release criteria.
- [Maintainer Release Checklist](docs/RELEASE_CHECKLIST.md) — Pre-release verification gates for maintainers.
- [Project Roadmap](ROADMAP.md) — Implemented milestones and upcoming features.

---

## Key Features & Status

| Feature | Status | Description |
|---|---|---|
| **Source-First Acquisition** | Ready | Streaming HTTPS download with SHA-256 integrity verification. |
| **Strict SemVer 2.0.0** | Ready | Caret (`^`), tilde (`~`), wildcard (`*`), and comparison ranges (`>=`, `<`). |
| **C/C++ ODR Safety** | Ready | Single-version-per-package resolution prevents duplicate symbol collisions. |
| **Deterministic Lockfiles** | Ready | Canonical, versioned `crux.lock` format (TOML) ensuring byte-for-byte reproducibility. |
| **Git Package Registries** | Ready | Clones and queries Git-backed package definitions and metadata. |
| **CMake Build Engine** | Ready | Isolated out-of-source CMake builds with deterministic fingerprinting. |
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

### 1. Initialize a New Package (`crux init`)

Initialize a new `crux.toml` manifest in the current directory:

```bash
# Initialize using the current directory name
crux init

# Initialize with a custom package name
crux init --name my-library
```

Generated `crux.toml`:
```toml
[package]
name = "my-library"
version = "0.1.0"
description = "A C/C++ package managed by Crux"
```

### 2. Validate Project Manifest (`crux check`)

Validates the syntactic and semantic correctness of `crux.toml`, including package naming rules and dependency version requirements:

```bash
crux check
```

Output:
```text
Package: my-library 0.1.0
Manifest: /path/to/crux.toml
Direct dependencies: 2
  - fmt: ^11.1
  - openssl: ^3.4
Status: Valid
```

### 3. Preview Dependency Resolution (`crux resolve`)

Resolves direct and transitive dependencies against the package catalog without modifying disk state:

```bash
crux resolve
```

Output:
```text
Resolving dependencies for 'my-library' (0.1.0)...
Note: Using in-memory test catalog (development/test fixture only; no remote registry configured).

Resolved 3 dependencies:
  fmt 11.1.1 (registry+default)
  openssl 3.4.0 (registry+default)
  zlib 1.3.1 (registry+default)

Status: Success
```

### 4. Lockfile Management (`crux lock`)

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
Lockfile synchronized: 3 packages locked
  fmt 11.1.1
  openssl 3.4.0
  zlib 1.3.1
```

Output for `crux lock --check`:
```text
Lockfile: /path/to/crux.lock
Packages: 3
Status: Valid (synchronized)
```

### 5. Registry Management (`crux registry`)

Crux supports Git-backed package registries. Manage configured registries using:

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

### 6. Source Acquisition & Verification (`crux fetch`)

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
### 7. CMake Build Engine (`crux build`)

Crux compiles acquired package sources into isolated, out-of-source build directories:

```bash
# Build a specific package (Release profile by default)
crux build fmt

# Build an exact version with Debug profile
crux build fmt@11.1.1 --profile debug

# Force re-configure and re-build, bypassing build cache
crux build fmt --force

# Build all project dependencies declared in crux.lock
crux build

# Print build subcommand help
crux build --help
```

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

### 8. Package Installation Engine (`crux install`)

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

### 9. CMake Consumer Integration (`crux integrate cmake`)

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
openssl = "^3.4"
catch2 = ">=3.4.0, <4.0.0"
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
dependencies = []

[[package]]
name = "openssl"
version = "3.4.0"
source = "registry+default"
dependencies = [
    "zlib 1.3.1",
]

[[package]]
name = "zlib"
version = "1.3.1"
source = "registry+default"
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

### Catalog Subsystem Distinctions

Crux strictly separates catalog sources:
1. **In-Memory Development Catalog**: Embedded fixture used automatically as a fallback when no registry is configured. Intended solely for unit testing and early development.
2. **Local Git Registry**: A Git repository on the local filesystem (e.g. `crux registry add default /path/to/repo` or seed repository at `registry/`). Cloned into Crux's managed storage (`~/.crux/registries/`) and queried directly without network dependencies.
3. **Remote Git Registry**: An upstream Git URL (e.g. `https://github.com/crux-pkg/registry.git`). Cloned locally via safe `git clone` and kept up-to-date with `crux registry update`.
4. **Source Acquisition Subsystem (Milestone 10)**: Package definitions declare source archive URLs and SHA-256 checksums. Crux securely downloads, cryptographically verifies, caches, and extracts archives into content-addressed directories with defense-in-depth safety checks.

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
- **Process Privileges**: Crux currently runs external build scripts and CMake with ambient user privileges. No OS-level containerization or namespace sandbox is active in v0.1.0. See [docs/SECURITY.md](docs/SECURITY.md) for complete details.
- **Archive Extraction Defenses**: Zip Slip path traversal defenses, symlink escape detection, and decompression bomb ceilings are strictly enforced.

---

## Supported Platforms

- **Linux (x86_64)**: Fully supported and validated in CI.
- **Linux (aarch64)**: Planned.
- **macOS / Windows**: Planned for post-v0.1.0 releases.

---

## License

Licensed under the MIT License. See [LICENSE](LICENSE) for details.
