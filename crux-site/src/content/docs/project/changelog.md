---
title: Changelog
description: Release notes and version history for Crux.
---

## Unreleased (v0.2.0 development)

- Standardized project initialization for C, C++, and Other projects (`crux init --language c|cpp|other`).
- Project configuration validation and compiler/toolchain detection with `CC`/`CXX` overrides.
- Application build engine: `crux build` compiles local C and C++ applications from sources under `src/` using internally generated CMake under `.crux/build/generated/`.
- Debug and Release application builds with isolated `.crux/build/debug/` and `.crux/build/release/` trees.
- Recursive source discovery, deterministic ordering, mixed-language rejection, and symlink confinement.

## [0.1.0] - 2026-11-15

Initial pre-release of **Crux**, a source-first, deterministic C/C++ package manager in Rust.

### Key Capabilities
- Project manifest initialization (`crux init`) and validation (`crux check`).
- Backtracking dependency resolver with C/C++ One Definition Rule (ODR) collision prevention (`crux resolve`).
- Canonical, deterministic lockfile format (`crux lock`).
- Git package registry management (`crux registry add`, `crux registry list`, `crux registry update`).
- Streaming HTTPS source download and real-time SHA-256 verification (`crux fetch`).
- Safe archive extraction with Zip Slip and decompression bomb limits.
- Isolated out-of-source CMake build engine with build fingerprinting (`crux build`).
- Staged, atomic package installation with file inventory metadata (`crux install`).
- Relocatable CMake consumer integration (`crux integrate cmake`).
- One-line curl installer (`install/install.sh`).
- Reproducible Nix flake packaging (`flake.nix`).
