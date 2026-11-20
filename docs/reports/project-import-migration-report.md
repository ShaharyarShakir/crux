# Crux Milestone 23 Completion Report: Existing Project Import & Migration

**Milestone**: M23 — Import Existing C/C++ Projects  
**Crux Version**: v0.2.0-dev  
**Date**: 2026-11-20  
**Status**: Completed & Verified  

---

## 1. Summary

Milestone 23 adds `crux import`, a safe, non-destructive path for adopting existing C and C++ projects. The command inspects a project directory, reports detected facts separately from assumptions, and writes a conservative `crux.toml` using the existing manifest schema.

The implementation deliberately avoids introducing a second build engine or translating arbitrary build graphs. Existing sources, headers, CMake files, and Makefiles remain in place. Source roots and include directories observed during discovery are stored in Crux-managed `.crux/import.toml`, and the existing application build engine compiles those sources through Crux's internal CMake project.

Key capabilities delivered:

- `crux import`, `crux import --path <DIR>`, `crux import --path .`, `crux import --dry-run`, and `crux import --force`.
- Deterministic, read-only discovery of build systems, conventional source/include/test directories, root-level C/C++ compilation units, language, likely entry points, multiple targets, and project-name hints.
- Conservative manifest generation with no guessed dependencies and no invented manifest keys.
- Strict preservation of user-authored files, including an absolute refusal to overwrite an existing `crux.toml`.
- Build-engine reuse through validated import metadata for non-canonical source layouts and project-relative include directories.
- Explicit reporting of detected facts, assumptions, limitations, created files, preserved files, and actionable next steps.
- Makefile detection without executing or translating Make recipes.
- CMake configuration preservation without creating a competing root-level CMake project.
- Comprehensive offline tests and updated user/reference documentation.

---

## 2. Problem Statement

Before M23, Crux projects were expected to follow the canonical `src/` layout and a `crux.toml` created by `crux init`. Existing projects often use `source/`, `app/`, root-level sources, Makefiles, or existing CMake configurations. Importing such projects needed to satisfy several constraints:

1. Never destroy, rename, or edit user sources or build configuration.
2. Do not execute untrusted project build scripts during discovery.
3. Do not invent manifest keys, dependencies, or build semantics.
4. Preserve deterministic, offline behavior.
5. Reuse the existing application build engine rather than create a parallel import-specific compiler path.
6. Remain conservative when discovery cannot identify a reliable build intent.

The design separates observed facts from user-authored configuration and keeps the manifest schema unchanged.

---

## 3. Design Decisions

### 3.1 Conservative manifest generation

`crux import` generates only fields already supported by `Manifest`:

```toml
[project]
name = "legacy-app"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
```

- The name comes from a simple CMake `project()` declaration when found, otherwise from the project directory name.
- Names are sanitized to valid Crux package names while preserving underscores.
- The version defaults to `0.1.0`.
- Language is inferred from discovered compilation units.
- The standard is the existing Crux default for the detected language.
- Dependencies are never guessed; the table remains empty.

No manifest keys were added for legacy source roots or include directories.

### 3.2 Crux-managed import metadata

Legacy layouts are recorded in `.crux/import.toml`:

```toml
version = 1
source_roots = ["app", "source"]
include_dirs = ["include"]
```

This is a versioned, Crux-managed file, not user-facing package configuration. It allows `crux build` and `crux run` to compile existing sources without relocating them.

All paths are validated as safe relative paths. Symlinks resolving outside the project root are rejected, and generated CMake uses project-relative `${CRUX_PROJECT_ROOT}/...` paths so output remains relocatable.

### 3.3 Canonical behavior preserved

Projects without `.crux/import.toml` retain existing behavior:

- If `src/` exists, only `src/` is scanned.
- If no `src/` exists, conventional `source/`, `app/`, or root-level layouts are considered.
- Existing `crux init`, `check`, `build`, and `run` workflows are unchanged.

Import metadata source roots override conventional fallback behavior only after validation.

### 3.4 Strict `--force` semantics

Low-confidence discoveries fail by default:

- No detected `main()` entry point.
- Multiple likely executable entry points.

`--force` permits these cases and records their limitations in the report. It may also explicitly refresh `.crux/import.toml` when discovery facts change, with refreshed files listed under `Updated (--force)`.

`--force` never permits overwriting `crux.toml`. If a manifest exists, import reports that the project is already initialized and preserves the file.

Hard blockers remain blocking even with `--force`:

- No C/C++ compilation units.
- Mixed C and C++ sources, because the current application build engine supports one language per target.

### 3.5 Avoiding build-script execution

Discovery performs only filesystem reads and simple text parsing. It does not run CMake, Make, compiler commands, scripts, or network operations. Existing build files are reported and preserved, not consumed or translated.

---

## 4. Implementation

### 4.1 Project discovery

`src/project/discovery.rs` provides `discover_project`:

- Canonicalizes and validates the target directory.
- Sorts directory traversal deterministically.
- Uses ordered collections for discovered files, directories, targets, and warnings.
- Detects `CMakeLists.txt`, `Makefile`, `makefile`, and `GNUmakefile`.
- Parses simple top-level CMake `project()` declarations for a name hint.
- Finds conventional source roots (`src/`, `source/`, `app/`), include roots (`include/`, `inc/`, `headers/`), root-level sources/headers, and test directories.
- Detects likely entry points from `main.*` names and simple top-level `int main` patterns.
- Skips `.git`, `.crux`, `build`, `out`, `dist`, `node_modules`, `target`, `vendor`, hidden paths, and symlinks escaping the project root.
- Produces deterministic detected facts and warnings.

### 4.2 Import core

`src/project/import.rs` provides:

- `ImportOptions`
- `ImportError`
- `ImportSummary`
- `ImportMetadata`
- `import_project`
- `format_import_report`

Safety properties:

- Rejects invalid directories before writing anything.
- Detects existing `crux.toml` before discovery and always preserves it.
- Uses create-only writes for new files.
- Uses atomic replacement only for Crux-managed metadata refreshed with explicit `--force`.
- Validates all metadata paths before use.
- Emits deterministic reports.

### 4.3 Build engine integration

`src/build/discovery.rs` was extended with `discover_sources_with_roots`:

- Validates metadata source roots against the canonical project root.
- Reuses the existing language, extension, header, symlink, and deterministic-order checks.
- Applies the same ignored-directory policy as project discovery.
- Preserves canonical `src/` behavior when no metadata roots are present.

`src/build/app.rs` now:

- Loads `.crux/import.toml` during `AppBuildModel` preparation.
- Resolves validated project-relative include directories.
- Passes metadata source roots to build discovery.
- Maps new discovery errors to actionable `BuildError::InvalidConfiguration` values.

`src/build/generator.rs` emits imported include directories into the internal CMake target as project-relative paths and rejects unsafe relative values.

`src/build/adhoc.rs` was updated for the extended source model; ad-hoc `crux run --bin/--sources` behavior remains unchanged.

### 4.4 CLI surface

`src/cli/args.rs` adds:

- `import` in top-level help.
- `IMPORT_HELP_MESSAGE`.
- `ImportArgs`.
- `Action::ImportHelp` and `Action::Import`.
- `parse_import_args` with `-p/--path`, `--path=...`, `--dry-run`, `--force`, and `-h/--help`.

`src/cli/mod.rs` adds `CliError::Import`, top-level dispatch, help output, and CLI parsing tests.

`src/cli/commands/import.rs` resolves relative paths against the caller's working directory, calls the import core, prints the report to stdout, and writes diagnostics to stderr on failure.

---

## 5. Supported Layouts

| Layout | Import behavior |
|---|---|
| `src/` with C or C++ sources | Canonical; records `src` as source root |
| `source/` or `app/` | Records discovered roots; build engine compiles from metadata |
| Multiple source directories | Records every directory containing compilation units |
| Root-level sources | Records `.` as source root; ignored directories are excluded |
| Existing `CMakeLists.txt` | Detected and preserved; simple `project()` name used when available |
| `Makefile`, `makefile`, `GNUmakefile` | Detected and preserved; recipes are never executed or translated |
| Include directories (`include/`, `inc/`, `headers/`) | Recorded for relocatable internal CMake include paths |
| Test directories (`test/`, `tests/`) | Reported as detected facts |
| No recognized build system | Importable when C/C++ compilation units are present |

---

## 6. Compatibility and Limitations

- Existing CMake configurations are preserved but not consumed. `crux build` compiles detected sources using Crux's internal CMake project; `add_executable()`, custom commands, compile definitions, and linked libraries are not translated.
- Makefile builds are detected only. Make is not executed or translated, and generated sources or custom flags require manual adaptation.
- Mixed C/C++ projects are rejected because the current build engine models one language per application.
- Multiple executable targets are accepted only with `--force`, and Crux still generates a single executable target.
- Projects without a detected `main()` may fail to link; `--force` allows import but reports this limitation.
- Dependencies are never inferred. Vendored or build-script-provided dependencies must be declared manually.
- Import records include directories only for direct project-relative paths; arbitrary preprocessor definitions and system include paths are not translated.
- `.crux/import.toml` is Crux-managed metadata. Manual edits must use safe relative paths and version `1`.
- Repository hygiene: the existing broad `build/` rule in `.gitignore` also matches the source directory `src/build/`. As a result, the build-engine files modified for M23 are present in the working tree but are ignored by Git. This is a pre-existing repository configuration issue and was not changed in M23; the repository owner must address it before committing the subsystem.

---

## 7. Tests

### 7.1 New offline integration suite

`tests/import.rs` contains 15 tests:

1. Basic C project: import, check, build, run, manifest/metadata structure, generated CMake content, and preserved sources.
2. C++ project with `include/`: import and build with a detected project-relative include directory.
3. CMake project: project-name detection, build-system reporting, and byte-for-byte preservation of `CMakeLists.txt`.
4. Makefile detection: `Makefile`, `makefile`, and `GNUmakefile`; verifies recipes are never executed and files are preserved.
5. Multiple source directories: `app/` plus `source/`, metadata roots, and generated CMake sources.
6. Root-level sources with ignored directories: `build`, `out`, `dist`, `node_modules`, and `.git` are excluded.
7. Multiple executable targets: rejected by default with actionable diagnostics and accepted with `--force`.
8. Missing entry point: rejected by default and accepted with `--force` with a linker warning.
9. Invalid project directory: missing and non-directory paths produce actionable errors.
10. Existing manifest: preserved under normal, `--force`, and `--dry-run --force` invocations.
11. Dry run and `--path`: target directory selection and no writes.
12. Forced metadata refresh: only `.crux/import.toml` is updated and clearly reported.
13. Symlink escape protection: symlinks outside the project root are skipped and reported.
14. Unsafe metadata: build rejects traversal paths in `.crux/import.toml`.
15. Regression: `crux init`, `build`, and `run` remain functional.

All tests are offline and use isolated temporary directories.

### 7.2 Existing suite

The full suite remains green. No existing command behavior was changed incompatibly.

---

## 8. Verification Results

The following commands were executed in the Nix development shell from the repository root:

| Command | Result |
|---|---|
| `nix develop --command cargo fmt --all` | Passed |
| `nix develop --command cargo fmt --all -- --check` | Passed |
| `nix develop --command cargo clippy --all-targets --all-features -- -D warnings` | Passed with no warnings |
| `nix develop --command cargo check` | Passed |
| `nix develop --command cargo test --all` | Passed: 492 tests total |
| `nix develop --command cargo build --release` | Passed |

Test breakdown from `cargo test --all`:

- Library tests: 211 passed.
- Integration tests: 281 passed.
- Doc tests: 0 run.

The target suite includes 15 new `tests/import.rs` integration tests. All passed.

---

## 9. Files Created and Modified

### Created

- `src/project/discovery.rs`
- `src/project/import.rs`
- `src/cli/commands/import.rs`
- `tests/import.rs`
- `docs/guides/import-existing-projects.md`
- `docs/reports/project-import-migration-report.md`

### Modified

- `src/build/discovery.rs`
- `src/build/app.rs`
- `src/build/generator.rs`
- `src/build/adhoc.rs`
- `src/build/mod.rs`
- `src/lib.rs`
- `src/project/mod.rs`
- `src/cli/args.rs`
- `src/cli/mod.rs`
- `src/cli/commands/mod.rs`
- `docs/README.md`
- `docs/reference/cli-reference.md`
- `docs/reference/architecture.md`

---

## 10. Security and Integrity Review

- No network access is introduced by import or discovery.
- No project build scripts are executed.
- Existing files are never deleted or modified by normal import.
- New files use create-only writes; an existing manifest is treated as a hard stop.
- Metadata replacement is restricted to explicit `--force` and uses atomic writes.
- All metadata paths are validated against traversal, absolute paths, prefixes, and symlink escapes.
- Generated CMake paths remain project-relative and deterministic.
- Ordered collections and sorted traversal preserve byte-for-byte deterministic reports and output.
- No checksums or verification claims are generated.

---

## 11. Follow-Up Work

M23 intentionally stops short of build-graph translation. Possible future milestones may consider:

- Multiple executable targets and target-specific source selection.
- Translation of a limited subset of CMake compile definitions and include paths.
- Explicit user-declared source roots or include paths in a future manifest schema revision.
- Dedicated make-generated-source discovery without executing recipes.
- Richer language/standard inference from existing compiler flags.

These items are not required for M23 and were not implemented.

---

## 12. Conclusion

M23 delivers a safe, deterministic import workflow for existing C/C++ projects. It preserves user files, avoids executing untrusted build scripts, uses the existing manifest schema, and reuses the established application build engine through validated Crux-managed metadata. The complete offline test suite and all required quality gates pass.
