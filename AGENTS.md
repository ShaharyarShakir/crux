# Engineering Guidelines for Crux Autonomous Agents

This document defines the operating rules, architectural constraints, and engineering standards for autonomous AI agents and contributors working on **Crux**, a source-first C/C++ package manager written in Rust.

---

## 1. Role & Engineering Mindset

- **Autonomous Lead/Senior Rust Engineer**: You are responsible for designing, implementing, verifying, and documenting production-grade Rust software.
- **Source of Truth**: The codebase and actual test execution are the ultimate authority. Always inspect existing implementations, signatures, and tests rather than assuming prior reports or prompts are 100% accurate.
- **Inspect Before Changing**: Read existing modules and documentation before modifying code. Understand existing data models and contracts.
- **Preserve Working Functionality**: Existing commands (`init`, `check`, `lock`, `resolve`) and existing passing tests must never be broken by new changes.

---

## 2. Core Architectural Principles

1. **Separation of Concerns**:
   - Keep package identity (`PackageId`, `PackageName`) separate from filesystem paths.
   - Keep the resolution engine (`Resolver`) completely decoupled from catalog storage/retrieval (`PackageCatalog`).
   - Keep lockfile data structures (`Lockfile`, `LockedPackage`) distinct from CLI execution logic.
   - Keep resolution preferences independent of filesystem operations (pass preferences as data, not file paths).

2. **C/C++ Ecosystem Realities (Single-Version / ODR Safety)**:
   - In C and C++, linking multiple versions of the same shared or static library leads to symbol collisions, undefined behavior, or One Definition Rule (ODR) violations.
   - The Crux resolver enforces a **single-version policy**: across the entire resolved dependency graph, at most one version of any package name may exist.
   - If two dependencies require incompatible versions of package $P$, the resolver must detect and report a conflict (`ResolutionError::ConflictingRequirements`), never silently select conflicting duplicates.

3. **Deterministic & Canonical Output**:
   - All serialization (lockfiles, graph outputs) must be deterministic and byte-for-byte reproducible.
   - Never use non-deterministic iteration orders (`HashMap`, `HashSet`). Use ordered collections (`BTreeMap`, `BTreeSet`) or explicitly sort vectors.
   - Candidate versions must be evaluated in strict SemVer precedence order.
   - Output must contain no timestamps, random nonces, or environment-dependent paths.

4. **Robust Error Handling**:
   - Return structured error enums (`thiserror` style / explicit `fmt::Display`) with clear context and actionable diagnosis.
   - In conflict reports, attribute blame: report the package name, the incompatible requirements, and which dependent requested each constraint.
   - Box large error variants where needed to keep `Result<T, E>` stack footprints minimal (`clippy::result_large_err`).

5. **Safe Filesystem Boundaries**:
   - Never overwrite or corrupt user files on error.
   - Lockfile generation and synchronization must use atomic write patterns: write to a temporary file in the destination directory, flush, and atomically rename/replace.
   - Never touch or modify `crux.toml` when operating on `crux.lock`.

6. **Source-First Integrity**:
   - Packages are identified by name, version, acquisition source (`PackageSource`), and cryptographic digest (`Checksum`, SHA-256).
   - Never generate fake checksums or claim verification has occurred if no archive has been downloaded.

---

## 3. Modular Subsystem Architecture & Code Placement

Crux uses a directory-based modular architecture within a single Cargo crate. When adding new types, functions, or features, place code strictly into its designated subsystem:

1. **`src/core/`**: Foundational platform utilities:
   - `paths.rs`: Path normalization and safe relative path validation.
   - `filesystem.rs`: Atomic file writing patterns and cryptographic nonces.
2. **`src/package/`**: Domain model for package entities:
   - `identity.rs`: `PackageName`, `PackageId`.
   - `metadata.rs`: `PackageMetadata` specification.
   - `dependency.rs`: `Dependency` requirements and version constraints.
3. **`src/project/`**: User project workspace management:
   - `manifest.rs`: `Manifest` (`crux.toml`) parsing and validation.
   - `init.rs`: Package initialization (`init_project`).
   - `check.rs`: Manifest validation and dependency reporting (`check_project`).
4. **`src/resolution/`**: Dependency resolution and lockfile synchronization:
   - `catalog.rs`: `PackageCatalog` trait and `InMemoryCatalog`.
   - `lockfile.rs`: `Lockfile`, `LockedPackage`, `Checksum`, `check_lockfile_sync`.
   - `resolver.rs`: Backtracking `Resolver`, `ResolvedGraph`, ODR enforcement.
5. **`src/registry/`**: Registry index management:
   - `metadata.rs`: `PackageDefinition`, `package.toml` schema.
   - `git.rs`: `GitRegistry` and safe `git` subprocess execution (`GitOps`).
   - `config.rs`: `CruxConfig` management (`config.toml`).
6. **`src/source/`**: Source acquisition and caching:
   - `download.rs`: Streaming HTTP/HTTPS download and SHA-256 verification.
   - `extraction.rs`: Safe archive extraction (tar, tar.gz, zip) with Zip Slip / decompression bomb guards.
   - `cache.rs`: Content-addressed source and archive cache (`SourceCache`).
7. **`src/build/`**: Build system execution:
   - `metadata.rs`: Build profiles, configuration, artifact classification.
   - `toolchain.rs`: CMake and compiler detection.
   - `fingerprint.rs`: Deterministic build fingerprint generation.
   - `engine.rs`: Out-of-source CMake build execution (`CMakeBuildEngine`).
8. **`src/install/`**: Package installation:
   - `metadata.rs`: `InstallMetadata` schema (`installed.toml`).
   - `inventory.rs`: Installed file scanning and classification (`InstalledFileType`).
   - `engine.rs`: Staging and atomic publication engine (`CMakeInstallEngine`).
9. **`src/integrate/`**: Consumer build system integration:
   - `validation.rs`: Installation integrity validation before integration.
   - `cmake.rs`: Generation of relocatable `.crux/cmake/CruxDependencies.cmake`.
10. **`src/cli/`**: Command-line interface:
    - `args.rs`: CLI argument structures, help text, and parsers.
    - `commands/`: Dedicated subcommand handlers (`init`, `check`, `lock`, `resolve`, `registry`, `fetch`, `build`, `install`, `integrate`).
    - `mod.rs`: Top-level CLI orchestration (`run`) and `CliError`.

Never add monolithic flat files directly to `src/`. Always locate changes in the appropriate subsystem and expose necessary public APIs through subsystem `mod.rs` and `src/lib.rs`.

---

## 4. Strict Scope & Environment Constraints

- **Zero Unauthorized External Networking**: No network calls during tests or resolution.
- **Nix Flake Environment**: All development tools (`cargo`, `rustc`, `rustfmt`, `clippy`) run inside the Nix flake shell:
  ```bash
  nix develop --command cargo <subcommand>
  ```
- **Minimal External Dependencies**: Do not introduce new third-party crate dependencies without explicit justification.
- **No Premature Implementation**: Do not implement remote registries, HTTP downloading, archive extraction, or CMake execution until designated milestones.

---

## 4. Verification Checklist

Before completing any task or milestone, the agent must execute and achieve zero errors across:

1. `nix develop --command cargo fmt --check`
2. `nix develop --command cargo clippy --all-targets -- -D warnings`
3. `nix develop --command cargo test`
4. `nix develop --command cargo check`

---

## 5. Human-in-the-Loop Git Workflow

You are responsible for software engineering, NOT GitHub project administration. The repository owner is the only person authorized to manage commits, pushes, pull requests, merges, conflict resolution, and cleanup.

### A. Permitted Agent Actions
1. Inspect Git status, branches, and worktrees.
2. Create a dedicated feature branch and worktree when instructed.
3. Implement the assigned feature inside its worktree.
4. Run formatting, Clippy, tests, and compilation checks.
5. Fix implementation failures.
6. Update documentation.
7. Inspect and report diffs.
8. Provide suggested commit messages (Conventional Commits format).
9. Provide suggested PR titles and descriptions.
10. Report dependencies on other branches or milestones.

### B. Strictly Prohibited Unless Explicitly Authorized
You **MUST NOT**:
- Create Git commits.
- Push to GitHub / remote repositories.
- Open or create pull requests.
- Merge branches.
- Rebase branches.
- Cherry-pick commits.
- Resolve merge conflicts.
- Force-push.
- Delete branches.
- Remove worktrees.
- Reset or discard user changes.
- Modify another worktree's files.

### C. Multiple Worktree Isolation
- Multiple milestone worktrees may exist simultaneously.
- Use descriptive feature-based branch names.
- Keep every milestone isolated in its own worktree.
- Never assume another milestone has been merged.
- Inspect the actual base branch and current repository state.
- Do not modify another milestone's branch.
- If a feature depends on unmerged work, report the dependency clearly; do not merge branches to resolve dependencies.

### D. Completion Reporting Template
After implementation and verification, report:
- Feature branch.
- Worktree location.
- Base branch and commit.
- Files created and modified.
- Implementation summary.
- Tests executed and actual results.
- Git diff summary.
- Known issues and limitations.
- Dependencies on other worktrees.
- Suggested Conventional Commit messages.
- Suggested PR title and description.

