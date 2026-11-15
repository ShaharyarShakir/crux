# Changelog

All notable changes to **Crux** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0] - 2026-11-15

### Added
- **Project Initialization & Manifest Validation**:
  - `crux init` command for scaffolding `crux.toml` manifests with strict name validation.
  - `crux check` command validating package name, version, and dependency constraints.
- **Strict Semantic Versioning & Resolution Engine**:
  - SemVer 2.0.0 compliance via `semver` crate with support for caret (`^`), tilde (`~`), wildcard (`*`), and comparison ranges (`>=`, `<`).
  - Deterministic backtracking dependency resolver enforcing C/C++ One Definition Rule (ODR) collision prevention (at most one version of any package in the resolved dependency graph).
  - Breadth-first search cycle detection.
  - `crux resolve` command for previewing the resolved dependency graph without touching disk state.
- **Deterministic Lockfiles**:
  - `crux.lock` TOML format (version 1) with byte-for-byte canonical reproducibility and SHA-256 digests.
  - `crux lock` command for generating, synchronizing, and checking (`--check`, `--update`) lockfiles.
- **Git Package Registries**:
  - `crux registry` CLI (`add`, `list`, `update`) for cloning and managing local and remote Git registries.
  - Initial seed registry supporting `fmt` (11.0.2, 11.1.1), `zlib` (1.3.0, 1.3.1), and `catch2` (3.7.1, 3.8.0).
- **Source Acquisition & Security**:
  - Streaming HTTPS downloads using `ureq` with TLS certificate validation.
  - SHA-256 integrity verification before cache persistence or extraction; automatic deletion of corrupt payloads.
  - Safe extraction engine for `.zip`, `.tar`, and `.tar.gz` with defense-in-depth against Zip Slip path traversal and decompression bombs (500 MB max size, 50k files, 100:1 compression ratio).
  - Content-addressed archive and source caches (`~/.crux/cache`).
  - `crux fetch` command.
- **CMake Build Engine**:
  - Out-of-source CMake compilation (`.crux/builds/<pkg>/<version>/<fingerprint>/`).
  - Deterministic `BuildFingerprint` generation incorporating package identity, build profile, source checksum, and compiler toolchain detection.
  - Build diagnostics and stdout/stderr capture in dedicated log files.
  - `crux build` command.
- **Staged Package Installation**:
  - Atomic two-stage installation engine: staging directory verification followed by atomic directory publication to `.crux/installed/<pkg>/<version>/<fingerprint>/`.
  - Symlink escape detection and classification of installed files into `installed.toml` inventory.
  - `crux install` command supporting `--locked` and `--force` flags.
- **CMake Consumer Integration**:
  - Relocatable `.crux/cmake/CruxDependencies.cmake` generator with safe path escaping and variable sanitization.
  - Automatic `CMAKE_PREFIX_PATH` configuration enabling standard `find_package(<Pkg> CONFIG REQUIRED)`.
  - `crux integrate cmake` command.
- **Distribution & Developer Tooling**:
  - Reproducible `flake.nix` providing development shell and `packages.default` for Nix builds.
  - GitHub Actions CI workflow (`.github/workflows/ci.yml`), release workflow (`.github/workflows/release.yml`), and documentation deployment (`.github/workflows/website.yml`).
  - One-line curl/sh installer script (`install/install.sh`).
  - Complete documentation suite: Installation, Quickstart, CLI Reference, Security Threat Model, Architecture, and Release Checklist.
