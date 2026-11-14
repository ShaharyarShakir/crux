# Crux

**Crux** is a source-first, deterministic C/C++ package manager designed to bring modern dependency management, reproducible builds, and first-class semantic versioning to the C and C++ ecosystems.

Written in Rust, Crux provides reliable manifest validation, semantic version resolution with C/C++ ODR (One Definition Rule) safety, and canonical lockfile generation.

---

## Key Features

- **Source-First Philosophy**: Dependencies are fetched from trusted sources and built consistently across environments.
- **Strict Semantic Versioning**: Full support for SemVer 2.0.0 via the `semver` standard crate, including caret (`^`), tilde (`~`), wildcard (`*`), and comparison ranges (`>=`, `<`).
- **C/C++ ODR & Symbol Collision Safety**: The resolution engine enforces a strict single-version-per-package policy across the entire resolved dependency graph, preventing duplicate symbols and undefined linking behavior.
- **Deterministic Lockfiles**: Canonical, versioned `crux.lock` format (TOML) ensuring byte-for-byte reproducibility across runs and environments.
- **Decoupled Architecture**: Clear separation of concerns between package identity, manifest validation, dependency resolution, catalog querying, and lockfile persistence.
- **Extensible Catalog Model**: Clean `PackageCatalog` trait allowing seamless transition from in-memory fixtures to Git-based and remote package registries.

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
```text
Fetching fmt 11.1.1 from registry 'default'...

Successfully acquired fmt 11.1.1:
  Archive: /home/user/.cache/crux/archives/sha256/a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45
  Source:  /home/user/.cache/crux/sources/sha256/a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45
  SHA-256: sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45
```

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
├── catalog.rs      # PackageCatalog trait, InMemoryCatalog, and dev test fixtures
├── check.rs        # Manifest verification and integrity checks
├── cli.rs          # Command-line interface definitions and dispatch
├── dependency.rs   # Dependency declaration model and VersionReq integration
├── init.rs         # Project initialization logic
├── lib.rs          # Library root and public API exports
├── lock.rs         # Lockfile data structures, serialization, and verification
├── main.rs         # CLI entry point
├── manifest.rs     # crux.toml parsing, validation, and serialization
├── package.rs      # PackageId, PackageName, and metadata models
├── registry.rs     # Git-backed package registry, package.toml parser, and config
├── resolver.rs     # Deterministic backtracking resolver with cycle detection
└── source.rs       # Source acquisition, streaming download, SHA-256 verification, and extraction
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

## License

Licensed under the MIT License. See [LICENSE](LICENSE) for details.
