---
title: Project Roadmap
description: Implementation milestones and future plans for Crux.
---

> **Milestone numbering note**: This user-facing roadmap keeps the historical implementation milestone numbers recorded when each feature was delivered. The repository [ROADMAP.md](https://github.com/ShaharyarShakir/crux/blob/main/ROADMAP.md) uses a planning-oriented numbering for some later milestones; where the two differ, the repository roadmap is authoritative. M24 was released in v0.2.0 and hardened by the v0.2.1 hotfix (M25).

## Completed Milestones

- **M1**: Project Initialization & CLI Scaffold
- **M2**: Manifest Format & Validation (`crux.toml`, `crux check`)
- **M3**: Workspace & Build Scaffold
- **M4**: Package Metadata & SemVer 2.0.0
- **M5**: Lockfile Model & SHA-256 Integrity
- **M6**: Dependency Specification (caret, tilde, comparator ranges)
- **M7**: Dependency Resolution Engine with ODR Safety (`crux resolve`)
- **M8**: Lockfile Generation & Sync (`crux lock`)
- **M9**: Git Registry Index (`crux registry`) & Seed Registry (fmt, zlib, Catch2)
- **M10**: Source Acquisition, Streaming Download, SHA-256 Verification & Safe Extraction (`crux fetch`)
- **M11**: Isolated CMake Build Engine & Deterministic Fingerprints (`crux build`)
- **M12**: Package Installation Engine & File Inventory (`crux install`)
- **M13**: CMake Consumer Integration (`crux integrate cmake`)
- **M14**: Modular Architecture Refactoring
- **M15**: Release Engineering, Nix Packaging, and CI Pipeline
- **M16**: First Public Release Preparation (v0.1.0)
- **M17**: Standardized Project Initialization (`crux init --language c|cpp|other`)
- **M18**: Project Configuration & Toolchain Detection (`CC`/`CXX`, standards)
- **M19**: Application Build Engine (`crux build` for local C/C++ applications)
- **M20**: Application Execution (`crux run`, Debug/Release profiles, dependency runtime paths)
- **M21**: Dependency Management & Application Integration (`crux add`, `crux remove`, automatic install/link)
- **M22**: Official Package Ecosystem Expansion (50 packages, 15 verified, five reference applications)
- **M23**: Existing Project Import (`crux import`, conservative source discovery, `.crux/import.toml`)
- **M24**: Direct Package Sources (Git, SHA-256 archive, and local path sources) and v0.2.0 release (released in v0.2.0, hardened in v0.2.1)
- **M25**: v0.2.1 Security & Documentation Hotfix (direct-source symlink-escape validation, HTTPS-only archives, Git `rev` validation, `--output` CLI support, and documentation alignment)

---

## Upcoming Milestones

Subsequent milestones are planned by the project maintainers. Crux does not document unreleased commands as implemented.
