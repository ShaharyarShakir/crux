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
| **M10** | Source Download & Verification | Planned | Archive fetching, SHA-256 verification against lockfile, source cache. |
| **M11** | Build System Integration | Planned | CMake toolchain file generation, pkg-config generation, dependency linking. |
| **M12** | Local Path & Override Deps | Planned | Path dependencies (`path = "../lib"`), developer workflow overrides. |
