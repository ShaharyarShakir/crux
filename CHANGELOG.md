# Changelog

All notable changes to **Crux** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [0.3.0] - 2026-11-24

### Added
- **`[build]` manifest table** with `c_flags`, `cxx_flags` and `linker_flags`, applied to the application only (never to dependencies) as individually quoted CMake arguments; no shell is involved.
- **Sanitizers**: `[build.sanitizers] address / undefined` for GCC and Clang, applied to compile and link, verified by a compile-and-link probe; unsupported toolchains produce a structured diagnostic instead of silently disabling the sanitizer.
- **Application build fingerprint** (compiler identity, flags in declared order, sanitizers, profile, CMake, dependencies); a differing or missing record discards the build tree.
- Unicode status glyphs and block progress bars on capable terminals; 256-color gray for secondary text. Redirected output remains plain ASCII.

---

## [0.2.1] - 2026-11-20

### Security
- **Direct/path source tree symlink escape (HIGH)**: hardened direct Git and local-path source-tree validation so symlinks resolving outside the validated source root are rejected before hashing, copying, or building.
- **HTTPS-only archive enforcement (MEDIUM)**: remote source archives must use HTTPS; unencrypted `http://` archive URLs are rejected.
- **Registry Git `rev` validation (MEDIUM)**: invalid registry and direct-source Git `rev` values are validated before subprocess execution, producing an actionable error instead of a panic.

### Fixed
- **CLI/documentation mismatch (HIGH)**: `crux integrate cmake --output <path>` is now accepted by the CLI. The default output remains `.crux/cmake/CruxDependencies.cmake`, and both CLI references include an example.
- Corrected the documented extraction ceiling from 500 MB to 1 GB uncompressed and removed the inaccurate constant-time digest comparison claim; SHA-256 digests are compared after normalization.
- Fixed stale "M19 application build engine" labels in current docs and repaired website footer anchors to point at real architecture headings.

### Documentation
- Added direct-source coverage to the security model and site security page, including Git argv-only execution with null stdin and timeouts, immutable commit resolution, SHA-256 verification before extraction, real deterministic tree digests, HTTPS-only archives, and conflict/cycle checks.
- Aligned `ROADMAP.md`, the release checklist, the M24 report (with a v0.2.1 amendment), and documentation indexes. v0.2.0 remains tagged/published; v0.2.1 is the forward hotfix. Users of untrusted direct sources should upgrade.

---

## [0.2.0] - 2026-11-20

### Added
- **Project Configuration & Toolchain Detection**:
  - Standardized C, C++, and `other` project initialization with `--language` and `--standard` options.
  - Project-aware compiler detection with explicit `CC`/`CXX` overrides and actionable diagnostics.
- **Application Build & Execution**:
  - `crux build` compiles local C/C++ applications using internal deterministic CMake generation under `.crux/build/generated/`.
  - Isolated Debug and Release application build and runtime trees, `crux run` argument forwarding, stdin passthrough, environment inheritance, and nonzero exit propagation.
  - Recursive source discovery with deterministic ordering, mixed-language rejection, and symlink confinement.
- **Dependency Management**:
  - `crux add` and `crux remove` update `crux.toml` and `crux.lock` transactionally with manifest rollback on failure.
  - Lock preferences are preserved, Debug dependencies are installed automatically, and local applications link installed packages through generated CMake.
- **Official Package Ecosystem**:
  - 50 curated package identities and 53 concrete versions, including a verified CMake-buildable tier and five reference applications.
  - Offline catalog consistency tests and a dedicated package catalog guide.
- **Existing Project Import**:
  - `crux import` performs conservative read-only discovery of C/C++ sources, CMake and Makefile layouts, include roots, and multiple executable candidates.
  - Writes a minimal manifest plus Crux-managed `.crux/import.toml`; preserves user build files and never overwrites an existing manifest.
- **Direct Package Sources**:
  - Git (`--git` with `--tag`, `--branch`, `--rev`, or `--commit`), SHA-256-verified remote archive (`--url` + `--sha256`), and local directory (`--path`) dependencies.
  - Direct sources are normalized into the existing resolver, lockfile, acquisition, build, install, and CMake integration pipeline.
  - Git refs resolve to immutable commits, locked commits are never silently upgraded, and `crux lock --update` is the explicit refresh path.
  - Deterministic source-tree digests, source conflict/cycle detection, path rebasing, moved-workspace support, and safe Git argv execution.
- **Documentation & Release Engineering**:
  - Direct-source and project-import guides, expanded CLI/architecture/security references, and v0.2.0 release notes and checklist.
  - 515 automated unit and integration tests, including offline Git, archive, direct-source, import, application, and E2E CLI coverage.

### Changed
- Version metadata in `Cargo.toml`, `Cargo.lock`, `flake.nix`, installer defaults, release workflow examples, documentation, and website badges updated to v0.2.0.
- `.gitignore` uses the anchored `/build/` rule, keeping `src/build/` Rust sources tracked.

### Security
- Git URLs and refs are validated and passed without shell interpretation; Git subprocesses use null stdin and hard timeouts.
- Remote archives require SHA-256 verification before extraction; existing Zip Slip, symlink escape, and decompression-bomb protections remain enforced.
- Direct Git and local sources receive real deterministic source-tree hashes. No OS-level build sandbox is present in v0.2.0; see the security model.

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
