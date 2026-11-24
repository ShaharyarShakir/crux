---
title: Changelog
description: Release notes and version history for Crux.
---

## [0.3.0] - 2026-11-24

- **Build configuration**: `[build]` table with `c_flags`, `cxx_flags`, and `linker_flags` applied strictly to the local application as discrete CMake arguments without shell invocation.
- **Sanitizers**: AddressSanitizer and UndefinedBehaviorSanitizer (`[build.sanitizers] address / undefined`) for GCC and Clang, verified by automated compile-and-link toolchain probes with structured diagnostics.
- **Application build fingerprinting**: deterministic cache tracking compiler identity, flags in declared order, sanitizers, profile, CMake generator, and dependency graph hash with automatic invalidation.
- **Terminal UI**: 256-color palette and Unicode block progress formatting on capable interactive terminals, with automatic plain ASCII fallback when output is redirected.

## [0.2.1] - 2026-11-20

Forward security/documentation hotfix after v0.2.0. The v0.2.0 tag and release remain published; users of untrusted direct sources should upgrade to v0.2.1.

- **Security**: hardened direct Git and local-path source-tree validation against symlink escape; enforced HTTPS-only remote archives; validated registry and direct-source Git `rev` values so malformed revisions no longer panic.
- **CLI fix**: `crux integrate cmake --output <path>` is now accepted. The default output is `.crux/cmake/CruxDependencies.cmake`.
- **Documentation**: direct-source security coverage, extraction ceiling corrected to 1 GB uncompressed, digest comparison described accurately, stale milestone labels fixed, footer anchors repaired.
- **Release alignment**: M24 released in v0.2.0 and hardened in v0.2.1; M25 hotfix recorded as complete; M24 report amendment and indexes updated.

## [0.2.0] - 2026-11-20

- Standardized project initialization for C, C++, and Other projects (`crux init --language c|cpp|other`) with project configuration validation and compiler/toolchain detection (`CC`/`CXX` overrides).
- Application build and run support: `crux build` and `crux run` compile local C/C++ applications with deterministic internal CMake, isolated Debug/Release trees, recursive source discovery, and symlink confinement.
- Transactional `crux add` and `crux remove`, lockfile preference preservation, automatic Debug dependency installation, and CMake linking for local applications.
- Official catalog expansion to 50 package identities and 53 concrete versions, with a verified CMake tier and reference applications.
- Conservative `crux import` for existing C/C++ projects, including CMake/Makefile detection, include-root discovery, and Crux-managed `.crux/import.toml`.
- Direct package sources: Git tags/branches/revisions/commits, SHA-256-verified archives, and local paths through the normal resolver, lockfile, build, install, and CMake integration pipeline. Git refs lock to immutable commits and are never silently upgraded.
- Expanded guides and references for direct sources, imports, CLI behavior, architecture, security, and release engineering.

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
