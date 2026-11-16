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
- `-V, --version`: Display Crux version

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

### `crux check`
Validates syntactic and semantic correctness of the project manifest (`crux.toml`).

```bash
crux check
```

---

### `crux resolve`
Resolves dependencies against configured registries and previews the resolved graph without modifying disk state.

```bash
crux resolve
```

---

### `crux lock`
Manages deterministic lockfile (`crux.lock`) generation, synchronization, and verification.

```bash
# Generate or synchronize crux.lock with crux.toml
crux lock

# Upgrade all dependencies to their highest compatible versions
crux lock --update

# Verify lockfile validity without modifying files (exits with error if out of sync)
crux lock --check
```

---

### `crux registry`
Manages Git package registries.

```bash
# Add a new Git package registry (local path or remote URL)
crux registry add default https://github.com/ShaharyarShakir/crux-registry.git

# List configured registries
crux registry list

# Update an existing registry to latest remote commit
crux registry update default
```

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

Application builds discover `.c` sources for C projects and `.cpp`/`.cc`/`.cxx` sources for C++ projects recursively under `src/`. Crux generates internal CMake configuration at `.crux/build/generated/CMakeLists.txt` and writes artifacts to `.crux/build/debug/` or `.crux/build/release/`. A root-level `CMakeLists.txt` is not required or generated.

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
crux integrate cmake

# Specify custom output path
crux integrate cmake --output .crux/cmake/CruxDependencies.cmake
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
| `CC` | Explicit C compiler for checks and application builds | Detected (`gcc`, then `clang`) |
| `CXX` | Explicit C++ compiler for checks and application builds | Detected (`g++`, then `clang++`) |
