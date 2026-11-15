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
| **M15** | Workspace Multi-Package & Release Engineering | Planned | Multi-package workspace support, dependency graph visualization, binary distribution, and v0.1.0 release packaging. |
