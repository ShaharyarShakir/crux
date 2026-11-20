# Crux Project Roadmap

This document outlines the milestones, current implementation status, and future technical roadmap for **Crux**, a source-first C/C++ package manager in Rust.

---

## Milestone Overview & Implementation Status

| Milestone | Title | Status | Description |
|---|---|---|---|
| **M1** | Project Initialization & Scaffold | Completed | Initial repository setup, CLI scaffold (`crux init`, `crux --help`, `crux --version`), Nix flake environment. |
| **M2** | Manifest Format & Validation | Completed | `crux.toml` schema, TOML parser, package validation (`crux check`), strict naming rules. |
| **M3** | Workspace & Build Scaffold | Completed | Multi-crate foundation, clean module architecture, and unified error handling. |
| **M4** | Package Metadata & SemVer | Completed | `PackageId`, `PackageName`, `PackageMetadata`, SemVer 2.0.0 integration (`semver` crate), precedence ordering. |
| **M5** | Lockfile Model & Integrity | Completed | `crux.lock` TOML format (version 1), deterministic canonical serialization, SHA-256 `Checksum`, `crux lock --check`. |
| **M6** | Dependency Specification | Completed | `[dependencies]` in `crux.toml`, caret (`^`), tilde (`~`), comparator ranges, typed `Dependency` model. |
| **M7** | Dependency Resolution Engine | Completed | Deterministic backtracking resolver (`Resolver`), `PackageCatalog` trait, `InMemoryCatalog`, single-version ODR safety, BFS cycle detection, `crux resolve`. |
| **M8** | Lockfile Generation & Sync | Completed | Persistent `crux.lock` writing, lock-aware resolution preferences, `--update`, synchronization check, atomic file replacement. |
| **M9** | Git Registry Index | Completed | Git-backed package catalog (`GitRegistry`), `package.toml` schema, `crux registry` CLI (`add`, `list`, `update`), seed registry with verified upstream packages (fmt, zlib, catch2). |
| **M10** | Source Download & Verification | Completed | Secure source acquisition (`SourceAcquirer`), streaming HTTP/HTTPS, SHA-256 integrity verification, content-addressed archive & source cache, safe extraction (.zip, .tar, .tar.gz), Zip Slip/bomb prevention, `crux fetch` CLI. |
| **M11** | CMake Build Engine | Completed | Isolated CMake compilation (`CMakeBuildEngine`), out-of-source builds (`.crux/builds/<pkg>/<version>/<fingerprint>/`), deterministic `BuildFingerprint`, toolchain detection, persistent `metadata.toml`, caching & rebuild, `crux build` CLI. |
| **M12** | Package Installation Engine | Completed | Project-local staging and atomic installation (`cmake --install`), persistent `installed.toml` file inventory metadata, symlink escape jail, deterministic publication, and CLI (`crux install`). |
| **M13** | CMake Consumer Integration | Completed | Deterministic `.crux/cmake/CruxDependencies.cmake` generation, relocatable path resolution (`CMAKE_CURRENT_LIST_DIR`), `CMAKE_PREFIX_PATH` & `CMAKE_MODULE_PATH` integration, upstream config package discovery (fmt, Catch2) & built-in Find modules (ZLIB), system-library isolation, CLI (`crux integrate cmake`). |
| **M14** | Modular Architecture Refactoring | Completed | Modular decomposition of codebase into domain directories (`core`, `package`, `project`, `resolution`, `registry`, `source`, `build`, `install`, `integrate`, `cli`), preserving single-crate model and backwards compatibility. |
| **M15** | Release Engineering | Completed | Release readiness audit, Cargo package metadata, MIT licensing, release binary validation, Nix flake packaging, GitHub Actions CI quality gates, E2E validation against real C++ consumer with fmt, security threat model and sandbox disclosure, and developer documentation suite. |
| **M16** | v0.1.0 Release Readiness & Distribution | Release Ready | Release binaries, installer script (install.sh), GitHub Actions CI/Release/Website workflows, Astro documentation site (crux-site), multi-dependency E2E validation, and release notes prepared. Awaiting repository owner approval and formal Git tagging. |
| **M17** | Workspace Multi-Package Support | Planned | Multi-project workspaces with shared dependencies and lockfiles (`[workspace]` manifests). |
| **M18** | Isolated Build Sandboxing | Planned | OS-level containerized / namespaced build sandboxing (`bubblewrap` on Linux, `sandbox-exec` on macOS) to isolate CMake and compiler execution. |
| **M19** | Binary Artifact Caching | Planned | Prebuilt binary package cache distribution with cryptographic provenance. |
| **M20** | Application Toolchain & Execution | Completed | Toolchain detection, application execution, and release/Debug runtime handling. |
| **M21** | Dependency Management & Application Integration | Completed | `crux add`/`remove`, lock synchronization, automatic installation, and internal CMake linking for local applications. |
| **M22** | Official Package Ecosystem Expansion | Completed | 50-package official catalog, 15 verified CMake packages, five real reference applications, catalog tests, and ecosystem documentation. |
