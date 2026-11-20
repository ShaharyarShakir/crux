# Crux Installation & Setup Guide

**Document Status**: Current  
**Version**: 0.1.0  
**Target Platforms**: Linux (tested on x86_64; aarch64 planned)  

---

## 1. Prerequisites

Before installing Crux, verify that the following system tools are available:

| Requirement | Minimum Version | Purpose |
|---|---|---|
| **Rust Toolchain** | 1.85.0+ (Edition 2024) | Compiling the Crux CLI and library |
| **CMake** | 3.20+ | Package build configuration, compilation, and staging |
| **C/C++ Compiler** | GCC 10+ or Clang 11+ | Compiling C and C++ dependency sources |
| **Git** | 2.25+ | Registry cloning, index synchronization, and version queries |
| **OpenSSL / TLS Roots** | System CA certificates | Secure HTTPS package source acquisition |

---

## 2. Recommended: Nix / NixOS (Flake)

Crux provides a fully reproducible [flake.nix](../../flake.nix) providing a verified Rust 1.85+ environment, CMake, GCC, and dependencies.

### Interactive Development Shell
```bash
# Clone the repository
git clone https://github.com/ShaharyarShakir/crux.git
cd crux

# Enter the isolated Nix environment
nix develop

# Build Crux
cargo build --release
```

### Direct Nix Build & Installation
```bash
# Build the release package via Flake
nix build

# Run the built executable
./result/bin/crux --version

# Install into user profile
nix profile install .
```

---

## 3. Standard Cargo Build & Install

If using standard `rustup` on Linux:

### Install via Cargo Path
```bash
# Clone the repository
git clone https://github.com/ShaharyarShakir/crux.git
cd crux

# Install directly to ~/.cargo/bin
cargo install --path .

# Verify installation (ensure ~/.cargo/bin is in your PATH)
crux --version
```

### Build Precompiled Release Binary
```bash
cargo build --release

# The compiled binary is located at:
./target/release/crux --help

# Optionally copy to /usr/local/bin or ~/.local/bin
cp ./target/release/crux ~/.local/bin/
```

---

## 4. Verifying the Installation

Run the verification commands to confirm that Crux is correctly installed and discovers system toolchains:

```bash
# 1. Verify executable version
crux --version
# Expected: crux 0.1.0

# 2. Inspect available subcommands
crux --help

# 3. Test project initialization in a temporary directory
mkdir /tmp/crux_verify && cd /tmp/crux_verify
crux init --name verify-project
crux check
cd .. && rm -rf /tmp/crux_verify
```

---

## 5. Troubleshooting & FAQ

### 1. `rustc` Compilation Errors (MSRV Failure)
- **Problem**: `error: package `crux v0.1.0` requires rust-version = "1.85.0"` or syntax errors with edition 2024.
- **Solution**: Crux requires Rust 1.85.0 or later. Update your toolchain via `rustup update stable` or use the provided `nix develop` shell.

### 2. Missing CMake or Compiler During `crux build`
- **Problem**: `Toolchain detection failed: cmake binary not found in PATH`.
- **Solution**: Install CMake and build essentials:
  - Ubuntu/Debian: `sudo apt-get install cmake build-essential git`
  - Fedora/RHEL: `sudo dnf install cmake gcc-c++ git`
  - Arch Linux: `sudo pacman -S cmake gcc git`
  - Nix: run inside `nix develop`.

### 3. Git Registry Access Denied or SSH Host Verification Failure
- **Problem**: Adding a registry fails during Git clone.
- **Solution**: Verify network access and valid Git credentials. When using local registries or file paths, pass the absolute path:
  ```bash
  crux registry add default /path/to/registry
  ```

### 4. Cache and Environment Isolation
Crux stores global configuration, caches, and cloned registries in `~/.crux`. You can override these directories using environment variables:
- `CRUX_HOME`: Base directory for Crux data.
- `CRUX_CONFIG_PATH`: Path to `config.toml`.
- `CRUX_REGISTRIES_DIR`: Storage path for cloned registries.
- `CRUX_CACHE_DIR`: Storage path for downloaded archives and extracted sources.
