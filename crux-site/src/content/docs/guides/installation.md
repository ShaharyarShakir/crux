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
# Expected: crux 0.1.0

crux --help
```
