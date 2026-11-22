---
title: Command-Line Interface Reference
description: Complete CLI reference for all Crux commands, flags, environment variables, and exit codes.
---

## Global Options

```text
Usage:
    crux [options]
    crux <command> [args]
```

- `-h, --help`: Display help information
- `-V, --version`: Display Crux version (for example, `crux 0.2.1`)
- `--color <auto|always|never>`: Control colored diagnostics. Precedence: `--color always/never` > `CLICOLOR_FORCE` > `NO_COLOR` > terminal detection.
- `-q, --quiet`: Suppress routine progress; errors and warnings remain.
- `-v, --verbose` / `-vv`: More detail, and with `-vv`, live CMake and compiler output.

Global options work with every command, before or after its name (but before `--`). Status and diagnostics go to stderr; results go to stdout. See [Terminal Output & Diagnostics](/crux/guides/terminal-output/).

---

## Commands

### `crux init`
Initializes a standardized Crux project with `crux.toml`, an initialized `crux.lock`, and a working starter application for C and C++.

```bash
# Initialize a C++ application (default standard c++20)
crux init hello-crux --language cpp

# Initialize a C application (default standard c17)
crux init hello-crux-c --language c

# Initialize an Other project without C/C++ compiler assumptions
crux init my-tool --language other

# Override the language standard
crux init my-app --language c --standard c11

# Initialize using the current directory name
crux init
```

---

### `crux add`
Adds a direct dependency to `crux.toml`, resolves the full dependency graph deterministically, updates `crux.lock`, and installs the package for application builds.

```bash
# Add a dependency (wildcard constraint "*", resolves to latest compatible version)
crux add fmt

# Add with an explicit SemVer constraint
crux add fmt@^11.0
crux add zlib@">=1.2"
crux add zlib@^1.3

# Direct sources
crux add util --git https://example.invalid/util.git --tag v1.2.3
crux add util --url https://example.invalid/util.tar.gz --sha256 <64-hex>
crux add util --path ../local-util
```

Direct-source flags are `--git` with `--tag`, `--branch`, `--rev`, or `--commit`; `--url` with mandatory `--sha256`; and `--path`. See [Direct Package Sources](/crux/guides/direct-sources/).

- When the version constraint is omitted, defaults to `*` (any version).
- Modifies only the `[dependencies]` table of `crux.toml`, preserving comments, ordering, and unrelated sections.
- Atomically updates `crux.toml` and `crux.lock` with automatic rollback on error.
- Immediately installs the new dependency for the Debug profile.
- Rejects duplicate dependencies with an actionable error message.

---

### `crux remove`
Removes a direct dependency from `crux.toml`, re-resolves the remaining dependency graph, and deterministically updates `crux.lock`.

```bash
# Remove a direct dependency
crux remove fmt
```

- Re-resolves remaining dependencies *before* modifying files; shared transitive dependencies required by other packages remain in `crux.lock`.
- Atomically updates `crux.toml` and `crux.lock`.
- Rejects nonexistent direct dependencies with an actionable diagnostic.
- Installed files are left in place to avoid unintended cache deletions.

---

### `crux check`
Validates syntactic and semantic correctness of the project manifest (`crux.toml`).

```bash
crux check
```

---

### `crux resolve`
Resolves dependencies against the active Git registry and previews the resolved graph without modifying disk state. Fresh projects bootstrap the official Crux registry automatically; production resolution never uses an in-memory test catalog.

```bash
crux resolve
```

---

### `crux lock`
Manages deterministic lockfile (`crux.lock`) generation, synchronization, and verification.

```bash
# Generate or synchronize crux.lock with crux.toml
crux lock

# Upgrade all dependencies to their highest compatible versions, and
# explicitly accept changed checksums for already-locked packages
crux lock --update

# Verify lockfile validity and audit locked checksums against the local
# registry metadata (never modifies crux.toml/crux.lock or downloads archives)
crux lock --check
```

A plain `crux lock` never replaces the checksum of a package whose version and source are unchanged: a changed checksum is an integrity error, and `crux.lock` is left untouched. Use `crux lock --update` to accept the change deliberately. Legacy Git checksums from Crux v0.2.0 are migrated automatically, and local path dependencies are refreshed without error. See [Security](/crux/design/security/#lockfile-integrity--re-locking).

`crux lock --check` reports each package as `ok`, `checksum-drift`, `version-missing`, `package-missing`, or `source-changed` (failures); `checksum-unavailable`, `lock-checksum-missing`, or `legacy-checksum` (warnings); or `mutable-path`. It compares only the **locally available** registry metadata, so refresh the registry (`crux registry update <name>`) to see remote changes. An unloadable registry is reported as `Unverified`.

---

### `crux registry`
Manages Git package registries.

```bash
# Add a custom Git package registry (local path or remote URL)
crux registry add company https://example.com/company-crux-registry.git

# List configured registries
crux registry list

# Update an existing registry to latest remote commit
crux registry update company
```

Fresh projects automatically use the official Crux registry (`https://github.com/ShaharyarShakir/crux.git`, registry root `registry/`) and persist it under the name `default`. Custom registries are preserved and are never overwritten by bootstrap.

---

### `crux fetch`
Downloads source archives, validates cryptographic SHA-256 digests, and extracts them into content-addressed cache storage.

```bash
# Fetch all locked dependencies
crux fetch

# Fetch a specific dependency
crux fetch fmt

# Force re-download
crux fetch --force
```

---

### `crux build`
Builds the local Crux application from sources under `src/`, or compiles registry package sources with the internal CMake engine.

```bash
# Build the local application (Debug by default)
crux build

# Select an explicit configuration
crux build --debug
crux build --release

# Show detected toolchain paths and full CMake/compiler output
crux build --verbose

# Force a clean rebuild of the selected configuration
crux build --force

# Build a registry package instead of the local application
crux build fmt
crux build fmt@11.1.1 --profile debug
```

**Dispatch rule**:
- `crux build <package>` builds that package from the configured registry.
- `crux build` with a `[project]` manifest builds the local application.
- `crux build` with a legacy `[package]` manifest (or only a `crux.lock`) builds locked dependencies.

Only registry entries declared with `system = "cmake"` are buildable. Catalog-only packages are rejected with an explicit diagnostic before source acquisition.

Application builds discover `.c` sources for C projects and `.cpp`/`.cc`/`.cxx` sources for C++ projects recursively under `src/`. Crux generates internal CMake configuration at `.crux/build/generated/CMakeLists.txt` and writes artifacts to `.crux/build/debug/` or `.crux/build/release/`. A root-level `CMakeLists.txt` is not required or generated.

---

### `crux run`
Builds and runs a C/C++ application using the application build engine. It accepts either a `[project]` manifest or ad-hoc source files.

```bash
# Build (Debug, default) and run the project application
crux run

# Build and run Release
crux run --release

# Pass arguments to the application (after --)
crux run -- --name "Alice" --count 5

# Run one source file without crux.toml
crux run --bin hello.c

# Run multiple sources with a header and include directory
crux run --sources main.cpp,math.cpp --include-dir include
```

- Defaults to Debug; `--release` runs the Release binary. Compilation failures prevent execution.
- Arguments after `--` are forwarded unchanged to the application.
- Runs with working directory = the caller's directory and inherits environment/stdin/stdout/stderr.
- Crux exits with the application's exit status (nonzero propagated).
- Ad-hoc mode needs no manifest, lockfile, or registry. `.c` selects C; `.cpp`/`.cc`/`.cxx` select C++. `--sources` is repeatable and also accepts comma-separated lists. Include directories may be given as `-I <dir>`, `-I<dir>`, `--include-dir <dir>`, or `--include-dir=<dir>`.
- Ad-hoc artifacts are isolated under `.crux/run/<name>-<hash>/`; the scratch workspace is cleared before each run so failed builds cannot execute stale binaries.

---

### `crux install`
Builds and installs packages into project-local storage (`.crux/installed/`) using staged publication.

```bash
# Install all dependencies
crux install

# Require exact match with crux.lock
crux install --locked
```

---

### `crux integrate`
Generates consumer build-system configuration files.

```bash
# Generate relocatable CMake integration script
# Default output: .crux/cmake/CruxDependencies.cmake
crux integrate cmake

# Specify a custom output path
crux integrate cmake --output .crux/cmake/custom-dependencies.cmake
```

---

## Environment Variables

| Variable | Description | Default |
|---|---|---|
| `CRUX_HOME` | Root directory for Crux data | `~/.crux` |
| `CRUX_CONFIG_PATH` | Path to global configuration | `$CRUX_HOME/config.toml` |
| `CRUX_REGISTRIES_DIR` | Directory containing cloned registries | `$CRUX_HOME/registries` |
| `CRUX_CACHE_DIR` | Directory for downloaded archives & sources | `$CRUX_HOME/cache` |
| `CRUX_REGISTRY_PATH` | Direct override path to an active Git registry | Unset |
| `CRUX_OFFICIAL_REGISTRY_URL` | Git URL or local checkout used to bootstrap the official registry | `https://github.com/ShaharyarShakir/crux.git` |
| `CRUX_OFFLINE` | `1`/`true`/`yes`/`on` disables network bootstrap and requires a local registry | Unset |
| `CC` | Explicit C compiler for checks and application builds | Detected (`gcc`, then `clang`) |
| `CXX` | Explicit C++ compiler for checks and application builds | Detected (`g++`, then `clang++`) |
