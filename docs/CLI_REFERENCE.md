# Crux Command-Line Interface Reference

**Version**: 0.1.0  
**Binary Name**: `crux`  

---

## 1. Global Usage & Exit Codes

```text
Usage:
    crux [options]
    crux <command> [args]
```

### Global Options

| Option | Short | Description |
|---|---|---|
| `--help` | `-h` | Display top-level help or subcommand help when combined with `<command>` |
| `--version` | `-V` | Print Crux version information (`crux 0.1.0`) |

### Standard Exit Codes

| Exit Code | Classification | Meaning |
|---|---|---|
| `0` | Success | Operation completed successfully |
| `1` | Runtime / General Error | Command failed (e.g. build failure, lockfile out-of-sync, network failure, missing file) |
| `2` | Argument Parsing Error | Unknown flag, invalid argument value, or missing required parameter |

---

## 2. Command Reference

### `crux init`
Scaffolds a new Crux project or package with standardized conventions, `crux.toml`, initialized `crux.lock`, and working starter applications for C and C++.

```text
Usage:
    crux init [path] [options]

Arguments:
    [path]               Target directory for the project (defaults to current directory)

Options:
    -n, --name <NAME>    Specify custom package/project name (defaults to directory name)
    -l, --language <LANG> Project language: c, cpp (or c++), other
    -s, --standard <STD> Language standard (e.g. c17 for C, c++20 for C++)
    -h, --help           Print help information
```

**Behavior**:
- Interactive selection: Prompts user to select C, C++, or Other when stdin is a terminal and `--language` is omitted.
- Non-interactive mode: Supports piped input or safely defaults to C++ (`c++20`) in automated environments.
- Minimal default structure ("Start small. Let projects grow naturally."):
  - **C**: Generates `crux.toml`, `crux.lock`, and `src/main.c`.
  - **C++**: Generates `crux.toml`, `crux.lock`, and `src/main.cpp`.
  - **Other**: Generates `crux.toml`, `crux.lock`, and an empty `src/` directory.
- Optional directories (`src/modules/`, `include/`, `tests/`, `.crux/`) may be created by users as their project grows, or by Crux commands on demand.
- Source Discovery Rule: Crux recursively discovers application source files under `src/` only.
- Generates compilable starter application for C and C++; leaves directories clean for Other.
- Initializes canonical `crux.lock` version 1.
- Validates language-standard combinations (e.g. rejects C++ standards for C, or standards on Other).
- Atomic safety: Refuses to overwrite existing manifests, lockfiles, or user source files.
- Option A CMake Boundary: Ordinary Crux-managed projects do not require or generate a root-level `CMakeLists.txt`.

---

### `crux check`
Validates syntactic and semantic correctness of the project manifest (`crux.toml`) and inspects toolchain availability.

```text
Usage:
    crux check [options]

Options:
    -h, --help       Print help information
```

**Checks Performed**:
- Valid TOML syntax.
- Required `[project]` or legacy `[package]` metadata (`name`, `version`).
- Valid SemVer 2.0.0 package version.
- Language and standard validation (supported C/C++ standards).
- Toolchain inspection: Verifies compiler availability for C and C++ projects and tests standard compatibility.
- Dependency name and constraint syntax.

---

### `crux toolchain`
Detects and inspects installed C and C++ compilers, reporting compiler family, version, target triple, executable path, and supported language standards.

```text
Usage:
    crux toolchain [options]

Options:
    -l, --language <LANG> Filter toolchain inspection by language (c or cpp)
    -h, --help           Print help information
```

**Behavior**:
- Without `--language`: Automatically detects both C and C++ toolchains on the host platform. If run inside a project directory, evaluates compatibility against the project's requested standard.
- With `--language c`: Inspects the C compiler (`gcc`, `clang`, or `$CC`).
- With `--language cpp`: Inspects the C++ compiler (`g++`, `clang++`, or `$CXX`).
- With `--language other`: Reports that compiler detection is not applicable for Other projects.
- Standard capability probing: Performs isolated, non-destructive probe compilations in a temporary directory to verify compiler support for standards (`c17`, `c23`, `c++20`, `c++23`).
- Compiler override: Respects `CC` and `CXX` environment variables. If an override path is invalid or missing, reports clear, actionable errors.

---

### `crux resolve`
Executes dependency resolution against active registries and previews the resolved graph without modifying disk state.

```text
Usage:
    crux resolve [options]

Options:
    -h, --help       Print help information
```

**Behavior**:
- Queries active Git registries (or fallback development fixture).
- Evaluates constraints with strict SemVer precedence.
- Enforces single-version One Definition Rule (ODR) collision prevention.
- Prints resolution summary with resolved versions and package sources.

---

### `crux lock`
Manages deterministic lockfile (`crux.lock`) generation, synchronization, and integrity checks.

```text
Usage:
    crux lock [options]

Options:
    --update         Upgrade all dependencies to latest compatible versions (re-resolves)
    --check          Verify lockfile validity and manifest synchronization without modifying files
    -h, --help       Print help information
```

**Behavior**:
- `crux lock`: Synchronizes `crux.lock` with `crux.toml`, preserving existing locked versions if still compatible.
- `crux lock --update`: Discards locked version preferences and resolves highest compatible versions.
- `crux lock --check`: Exits with code `1` if `crux.lock` is missing, invalid, or out-of-sync with `crux.toml`.

---

### `crux registry`
Manages Git package registries.

```text
Usage:
    crux registry <subcommand> [args]

Subcommands:
    list             List all configured registries and package/version counts
    add <name> <url> Add a new Git package registry (clones into managed registry store)
    update <name>    Update an existing Git registry to the latest remote commit

Options:
    -h, --help       Print help information
```

**Notes**:
- `<url>` can be a remote Git URL (`https://...` or `git@...`) or a local filesystem directory.
- Registry definitions are stored under `~/.crux/registries/<name>/` and registered in `~/.crux/config.toml`.

---

### `crux fetch`
Downloads source archives, validates cryptographic SHA-256 digests, and safely extracts them into content-addressed cache storage.

```text
Usage:
    crux fetch [package] [options]

Arguments:
    [package]        Optional package name or name@version to fetch (defaults to all locked dependencies)

Options:
    --force          Force re-download and re-extraction even if cached
    -h, --help       Print help information
```

**Security Guards**:
- TLS required for HTTP downloads.
- SHA-256 verified before extraction; partial downloads deleted on mismatch.
- Zip Slip and symlink escape defenses enforced during extraction.
- Decompression bomb limits (500 MB max uncompressed size, 50,000 files, 100:1 ratio).

---

### `crux build`
Builds the local Crux application (for `[project]` manifests) or compiles package sources using the internal CMake engine.

```text
Usage:
    crux build [package] [options]

Arguments:
    [package]        Optional package name or name@version to build
                     (omit to build the local application)

Options:
    --debug          Build with debug configuration (default for applications)
    --release        Build with release configuration
    --profile <type> Build profile: 'debug' or 'release' (defaults to release for packages, debug for applications)
    -v, --verbose    Show verbose compiler and CMake output
    --force          Force rebuild even if cache is valid
    -h, --help       Print help information
```

**Dispatch rule**:
- `crux build <package>` builds that package from the configured registry.
- `crux build` with a `[project]` manifest builds the local application.
- `crux build` with a legacy `[package]` manifest (or only a `crux.lock`) retains v0.1.0 behavior and builds locked dependencies.

**Application build behavior**:
- Loads and validates `crux.toml`, then reuses project configuration and toolchain detection.
- Recursively discovers C sources (`.c`) or C++ sources (`.cpp`, `.cc`, `.cxx`) under `src/` only.
- Rejects mixed C/C++ sources, unsupported source extensions, and language `other` with actionable diagnostics.
- Generates internal CMake configuration under `.crux/build/generated/CMakeLists.txt`.
- Configures and compiles out-of-source into `.crux/build/debug/` or `.crux/build/release/`.
- Writes configure/build logs to `.crux/build/logs/`.
- Requires no root-level `CMakeLists.txt`; never modifies user sources or root CMake configuration.
- Defaults to Debug; `--release` selects Release. Debug and Release artifacts are isolated.
- `--verbose` prints detected compiler/CMake paths, discovered sources, and captured CMake output.
- `--force` discards the selected profile build tree and performs a clean rebuild.

**Package build behavior**:
- Automatically fetches and extracts dependencies if missing.
- Computes deterministic build fingerprints based on package identity, profile, source checksum, and toolchain info.
- Preserves full compiler diagnostics in `.crux/builds/<pkg>/<version>/<fingerprint>/logs/`.

---

### `crux install`
Builds and installs packages into project-local storage (`.crux/installed/`) using staged publication.

```text
Usage:
    crux install [package] [options]

Arguments:
    [package]        Optional package name or name@version to install (defaults to all locked dependencies)

Options:
    --profile <name> Build profile: 'debug' or 'release' (defaults to 'release')
    --force          Force reinstall even if already installed with matching fingerprint
    --locked         Require exact synchronization with crux.lock; fails if lockfile missing or modified
    -h, --help       Print help information
```

**Behavior**:
- Runs `cmake --install` into a private staging directory.
- Inspects and inventories all installed files into `installed.toml`.
- Atomically renames staging directory to destination on successful verification.

---

### `crux integrate`
Generates consumer build-system configuration files (e.g. CMake integration script).

```text
Usage:
    crux integrate <subcommand> [options]

Subcommands:
    cmake            Generate relocatable CMake integration script

Options (for cmake):
    --profile <type> Build profile: 'debug' or 'release' (defaults to 'release')
    --output <path>  Custom output path (defaults to '.crux/cmake/CruxDependencies.cmake')
    -h, --help       Print help information
```

**Generated Script Features**:
- Populates `CMAKE_PREFIX_PATH` and `<Package>_ROOT` variables.
- Enables standard `find_package(<Pkg> CONFIG REQUIRED)`.
- Relocatable path calculation relative to `${CMAKE_CURRENT_LIST_DIR}`.

---

## 3. Environment Variables

| Variable | Description | Default |
|---|---|---|
| `CRUX_HOME` | Root directory for Crux user data and configuration | `~/.crux` |
| `CRUX_CONFIG_PATH` | Path to global Crux configuration file | `$CRUX_HOME/config.toml` |
| `CRUX_REGISTRIES_DIR` | Directory containing cloned Git registries | `$CRUX_HOME/registries` |
| `CRUX_CACHE_DIR` | Directory for downloaded archives and extracted sources | `$CRUX_HOME/cache` |
| `CRUX_REGISTRY_PATH` | Direct override path to an active Git package registry | Unset |
| `CC` | Explicit C compiler executable for checks and application builds | Detected (`gcc`, then `clang`) |
| `CXX` | Explicit C++ compiler executable for checks and application builds | Detected (`g++`, then `clang++`) |

---

## 4. Configuration & Metadata Files

### `crux.toml` (Project Manifest)
Located at the root of a Crux-managed user project:
```toml
[project]
name = "my_project"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
fmt = "^11.1"
zlib = ">=1.2, <1.4"
```

Legacy `[package]` manifests remain supported for package projects. A `[project]` manifest declares a local application that `crux build` compiles from `src/`.

### `.crux/build/` (Generated Application Build Tree)
Created on demand by `crux build` for application projects:
```text
.crux/build/
├── generated/
│   └── CMakeLists.txt      # internal CMake project generated by Crux
├── debug/                  # Debug build tree and executable
├── release/                # Release build tree and executable
└── logs/
    ├── configure-debug.log
    ├── configure-release.log
    ├── build-debug.log
    └── build-release.log
```

These files are disposable internal artifacts. Projects should ignore `.crux/` in version control; Crux does not modify a user's `.gitignore` automatically.

### `crux.lock` (Lockfile)
Auto-generated canonical dependency record:
```toml
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"
```

### `config.toml` (Crux Global Configuration)
Located at `~/.crux/config.toml`:
```toml
[registries.default]
url = "https://github.com/ShaharyarShakir/crux-registry.git"
path = "/home/user/.crux/registries/default"
primary = true
```

### `installed.toml` (Installation Metadata)
Located at `.crux/installed/<pkg>/<version>/<fingerprint>/installed.toml`:
```toml
format_version = 1
package_name = "fmt"
package_version = "11.1.1"
fingerprint = "sha256:16980a71eba7eea86813317ad46fa0065484ce172c1e5fdeb234aaa0f721259c"
profile = "release"
total_size_bytes = 821076
success = true
installed_at = "2026-11-15T18:08:00Z"

[[installed_files]]
relative_path = "include/fmt/core.h"
file_type = "header"
size_bytes = 187
```
