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
Initializes a new `crux.toml` package manifest.

```bash
# Initialize using the current directory name
crux init

# Specify a custom package name
crux init --name my-library
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
Compiles package sources using CMake in an out-of-source build directory.

```bash
# Build all locked dependencies (defaults to release profile)
crux build

# Build with debug profile
crux build --profile debug

# Force rebuild
crux build --force
```

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
