# Crux v0.1.0 — Final Release Report

**Release**: Crux v0.1.0  
**Target Platform**: `x86_64-unknown-linux-gnu`  
**Base Commit**: `fe42741` (`HEAD -> main`)  
**Date**: November 15, 2026  
**Status**: **READY FOR OWNER APPROVAL**  

---

## 1. Release Summary

Crux is a source-first, deterministic C and C++ package manager written in Rust. Milestone 16 marks the preparation and packaging of the first official public release, **v0.1.0**.

The release provides:
- Single-version One Definition Rule (ODR) safe dependency resolution.
- Git-backed, decentralized package catalog management.
- Streaming HTTP/HTTPS package acquisition with mandatory SHA-256 checksum verification.
- Safe archive extraction with strict Zip Slip directory traversal defenses.
- Out-of-source CMake build orchestration with toolchain detection and build fingerprinting.
- Staged artifact verification and atomic store publication.
- Relocatable CMake integration via `.crux/cmake/CruxDependencies.cmake`.
- Standalone distribution binary, automated installer script, CI/CD automation, and documentation website.

---

## 2. Quality Gate Verification Results

All mandatory Rust quality gates were executed within the Nix flake development environment:

| Gate | Command | Result | Notes |
| :--- | :--- | :--- | :--- |
| **Formatting** | `nix develop --command cargo fmt --check` | **PASS** | 0 formatting discrepancies |
| **Static Analysis** | `nix develop --command cargo clippy --all-targets -- -D warnings` | **PASS** | 0 warnings across all targets |
| **Type Check** | `nix develop --command cargo check --all-targets` | **PASS** | 0 compiler errors |
| **Test Suite** | `nix develop --command cargo test` | **PASS** | **297 passed**, 0 failed, 0 filtered out |
| **Release Build** | `nix develop --command cargo build --release` | **PASS** | Release binary compiled at `target/release/crux` |
| **Website Build** | `cd crux-site && nix develop .. --command pnpm build` | **PASS** | 10 static HTML routes generated with Pagefind search |

---

## 3. Release Artifact Details

The production release artifacts were compiled, stripped of unneeded debug symbols, archived, and cryptographically hashed:

```text
dist/
├── crux-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
└── SHA256SUMS
```

### 3.1 Artifact Specifications

- **Archive File**: `crux-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`
- **File Size**: `2,269,286 bytes` (~2.16 MiB)
- **Archive Contents**:
  - `crux` (executable binary, ELF 64-bit LSB pie executable, x86-64, dynamically linked)
  - `README.md`
  - `LICENSE`
- **SHA-256 Digest**:
  ```text
  a1e4d7ca6e6dd0318c896d4cdfced7061b8a96c129768f80fd93677181a532d8
  ```

### 3.2 Checksum Manifest (`dist/SHA256SUMS`)
```text
a1e4d7ca6e6dd0318c896d4cdfced7061b8a96c129768f80fd93677181a532d8  crux-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
```

---

## 4. Installer Verification Results (`install/install.sh`)

The POSIX-compliant installation script [install/install.sh](file:///home/shaharyar/01__git_repos/crux/install/install.sh) was audited and verified against the following requirements:

1. **Platform Detection**: Identifies OS (`Linux`) and machine architecture (`x86_64`). Gracefully halts with actionable errors on unsupported configurations.
2. **Deterministic Version Pinning**: Defaults to `v0.1.0` with support for `CRUX_VERSION` override.
3. **Cryptographic Checksum Verification**:
   - Downloads `SHA256SUMS` and archive into an isolated `mktemp -d` directory.
   - Computes SHA-256 digest using `sha256sum` (or `shasum -a 256`).
   - Aborts immediately and exits with code 1 if checksum does not match.
4. **Zero Root Privilege**: Installs into `$HOME/.local/bin` (or `CRUX_INSTALL_DIR` override).
5. **Atomic Installation**: Copies binary to temporary filename in destination directory and performs atomic `mv` to avoid race conditions.
6. **Path Guidance**: Verifies if `$HOME/.local/bin` is in `$PATH` and emits exact export instructions for bash, zsh, and fish if not.
7. **Smoke Test Execution**: Executes `--version` on the installed binary to confirm dynamic linker and runtime sanity before declaring success.

---

## 5. GitHub Actions Workflow Automation

Three production CI/CD workflows were created in `.github/workflows/`:

1. **`ci.yml`**:
   - Triggers on pushes and pull requests to `main`.
   - Runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo check --all-targets`, `cargo test --all-targets`, and release compilation.
2. **`release.yml`**:
   - Triggers on published GitHub releases or version tags (`v*`).
   - Builds release binary on `ubuntu-24.04`, packages `crux-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`, calculates `SHA256SUMS`, and automatically attaches them to the GitHub release.
3. **`website.yml`**:
   - Triggers on pushes to `main` modifying `crux-site/**`.
   - Uses pnpm to build static assets and deploys directly to GitHub Pages with base path `/crux`.

---

## 6. Documentation & Website Verification

The documentation website in `crux-site/` was completely redesigned to reflect Crux's engineering-first identity:

- **Hosting Target**: `https://shaharyarshakir.github.io/crux/` (base path: `/crux`).
- **Brand Tokens**: Ice Blue (`#78A9C9`), Dark Theme (`#111110`), Light Theme (`#F5F3EE`).
- **Homepage**: Custom engineered layout featuring:
  - Hero with one-line copyable install command.
  - Interactive terminal window mockup demonstrating end-to-end CLI usage.
  - 6-card architectural capability grid.
  - 6-step workflow lifecycle diagram.
  - Tabbed file viewer for `crux.toml`, `crux.lock`, `CMakeLists.txt`, and `main.cpp`.
  - Responsive mobile navigation and accessible theme toggle.
- **Documentation Pages Published**:
  - `guides/getting-started`
  - `guides/installation`
  - `reference/cli`
  - `registry/packages`
  - `design/architecture`
  - `design/security`
  - `project/roadmap`
  - `project/changelog`
- **Search**: Built-in Pagefind search index generated automatically across all 10 pages.

---

## 7. Registry Readiness

The Crux seed registry format and metadata schema are fully operational:

- **Registry Format**: Git repository with structured `packages/<name>/<version>/package.toml`.
- **Seed Packages**:
  1. `fmt` (`11.1.1`): C++ formatting library.
  2. `zlib` (`1.3.1`): Standard compression library.
- **Deterministic Checksums**: Real, verified cryptographic digests matching upstream tarballs.
- **CMake Configurations**: Build profiles configured for hermetic compilation and target exports (`fmt::fmt`, `ZLIB::ZLIB`).

---

## 8. Real-World Consumer Validation Results

An end-to-end validation was executed outside the repository in `/tmp/crux_e2e_consumer` using the release binary and real seed registry packages:

1. Initialized project: `crux init multi_pkg --lang cpp --std c++20`.
2. Declared dual dependencies in `crux.toml`:
   - `fmt = "^11.1"`
   - `zlib = ">=1.3.0, <2.0.0"`
3. Resolved graph: Single-version ODR invariant satisfied; emitted canonical `crux.lock`.
4. Installed packages: Downloaded and verified upstream archives; built via CMake; staged and published to `.crux/store/`.
5. Generated CMake integration: Emitted `.crux/cmake/CruxDependencies.cmake`.
6. Configured consumer `CMakeLists.txt` and compiled `main.cpp` using GCC 14.3.0.
7. Executed binary: Output confirmed valid runtime linkage:
   ```text
   Crux M16 E2E Validation: Multi-Package Consumer
   fmt version: 110101
   zlib version: 1.3.1
   ```

---

## 9. Security Verification Summary

1. **Cryptographic Verification**:
   - Streaming SHA-256 hashing during acquisition ensures modified or corrupted archives are rejected before extraction.
2. **Path Traversal Defenses**:
   - Zip Slip protection checks all archive entry paths, rejecting directory traversal sequences (`..`), absolute paths, and leading slashes.
3. **Decompression Bomb Protection**:
   - Archive extraction enforces a maximum decompressed size limit (500 MB) to prevent denial-of-service via compression bombs.
4. **Hermetic Resolution**:
   - Dependency resolution is strictly isolated from network I/O and filesystem writes, taking registry metadata purely as in-memory data structures.
5. **Atomic Store Transitions**:
   - Artifacts are written to unique temporary staging trees and renamed atomically to ensure half-installed packages never occur.

---

## 10. Known Limitations & Scope Boundaries

1. **Platform Availability**: `v0.1.0` targets `x86_64-unknown-linux-gnu`. macOS (`aarch64-apple-darwin`), Windows (`x86_64-pc-windows-msvc`), and Linux ARM64 are scheduled for Milestone 17.
2. **Musl Static Target**: The host Nix environment does not include standard library crates for `x86_64-unknown-linux-musl`. The release binary is dynamically linked against GNU `libc` (glibc $\ge$ 2.38).
3. **Remote Registry Fallback**: When offline, registry operations require a pre-cloned local Git repository in `~/.crux/registries/main`.

---

## 11. Step-by-Step Owner Publishing Checklist

The repository owner should perform the following sequence to publish Crux v0.1.0:

### Step 1: Review Git Status & Commit Changes
```bash
cd /home/shaharyar/01__git_repos/crux
git status
git add .
git commit -m "chore(release): prepare crux v0.1.0 release and website redesign"
git push origin main
```

### Step 2: Create and Push Git Tag
```bash
git tag -a v0.1.0 -m "Crux v0.1.0 — First Public Release"
git push origin v0.1.0
```

### Step 3: GitHub Release Creation
Navigate to GitHub Releases: `https://github.com/ShaharyarShakir/crux/releases/new`
- **Tag**: `v0.1.0`
- **Release Title**: `Crux v0.1.0 — Source-First C/C++ Package Manager`
- **Description**: Copy content from [docs/RELEASE_NOTES_v0.1.0.md](file:///home/shaharyar/01__git_repos/crux/docs/RELEASE_NOTES_v0.1.0.md).

### Step 4: Upload Release Artifacts
Attach the pre-packaged files from the `dist/` directory:
- `dist/crux-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`
- `dist/SHA256SUMS`

### Step 5: Verify GitHub Pages Deployment
Check the GitHub Actions `website.yml` run:
- Confirm deployment completes successfully.
- Visit `https://shaharyarshakir.github.io/crux/` and verify the homepage, navigation links, and documentation pages.

### Step 6: Test Installer Against Published Release
Test the live installer script on a clean machine or container:
```bash
curl -fsSL https://raw.githubusercontent.com/ShaharyarShakir/crux/main/install/install.sh | bash
crux --version
```

### Step 7: Announce Release
Announce the release on relevant developer channels (GitHub Discussions, Reddit r/cpp, r/rust, Hacker News, C++ community forums).

---

## 12. Conclusion & Status

Crux Milestone 16 has achieved all technical, architectural, packaging, installer, verification, and documentation objectives with zero quality gate regressions.

**Status**: **READY FOR OWNER APPROVAL**
