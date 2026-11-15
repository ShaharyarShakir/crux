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
Scaffolds a new package manifest (`crux.toml`) in the current or target directory.

```text
Usage:
    crux init [options]

Options:
    --name <NAME>    Specify custom package name (defaults to current directory name)
    -h, --help       Print help information
```

**Behavior**:
- Rejects invalid package names (must be ASCII alphanumeric, `-`, `_`, non-empty, max 64 chars).
- Refuses to overwrite existing `crux.toml` manifests.

---

### `crux check`
Validates syntactic and semantic correctness of the project manifest (`crux.toml`).

```text
Usage:
    crux check [options]

Options:
    -h, --help       Print help information
```

**Checks Performed**:
- Valid TOML syntax.
- Required `[package]` metadata (`name`, `version`).
- Valid SemVer 2.0.0 package version.
- Dependency name and constraint syntax.

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
Compiles package sources using CMake in an out-of-source build directory.

```text
Usage:
    crux build [package] [options]

Arguments:
    [package]        Optional package name or name@version to build (defaults to all locked dependencies)

Options:
    --profile <type> Build profile: 'debug' or 'release' (defaults to 'release')
    --force          Force rebuild even if build fingerprint matches existing artifacts
    -h, --help       Print help information
```

**Behavior**:
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

---

## 4. Configuration & Metadata Files

### `crux.toml` (Project Manifest)
Located at the root of a Crux-managed user project:
```toml
[package]
name = "my_project"
version = "0.1.0"
description = "My application"

[dependencies]
fmt = "^11.1"
zlib = ">=1.2, <1.4"
```

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
