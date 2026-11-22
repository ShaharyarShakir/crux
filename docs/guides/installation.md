# Crux Installation & Setup Guide

**Document Status**: Current  
**Version**: 0.2.1  
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

### Installer checksum verification

If you install with the release script (`install/install.sh`), this is how the installer verifies downloads

`install/install.sh` downloads the release archive and its `SHA256SUMS` file. It refuses to install unless verification succeeds:

- **One entry, exact name.** `SHA256SUMS` must contain exactly one well-formed line (`<64 hex digits>  <file name>`) whose file name is *exactly* the archive name. A missing entry, a malformed line, or duplicate entries for the archive abort the installation with an explicit error.
- **Verify before execute.** The archive's SHA-256 must match that entry before the archive is extracted or the binary is executed.
- **HTTPS only.** Downloads use HTTPS, and curl is restricted to HTTPS for redirects as well. `CRUX_RELEASE_BASE_URL` may point to an `https://` mirror, or to a local `file://` directory when using curl. Plain `http://` is rejected.
- **Strict version input.** `--version` / `CRUX_VERSION` must be a release tag such as `v0.2.1`.

The checksum file is published next to the archive, so it protects against corrupted or tampered downloads and mirrors, but not against a compromised release page. For stronger assurance, compare the checksum with one obtained through a separate channel.

---

## 4. Verifying the Installation

Run the verification commands to confirm that Crux is correctly installed and discovers system toolchains:

```bash
# 1. Verify executable version
crux --version
# Expected: crux 0.2.1

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
- **Problem**: `error: package `crux v0.2.1` requires rust-version = "1.85.0"` or syntax errors with edition 2024.
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
