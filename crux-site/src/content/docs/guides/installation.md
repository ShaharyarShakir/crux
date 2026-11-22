---
title: Installation Guide
description: Installation options for Crux on Linux systems via curl, Nix, and Cargo.
---

Crux is currently tested on **Linux (x86_64)**.

---

## Method 1: Automated Script (Recommended)

Run the one-line installation script to download and install the official prebuilt binary to `~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/ShaharyarShakir/crux/main/install/install.sh | bash
```

### Inspect Before Executing
We encourage inspecting installer scripts before running them:

```bash
curl -fsSL https://raw.githubusercontent.com/ShaharyarShakir/crux/main/install/install.sh -o install-crux.sh
less install-crux.sh
bash install-crux.sh
```

### How the installer verifies downloads

`install/install.sh` downloads the release archive and its `SHA256SUMS` file. It refuses to install unless verification succeeds:

- **One entry, exact name.** `SHA256SUMS` must contain exactly one well-formed line (`<64 hex digits>  <file name>`) whose file name is *exactly* the archive name. A missing entry, a malformed line, or duplicate entries for the archive abort the installation with an explicit error.
- **Verify before execute.** The archive's SHA-256 must match that entry before the archive is extracted or the binary is executed.
- **HTTPS only.** Downloads use HTTPS, and curl is restricted to HTTPS for redirects as well. `CRUX_RELEASE_BASE_URL` may point to an `https://` mirror, or to a local `file://` directory when using curl. Plain `http://` is rejected.
- **Strict version input.** `--version` / `CRUX_VERSION` must be a release tag such as `v0.2.1`.

The checksum file is published next to the archive, so it protects against corrupted or tampered downloads and mirrors, but not against a compromised release page. For stronger assurance, compare the checksum with one obtained through a separate channel.

---

## Method 2: Nix / NixOS (Flake)

Crux provides a fully reproducible `flake.nix`:

### Enter Development Shell
```bash
git clone https://github.com/ShaharyarShakir/crux.git
cd crux
nix develop
cargo build --release
```

### Build Package via Flake
```bash
nix build
./result/bin/crux --version
```

### Install into User Profile
```bash
nix profile install .
```

---

## Method 3: Cargo Build from Source

Prerequisites: Rust 1.85+ (Edition 2024), CMake 3.20+, GCC or Clang, Git.

```bash
git clone https://github.com/ShaharyarShakir/crux.git
cd crux
cargo install --path .
```

---

## Verifying Installation

Verify that Crux is correctly installed:

```bash
crux --version
# Expected: crux 0.2.1

crux --help
```
