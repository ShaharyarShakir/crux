# M4: Extraction of the CMake Package Build Pipeline

Date: 2026-11-28. Scope: the package dependency build pipeline only. No behavior change intended.

## 1. Existing architecture and why extraction was needed

`CMakeBuildEngine` (`src/build/engine.rs`) held the whole package build in one method chain: fingerprinting, directory layout, cache validation and metadata were interleaved with CMake discovery, argument construction and subprocess execution. `CMakeInstallEngine` mixed `cmake --install` with staging, inventory and publication. `selection.rs` mixed compiler resolution with `CMakeCache.txt` inspection and `-DCMAKE_*` argument construction. A second build system would have required rewriting the lifecycle or copying it.

## 2. Responsibilities before and after

| Responsibility | Before | After |
| --- | --- | --- |
| Fingerprint, build layout, cache validation, metadata, reinstall marker | `engine.rs` | `pipeline.rs` |
| Artifact collection | `engine.rs` | `pipeline.rs`; the adapter says which entries are internal |
| Compiler and flag resolution | `selection.rs` | `selection.rs`; the adapter supplies the build system's own declarations |
| Dependency validation and search path environment | `package_dependencies.rs` | unchanged |
| `-DCMAKE_MODULE_PATH` | `package_dependencies.rs` | `systems/cmake.rs` |
| Tool version and generator detection | `engine.rs` | `systems/cmake.rs` |
| Configure and build commands, failure summaries | `engine.rs` | `systems/cmake.rs` |
| Compiler `-D` arguments, build tree inspection and reset | `selection.rs` | `systems/cmake.rs` |
| `CMakeLists.txt` and configured-tree checks | `engine.rs`, `install/engine.rs` | `systems/cmake.rs` |
| `cmake --install`, `DESTDIR` handling, `install.log` | `install/engine.rs` | `systems/cmake.rs` |
| Staging, inventory, publication, current record | `install/engine.rs` | unchanged |
| Build system selection | a string comparison in the CLI | `BuildSystemKind` |

## 3. Final module organization

```text
src/build/
├── engine.rs                public types: BuildRequest, BuildResult, BuildEngine, CMakeBuildEngine
├── pipeline.rs              shared build lifecycle (new)
├── selection.rs             compiler and flag resolution
├── package_dependencies.rs  dependency exposure
└── systems/                 (new)
    ├── mod.rs               BuildSystemKind, PackageBuildSystem dispatch, BuildStep
    └── cmake.rs             CMake adapter
src/install/engine.rs        shared install lifecycle
```

All new items are crate-private. No public type, method or signature changed.

## 4. Build-system selection and dispatch

`BuildSystemKind::from_declared` maps a definition's `[build] system` value:

- no `[build]` section, or `"cmake"`: `Cmake`;
- anything else (including `"CMake"`, `"meson"`, `"catalog-only"`): rejected, with the same diagnostic as before. There is no fallback.

`require_supported_build_system` now returns the kind. The commands pass it to `build_with_dependencies`, `is_fresh_with_dependencies` and `install_with_dependencies`, and each engine turns it into a `PackageBuildSystem`, the enum that dispatches to the adapter. The public `BuildEngine::build`, `InstallEngine::install`, `is_fresh` and `report::build_package` select `Cmake`, as their `CMake*` names imply.

The manifest, lockfile and registry formats are unchanged.

## 5. Why a closed enum rather than a trait

There is one implementation, so a trait's interface would be CMake's steps with trait syntax around them, and nothing shows which of them another system shares. With an enum, adding a variant makes the compiler list every decision the new system has to make. `PackageBuildSystem` has nine operations and none is optional. A trait can replace it later without touching the lifecycle.

Two enums are used on purpose: `BuildSystemKind` is data chosen per package, and `PackageBuildSystem` borrows the tool configuration held by an engine.

## 6. How the adapter integrates

The pipeline calls, in order: `check_source`, `detect`, then (after computing the fingerprint and layout) `conflicts_with` and `is_configured` for the cache, `tree_needs_reset` during preparation, `configure_and_build`, and `is_build_internal` while collecting artifacts. The install engine calls `check_installable` and `install_into`. The adapter does not compute fingerprints, read or write build or install metadata, or publish installations.

## 7. Application builds

`app.rs`, `adhoc.rs`, `generator.rs`, `dependency.rs`, `discovery.rs`, `sanitizer.rs`, `toolchain.rs` and `cli/commands/run.rs` were not edited in M4. They do not use the new modules. The `app_build`, `run` and `cmake_integration` suites pass.

## 8. Fingerprint and metadata compatibility

- `fingerprint.rs` and `metadata.rs` were not edited. No format bump.
- The M0 golden digest tests and metadata compatibility tests pass.
- A differential run compared the pre-M4 and post-M4 `crux` binaries on the same path-dependency chain with a fake `cmake`: `lock`, `install` (twice), `build`, `build --force`, `install --profile debug`, `install --force`, with `CFLAGS`, `LDFLAGS` and `CMAKE_PREFIX_PATH` set. The two runs were identical in:
  - all 41 CMake invocations: argument vectors and the `CC`, `CXX`, `*FLAGS`, `CMAKE_*`, `PKG_CONFIG_PATH`, `DESTDIR` and `PATH` environment;
  - CLI output (timings normalized);
  - build and install file layout, including fingerprint directory names;
  - build metadata (without `duration_millis`), install metadata and current records;
  - build and install logs (staging directory names normalized).

The differential run did not exercise failing steps, absolute install destinations or stale-tree recovery; those rest on the existing tests. The script was temporary and is not part of the repository.

## 9. Tests and verification

Added:

- `tests/package_build_system.rs` (3): an omitted `[build]` section and explicit `cmake` are built and installed by CMake; `meson` fails in `install` and `build` without creating a build directory, although the package has a `CMakeLists.txt`.
- `systems/mod.rs` (1): the selection table.
- `systems/cmake.rs` (1): `CMAKE_MODULE_PATH` ordering, the empty case and the unrepresentable path.

Adapted: the inventory test in `package_dependencies.rs` now asserts the collected module directories; its argument-formatting assertions moved to the `cmake.rs` test. No other test changed.

Results, inside `nix develop --command`:

| Command | Result |
| --- | --- |
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo check` | pass |
| `cargo test --no-fail-fast`, before M4 | 765 passed, 2 failed |
| `cargo test --no-fail-fast`, after M4 | 770 passed, 2 failed |

The two failures are the same before and after, both in `tests/build_settings.rs`: `address_and_undefined_sanitizers_instrument_the_binary` and `unchanged_settings_reuse_the_build_tree_and_changes_invalidate_it`. The sanitizer probe fails to link under Nix GCC 16.2.0 (`undefined reference to __pointer_chk_guard@GLIBC_PRIVATE`).

Focused runs (`cargo test --test <name>`), all passing: `build` 12, `install` 15, `m0_build_commands` 3, `m0_fingerprint_golden` 2, `m0_metadata_compat` 5, `toolchain_cache` 23, `current_installation` 16, `dependency_exposure` 13, `app_build` 21, `run` 12, `package_build_system` 3, `official_registry` 6, `cmake_integration` 23, and `--lib` 331.

## 10. Limitations and extension points

Still CMake-shaped, because changing them would change public API, formats or fingerprints:

- `BuildConfiguration` (`cmake_args`, `generator`), `ToolchainInfo::cmake_version`, `BuildProfile::as_cmake_build_type`.
- `BuildError` and `InstallError` variants such as `MissingCMakeLists` and `CMakeExecutionFailed`.
- The public names `CMakeBuildEngine` and `CMakeInstallEngine`.
- `selection.rs` and `package_dependencies.rs` still name `CMAKE_PREFIX_PATH` (a fingerprint label), and the invalid-compiler message in `selection.rs` says "CMake argument".
- Registry parsing validates `cmake_args` whatever the declared system.

A second build system would need:

- a variant in both enums and an adapter module;
- **the system kind in the fingerprint**, added only for non-CMake kinds so existing digests stay valid. Nothing records the kind today, in fingerprints or metadata;
- a decision on how the kind travels from build to install. The caller passes it to both engines, and `BuildResult` does not carry it;
- engine ownership of its tool settings, and neutral error variants.

Preserved quirk: `extra_env` is applied to the configure step but not the build step.

`CHANGELOG.md` is unchanged because nothing user-visible changed. `AGENTS.md` section 3 does not list the new files and was not edited. The task referred to `docs/ARCHITECTURE.md`; the file is `docs/reference/architecture.md`.

## 11. Suggested titles

- Commit: `refactor(build): extract the CMake package adapter from the build lifecycle`
- PR: `Extract the CMake package build pipeline behind a build-system boundary`

## 12. PR description

> Separates the package build and install lifecycle from CMake execution. The lifecycle lives in `build/pipeline.rs` and `install/engine.rs`; CMake discovery, commands, tree inspection and install live in `build/systems/cmake.rs`. Package definitions select a closed `BuildSystemKind`; CMake remains the only one and unknown values are still rejected.
>
> No behavior, format, fingerprint or public API change. Old and new binaries produced identical CMake invocations and metadata on a two-package chain. Application builds are untouched.
>
> Tests: 770 passed; the 2 known Nix GCC sanitizer-link failures in `build_settings` remain.
>
> Depends on M0–M3.
