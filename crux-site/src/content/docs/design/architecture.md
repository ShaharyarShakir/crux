---
title: System Architecture
description: Overview of Crux's modular subsystem architecture, data contracts, and design principles.
---

Crux is built as a modular single-crate application in Rust, organized into ten decoupled domain subsystems.

---

## Modular Subsystem Layout

```text
src/
├── core/           # Paths, atomic filesystem operations, nonces, timed subprocesses
├── package/        # PackageName, PackageId, PackageMetadata, Dependency
├── project/        # Manifest model (crux.toml), project init, and check
├── resolution/     # Catalog abstraction, backtracking Resolver, and Lockfile
├── registry/       # Git-backed package registry index and configuration
├── source/         # Streaming download, SHA-256 verification, and archive cache
├── build/          # Application source discovery, CMake generation, toolchain, fingerprints
├── install/        # Staging, file inventory, and atomic package installation
├── integrate/      # CMake consumer integration (CruxDependencies.cmake)
├── cli/            # Argument parsing (args.rs) and subcommand runners (commands/)
├── lib.rs          # Public library exports
└── main.rs         # CLI binary entry point
```

---

## Core Architectural Invariants

### 1. Single-Version One Definition Rule (ODR) Safety
In C and C++, linking multiple versions of the same shared or static library causes duplicate symbol collisions or undefined behavior. The Crux resolver enforces a strict single-version-per-package invariant across the resolved dependency graph: at most one version of any package may exist.

### 2. Deterministic & Canonical Serialization
`crux.lock` uses version 1 TOML schema with sorted tables and strict SemVer precedence. Repeated resolution against identical inputs yields byte-for-byte identical lockfiles without timestamps or machine-dependent nonces.

### 3. Out-of-Source Compilation
Registry package sources are extracted once into a content-addressed source cache and compiled in isolated build directories (`.crux/builds/<pkg>/<version>/<fingerprint>/`). Local applications are compiled from `src/` into isolated profile directories (`.crux/build/debug/` and `.crux/build/release/`) using internal CMake configuration generated under `.crux/build/generated/`. No source tree is modified by build executions, and ordinary applications never require a root-level `CMakeLists.txt`.

### 4. Staged & Atomic Installation
Installation runs `cmake --install` into an isolated staging directory. Crux validates all staged files against symlink escape rules, records an `installed.toml` inventory, and atomically renames the directory into `.crux/installed/`.

### 5. One Resolution Pipeline for Direct Sources
Git, archive, and local-path dependencies are normalized into the same `PackageCatalog` entries and `PackageSource` identities used by registry packages. The resolver remains storage-agnostic; an `ActiveCatalog` overlay validates direct sources, locks Git refs to immutable commits, rejects source conflicts/cycles, and feeds the same lockfile, build, install, and CMake integration pipeline. See [Direct Package Sources](/crux/guides/direct-sources/).
