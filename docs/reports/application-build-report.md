# Crux Milestone 19 Completion Report: Application Build Engine

**Milestone**: M19 — Application Build Engine  
**Target Release**: v0.2.0  
**Date**: 2026-11-15  
**Status**: Completed & Verified

---

## 1. Implementation Summary

M19 adds a complete application build engine to Crux. A user can initialize a C or C++ project, add one or more source files under `src/`, and run `crux build` to produce a working executable without writing or maintaining CMake.

The implementation builds on the standardized project structure from M17 and the project configuration/toolchain detection from M18:

- `crux.toml` remains the user-facing project definition.
- Compiler selection and language-standard validation reuse M18's `ProjectToolchain` detection, including `CC`/`CXX` overrides.
- Application sources are discovered recursively under `src/`.
- Crux generates an internal CMake project exclusively under `.crux/build/generated/`.
- Configure and compile steps run out-of-source under `.crux/build/debug/` or `.crux/build/release/`.
- CMake is an internal backend. No root-level `CMakeLists.txt` is created or required.

The build pipeline is split into explicit planning, generation, execution, and reporting stages. The model types (`AppBuildModel`, `AppTarget`, `AppTargetKind`, `AppBuildLayout`, `CMakeInfo`) are reusable and structured for future multi-target and library support without rewriting the engine.

---

## 2. Final Application Build Architecture

### Build pipeline

1. **Planning** — `AppBuildModel::prepare_with_cmake`:
   - Canonicalizes and validates the project directory.
   - Loads `ProjectConfig` from `crux.toml`.
   - Requires a `[project]` manifest for application builds.
   - Resolves and normalizes the language and standard.
   - Detects and validates the compiler through M18.
   - Recursively discovers sources under `src/`.
   - Detects CMake and selects a deterministic generator.
   - Computes the project-local build layout.
   - Writes nothing and performs no compilation.
2. **Generation** — `generate_app_cmakelists`:
   - Produces a deterministic CMake project at `.crux/build/generated/CMakeLists.txt`.
   - Escapes all injected paths and validates source confinement.
3. **Execution** — `AppBuildEngine::execute`:
   - Creates project-local profile/log directories.
   - Applies `--force` by discarding the selected profile tree.
   - Invalidates stale CMake caches when the compiler or generator changed.
   - Configures with CMake.
   - Compiles with CMake.
   - Validates that the expected executable exists.
   - Returns a structured `AppBuildResult`.
4. **Reporting** — the CLI prints project, language, standard, configuration, compiler, CMake, generator, source count, executable path, duration, and failure log paths.

### Module boundaries

- `src/build/discovery.rs` — source discovery only.
- `src/build/generator.rs` — CMake text generation only.
- `src/build/app.rs` — build model, layout, planning, execution, and result reporting.
- `src/core/process.rs` — safe timed subprocess execution.
- `src/cli/commands/build.rs` — dispatch and presentation.

No CMake or compiler invocation is performed during source discovery or CMake generation.

---

## 3. Source Discovery Rules

Application inputs are limited to the project's `src/` directory.

| Project language | Accepted compilation units |
|---|---|
| C | `.c` |
| C++ | `.cpp`, `.cc`, `.cxx` |

Rules enforced by `discover_sources`:

- Discovery is recursive, including nested directories such as `src/modules/` and `src/utils/`.
- Extensions are matched case-insensitively (for example `.C` and `.CPP`).
- Headers (`.h`, `.hpp`, `.hxx`, `.hh`, `.inl`, `.ipp`) are ignored as independent units.
- Files without extensions are ignored.
- Hidden files and hidden directories (names beginning with `.`) are skipped.
- Discovered paths are sorted lexicographically for deterministic output.
- A missing `src/` directory produces an actionable diagnostic.
- An empty source set produces an actionable diagnostic.
- A C project containing C++ sources, or a C++ project containing C sources, is rejected with a language-mismatch diagnostic. Mixed C/C++ application sources are never silently compiled.
- Known foreign source extensions such as `.rs`, `.go`, `.py`, `.java`, `.m`, `.mm`, `.swift`, `.cs`, `.rb`, `.asm`, and `.s` are rejected with an unsupported-extension diagnostic.
- Symlinked files and directories must resolve inside the canonical `src/` tree. Escapes are rejected.
- Symlink cycles are bounded using a canonical visited-directory set, so traversal terminates.
- Language `other` bypasses C/C++ discovery and produces a clear unsupported-language error.

`DiscoveredSources` exposes the source root and the ordered file list for reuse by future build features.

---

## 4. CMake Generation Design

Generated files live only under:

```text
.crux/build/generated/CMakeLists.txt
```

Design properties:

- **CMake version**: Selected from the language standard: 3.20 by default, 3.21 for C23, and 3.25 for C++26.
- **Language declaration**: `LANGUAGES C` or `LANGUAGES CXX`.
- **Standard enforcement**:
  - `CMAKE_<LANG>_STANDARD` set to the configured standard.
  - `CMAKE_<LANG>_STANDARD_REQUIRED ON`.
  - `CMAKE_<LANG>_EXTENSIONS OFF`.
- **Source inclusion**: Every discovered source is emitted as a quoted path relative to the project root via `CRUX_PROJECT_ROOT`.
- **Nested sources**: Nested paths such as `src/modules/calc.cpp` are included directly.
- **Output paths**: Runtime output directories are set explicitly for single- and multi-configuration generators.
- **Include directories**: `include/` and `src/` are added only when they exist.
- **Determinism**: The same project model produces byte-for-byte identical CMake output.
- **Atomic writes**: Generated files are written through `write_atomically`; unchanged content is not rewritten.
- **Path escaping**: Backslashes, double quotes, `$`, `;`, newlines, carriage returns, and tabs are escaped.
- **Security**: Source paths must remain under the project root and are rejected if they contain parent/root/prefix components.
- **No root CMake**: Generation never creates or modifies a root-level `CMakeLists.txt`.

The generated header is:

```cmake
# Auto-generated by Crux. Do not edit manually.
```

---

## 5. Build Configuration Behavior

- **Debug** is the default for application builds.
- **Release** is selected with `--release` or `--profile release`.
- **Debug** can also be selected explicitly with `--debug` or `--profile debug`.
- Debug and Release use fully separate build trees:
  - `.crux/build/debug/`
  - `.crux/build/release/`
- Switching profiles does not corrupt the other profile's tree.
- Logs are profile-specific:
  - `.crux/build/logs/configure-debug.log`
  - `.crux/build/logs/configure-release.log`
  - `.crux/build/logs/build-debug.log`
  - `.crux/build/logs/build-release.log`
- Changing the manifest language standard regenerates the CMake project and triggers reconfiguration/recompilation as needed.
- Changing the selected compiler invalidates the cached CMake tree before reconfiguration. The cache comparison checks `CMAKE_C_COMPILER` / `CMAKE_CXX_COMPILER` and `CMAKE_GENERATOR`.
- `--force` discards the selected profile tree and performs a clean configure/build.
- Normal incremental behavior is delegated to CMake/Ninja/Make; no custom incremental cache was added.

---

## 6. Compiler Integration

M18's toolchain detection is reused directly; compiler discovery was not duplicated.

- C projects respect `CC`; C++ projects respect `CXX`.
- Without explicit overrides, C candidates are `gcc` then `clang`; C++ candidates are `g++` then `clang++`.
- Compiler family, version, target triple, and executable path come from `ProjectToolchain`.
- The requested language standard is validated by probing compiler support before building.
- The selected compiler is passed to CMake explicitly:
  - `-DCMAKE_C_COMPILER=<path>` for C.
  - `-DCMAKE_CXX_COMPILER=<path>` for C++.
- The engine never silently switches compilers after a selected toolchain fails.
- CMake availability is verified by executing `cmake --version` before configuration.
- CMake generators are selected deterministically: Ninja when available, otherwise Unix Makefiles. `CMAKE_GENERATOR` environment overrides are not consulted.
- All subprocesses use argument vectors, never shell command strings, with stdin closed and captured stdout/stderr.
- Timeouts:
  - Toolchain/CMake probes: 30 seconds.
  - CMake configure: 10 minutes.
  - CMake build: 30 minutes.
- Failure summaries include trailing diagnostic lines and the full log path.

---

## 7. CLI Behavior

`crux build` dispatch is deterministic:

1. `crux build <package>` builds that registry package.
2. `crux build` with a `[project]` manifest builds the local application.
3. `crux build` with a legacy `[package]` manifest builds locked dependencies, preserving v0.1.0 behavior.
4. `crux build` with only `crux.lock` builds locked dependencies.
5. With neither manifest nor lockfile, a clear error is returned.

Application options:

```bash
crux build
crux build --debug
crux build --release
crux build --verbose
crux build --force
crux build --profile debug
```

Example output:

```text
Crux Application Build

Project:       demo
Language:      C++
Standard:      C++20
Configuration: Debug

Toolchain:
  Compiler:    GCC 15.3.0
  CMake:       CMake 4.1.6
  Generator:   Ninja

Source files:  2

Generating build configuration...
Configuring...
Building...

Build successful!

Executable:
  .crux/build/debug/demo

Finished in 0.40s
```

`--verbose` additionally prints compiler and CMake executable paths, each discovered source path, and captured configure/build output. In failure cases it prints the relevant log.

Package build behavior is preserved:

- `crux build <package>` still acquires, fingerprints, and compiles package sources.
- Package builds default to Release when no profile is specified.
- Existing `--profile` and `--force` behavior is unchanged.

---

## 8. Build Output Layout

```text
project/
├── crux.toml
├── crux.lock
├── src/
│   ├── main.c / main.cpp
│   └── nested/
│       └── helper.c / helper.cpp
└── .crux/
    └── build/
        ├── generated/
        │   └── CMakeLists.txt
        ├── debug/
        │   ├── <target>
        │   └── ...CMake/Ninja state...
        ├── release/
        │   ├── <target>
        │   └── ...CMake/Ninja state...
        └── logs/
            ├── configure-debug.log
            ├── configure-release.log
            ├── build-debug.log
            └── build-release.log
```

Invariants verified by tests and manual checks:

- No build artifacts are placed in the project root.
- No build artifacts are placed inside `src/`.
- Source files are not modified.
- `.crux/` is created automatically.
- Debug and Release artifacts are isolated.
- The executable name is derived from the project name, not hardcoded.

---

## 9. Files Created and Modified

### Created

- `src/core/process.rs` — safe timed subprocess runner.
- `src/build/discovery.rs` — recursive application source discovery.
- `src/build/generator.rs` — internal CMake project generation.
- `src/build/app.rs` — application build model, layout, engine, and result types.
- `tests/app_build.rs` — end-to-end application build integration tests.
- `docs/reports/application-build-report.md` — this report.

### Modified

- `src/core/mod.rs` — export `process` module and `run_with_timeout`.
- `src/build/mod.rs` — export application build modules and types.
- `src/lib.rs` — re-export new build model and generator APIs.
- `src/cli/args.rs` — updated `crux build` help text and dispatch documentation.
- `src/cli/commands/build.rs` — deterministic application/package dispatch, app build output, verbose logs, failure log reporting, duration.
- `tests/build.rs` — updated build help-text assertion.
- `docs/reference/architecture.md` — application build architecture, discovery, generation, and dispatch documentation.
- `docs/reference/cli-reference.md` — `crux build` application/package behavior, options, layout, environment variables.
- `docs/guides/quickstart.md` — application-first quickstart without manual CMake.
- `README.md` — application build workflow, layout, dispatch rule, feature table.
- `crux-site/src/content/docs/guides/getting-started.md` — website getting-started flow.
- `crux-site/src/content/docs/reference/cli.md` — website CLI build/init reference and `CC`/`CXX`.
- `crux-site/src/content/docs/project/roadmap.md` — corrected completed milestones through M19.
- `crux-site/src/content/docs/project/changelog.md` — unreleased v0.2.0 development entries.
- `crux-site/src/content/docs/design/architecture.md` — build subsystem and out-of-source compilation updates.

No new third-party dependencies were added.

---

## 10. Tests Added

### Unit tests

`src/build/discovery.rs`:

- Single and multiple source files.
- Nested directories.
- Deterministic ordering.
- Empty source directory.
- Missing source directory.
- Unsupported extensions.
- C/C++ language mismatch.
- Hidden directories.
- Symlinked file escape.
- Symlinked directory escape.
- Symlink cycle termination.
- Paths containing spaces.
- Uppercase extensions.
- Header and extensionless file filtering.

`src/build/generator.rs`:

- Target-name normalization.
- Standard translation for C and C++.
- Minimum CMake version selection.
- CMake path escaping.
- Valid C generation.
- Valid C++ generation with multiple sources.
- Spaces and special characters.
- Deterministic generation.
- Source-outside-project rejection.
- Invalid target name rejection.

`src/build/app.rs`:

- Target naming and executable file names.
- Debug/Release layout isolation.
- CMake cache value parsing.
- Diagnostic log-tail extraction.

`src/core/process.rs`:

- Captured stdout.
- Missing executable error.
- Timeout termination.

### Integration tests (`tests/app_build.rs`, 21 tests)

- Single-file C application compiles and executes.
- Multi-file C application with nested sources compiles and executes.
- Single-file C++ application compiles and executes.
- Multi-file C++ application with nested sources compiles and executes.
- Debug and Release executables coexist in isolated directories.
- Incremental rebuild picks up modified source content.
- Added source files are discovered and included.
- Removed source files are no longer included.
- Standard change causes reconfiguration to the new standard.
- Verbose output includes compiler/CMake paths, sources, and captured logs.
- `--force` discards the profile tree and rebuilds.
- Build writes only under `.crux/build/` and never modifies sources.
- Mixed C/C++ sources are rejected before generation.
- Missing `src/` produces an actionable diagnostic.
- Missing CMake produces an actionable diagnostic.
- Explicit `CXX` is respected and recorded in the CMake cache.
- Compiler switch (`gcc` → `clang`) reconfigures the CMake cache.
- Language `other` is rejected.
- Missing compiler fails clearly.
- Compilation errors preserve diagnostics and fail the command.
- Paths containing spaces build and execute correctly.
- Legacy `[package]` dispatch still builds locked dependencies instead of the application.

---

## 11. Actual Quality Gate Results

All gates were executed inside the repository's Nix development environment.

| Gate | Command | Result |
|---|---|---|
| Formatting | `nix develop --command cargo fmt --all -- --check` | Passed |
| Lints | `nix develop --command cargo clippy --all-targets --all-features -- -D warnings` | Passed |
| Tests | `nix develop --command cargo test --all` | Passed: 403 tests, 0 failed |
| Compilation check | `nix develop --command cargo check` | Passed |
| Release build | `nix develop --command cargo build --release` | Passed |

Test breakdown:

- 174 unit tests passed.
- 229 integration tests passed across the integration suites, including 21 application build tests.
- 0 doc tests.

No gate result above is claimed without having been executed successfully.

---

## 12. Manual End-to-End Build Results

### Fresh project initialization

```bash
crux init demo-c --language c
crux init demo-cpp --language cpp
```

Generated manifests used `[project]` with `language = "c"` / `"cpp"` and the default standards `c17` / `c++20`.

### Multi-file builds

Each project received two sources, including a nested directory:

```text
demo-c/src/main.c
demo-c/src/math/ops.c
demo-cpp/src/main.cpp
demo-cpp/src/greet/greeter.cpp
```

### Debug and Release

Both projects built successfully in both configurations:

```bash
crux build
crux build --release
```

Observed results:

- `demo-c` Debug and Release executables printed `C result: 42`.
- `demo-cpp` Debug and Release executables printed `C++ greeting: Crux`.
- Debug output: `.crux/build/debug/<project>`.
- Release output: `.crux/build/release/<project>`.
- Source count reported: 2 for each project.
- Detected toolchain: GCC 15.3.0, CMake 4.1.6, Ninja.

### Artifact and source inspection

- The only `CMakeLists.txt` files found were:
  - `demo-c/.crux/build/generated/CMakeLists.txt`
  - `demo-cpp/.crux/build/generated/CMakeLists.txt`
- No root-level `CMakeLists.txt` was created.
- Build outputs and logs were confined to `.crux/build/`.
- Source SHA-256 hashes were identical before and after builds.
- A no-op rebuild completed in 0.06s with no compile or link steps, confirming CMake-driven incremental behavior.

Additional manual checks during development confirmed:

- `c17` → `c11` manifest changes reconfigure and compile with `-std=c11`.
- Switching `CC` from GCC to Clang reconfigures cleanly, and switching back to GCC does the same.
- `--verbose` prints detected paths and full compile commands.
- Generated CMake includes nested source paths correctly.

---

## 13. Backward Compatibility

- `crux build <package>` package builds continue to work with existing acquisition, fingerprinting, caching, logging, and artifact reporting.
- No-argument `crux build` on legacy `[package]` manifests, or with only a `crux.lock`, still builds locked dependencies.
- Existing `--profile <debug|release>` and `--force` package behavior is preserved.
- Package builds still default to Release when no profile is specified.
- Application builds default to Debug, as required for M19.
- Existing `init`, `check`, `lock`, `resolve`, `registry`, `fetch`, `install`, and `integrate` behavior was not intentionally changed.
- Existing tests pass, including the package build integration suite.
- The build help text was updated; the corresponding help assertion was updated to match.

---

## 14. Known Limitations

M19 intentionally implements one executable target per project.

- No multiple application targets.
- No library target management.
- No linking of `crux.lock` dependencies into the local application build yet. Package dependency integration remains at the existing `fetch`/`build`/`install`/`integrate` level.
- No `crux run`, `crux test`, `crux add`, `crux remove`, or `crux import`.
- No custom CMake support or custom build systems.
- No cross-compilation.
- No registry redesign or binary caching.
- No user-defined compiler flags beyond the configured language standard.
- CMake is required on the host; it remains an internal backend but not an optional one.
- The implementation is verified on Linux x86_64. Windows and macOS behavior is not part of M19 verification.
- Symlinks are followed only when they resolve inside `src/`; external source trees are deliberately unsupported.
- Multi-configuration generators have an executable-location fallback, but deterministic single-configuration generators are preferred.
- Crux does not automatically edit a user's `.gitignore`; documentation instructs users to ignore `.crux/`.

---

## 15. Git Operations Confirmation

No state-changing Git operations were performed during M19.

Specifically, no commits, staging, branch creation or switching, worktrees, pushes, pull requests, merges, rebases, cherry-picks, conflict resolution, branch deletion, or Git cleanup were performed. Only read-only inspection commands (`git status`, `git diff`, `git show`) were used. All work was performed in the existing working tree, and pre-existing user changes and untracked files were preserved.

M20 was not started.
