# Crux Build-System Architecture Assessment

**Date**: 2026-11-25
**Baseline**: `main` @ `cdda319` (v0.3.0), clean working tree at assessment time
**Scope**: Assessment only. No source, test, manifest, documentation or website file was changed, and no Git operation was performed. This report is the only file created.

> Conclusions below are drawn from reading the code at the paths cited. Where something was inferred rather than observed, or would need a prototype to confirm, it is marked **[unverified]**.

---

## 1. Executive summary

1. **CMake is wired in at three layers, not one.** The package build engine (`src/build/engine.rs`), the package install engine (`src/install/engine.rs`) and the *application* build (`src/build/app.rs` + `src/build/generator.rs`) all call CMake. Only the first two are in scope for a "dependency build system" abstraction. The application build generates its own internal `CMakeLists.txt` from a link plan and does **not** care how dependencies were built.
2. **The cross-build-system contract already exists, and it is the install tree plus `installed.toml`.** `plan_app_dependencies` (`src/build/dependency.rs`) and `validate_package_installation` (`src/integrate/validation.rs`) consume only the file inventory (`InstalledFile { relative_path, file_type, size_bytes }`). Nothing on the application-link path reads `CMakeCache.txt`, `*.cmake` config files or `.pc` files. A Meson-built package that installs the same `include/` + `lib/` layout links today with no consumer-side change.
3. **`BuildEngine` and `InstallEngine` traits already exist but are not an abstraction.** Each has exactly one implementor, nothing uses `dyn`, and the CLI hard-codes `CMakeBuildEngine` and `CMakeInstallEngine` (`src/cli/report.rs:221` takes `&CMakeBuildEngine` and calls the inherent `is_fresh`). They are also the wrong granularity: they wrap the *whole* pipeline (cache check, metadata, log capture, artifact scan), so a second implementor would duplicate about 60% of `engine.rs`.
4. **The package fingerprint is the real compatibility constraint.** `BuildFingerprint::compute` hashes CMake fields in an unframed, unversioned byte stream, and the digest is also the on-disk directory name for both builds and installs. Any change to its preimage invalidates every existing cache. There is **no golden-digest test** pinning it (`tests/build.rs:125` only checks determinism and divergence). That must be fixed first.
5. **The manifest needs no change. The registry needs a small one.** `[build].system` already exists in `package.toml` and is already the selection key. Absent `[build]` already means CMake. The consuming project's `crux.toml` should not grow a build-system field: the build system is a property of the *package*, not of the application.
6. **Recommendation**: freeze CMake behaviour with golden tests, extract the shared pipeline (cache, logs, metadata, staging, inventory), then introduce a narrow, step-level adapter selected by closed `enum` dispatch. Add Meson as the second adapter. Defer Autotools and custom builds until real packages demand them. The trait is justified once the second adapter lands, not before (§5).

---

## 2. Current architecture and build lifecycle

### 2.1 Entry points and who calls what

| Command | Handler | Path to CMake |
|---|---|---|
| `crux build <pkg>` | `run_package_build` (`src/cli/commands/build.rs`) | `CMakeBuildEngine::new(builds_root)` → `report::build_package` → `BuildEngine::build` |
| `crux build` (legacy / lockfile only) | `run_lockfile_dependencies_build` (same file) | same engine, topologically ordered via `sort_packages_topologically` |
| `crux build` (`[project]`) | `run_app_build` → `ensure_app_dependencies` → `AppBuildEngine` | dependencies via `run_install`; application via `AppBuildEngine::{prepare, execute_with_tee}` |
| `crux install [pkg]` | `run_install` (`src/cli/commands/install.rs`) | per package: `CMakeBuildEngine` then `CMakeInstallEngine` |
| `crux run` | `run_run` (`src/cli/commands/run.rs`) | `AppBuildEngine` (also `run_adhoc` → `prepare_adhoc_build`, which calls `detect_cmake(Path::new("cmake"))` directly, `src/build/adhoc.rs:203`) |
| `crux integrate cmake` | `run_integrate_cmake` | `generate_cmake_dependencies` (`src/integrate/cmake.rs`), consumer-side CMake only |

### 2.2 Package lifecycle (`crux install fmt`, same for `crux build <pkg>` minus the last two steps)

```
1. Resolve        catalog.definition(&PackageId) -> PackageDefinition          [src/registry/metadata.rs]
2. Gate           require_supported_build_system(def, ..)                       [cli/commands/mod.rs:52]
                  rejects build.system != "cmake" (string compare, 4 call sites)
3. Acquire        catalog.acquire_definition / acquire_locked_with_status ->    [src/source/*]
                  AcquiredSource { source_dir, checksum }
4. Configure req  build_configuration_for(def, profile)                         [cli/commands/mod.rs:74]
                  -> BuildConfiguration { profile, cmake_args, .. }
5. Build          CMakeBuildEngine::build(&BuildRequest)                        [build/engine.rs:406]
     a. require <src>/CMakeLists.txt (MissingCMakeLists)
     b. detect_toolchain(): `cmake --version`, ninja-on-PATH => "Ninja" else "Unix Makefiles",
                           `gcc|g++ --version` first line (or config/engine overrides)
     c. BuildFingerprint::compute(id, source_checksum, toolchain, config)
     d. dir = <.crux/builds>/<name>/<version>/<fp-hex>/{build,logs,metadata.toml}
     e. cached_result(): metadata.toml parses, success, fingerprint equal, every recorded
        artifact exists with the recorded size  => return reused_cache = true
     f. configure: cmake -S <src> -B <build> -DCMAKE_BUILD_TYPE=.. -G <gen> [-DBUILD_SHARED_LIBS=..]
                   [-DCMAKE_C_COMPILER=..] [-DCMAKE_CXX_COMPILER=..] <cmake_args..>  (+ extra_env)
     g. compile:   cmake --build <build> --config <type> [--parallel N]
     h. scan_artifacts(build_dir) (skips CMakeFiles/, CMakeCache.txt, *.ninja, Makefile, ...)
     i. write metadata.toml atomically (BuildMetadata, format_version = 1)
6. Install        CMakeInstallEngine::install(&InstallRequest)                  [install/engine.rs:224]
     a. require CMakeCache.txt or cmake_install.cmake in the build dir
     b. dest = <.crux/installed>/<name>/<version>/<fp-hex>; reuse if installed.toml + files verify
     c. staging dir under .crux/staging/<unique>
     d. read cmake_install.cmake; if it contains CMAKE_ABSOLUTE_DESTINATION_FILES then
        `cmake --install --prefix / ` with DESTDIR=<staging>, else `--prefix <staging>`
     e. promote_unwrapped_prefix() for the DESTDIR case (usr/local -> root)
     f. scan_staging_dir() -> inventory (InstalledFileType classification, symlink/traversal checks)
     g. write installed.toml (InstallMetadata) inside staging; atomic rename into dest
7. Expose         validate_package_installation(..) -> plan_app_dependencies(..)  [integrate/validation.rs, build/dependency.rs]
                  derives include dirs / link libs / runtime dirs from the inventory only
8. App build      generate_app_cmakelists(..) with absolute dependency paths      [build/generator.rs]
                  AppBuildEngine::execute_with_tee: cmake -S generated -B .crux/build/<profile>
```

### 2.3 CMake execution boundaries (exact)

| Boundary | Location | What crosses it |
|---|---|---|
| Probe | `CMakeBuildEngine::detect_toolchain` (`build/engine.rs:~110`), `detect_cmake` (`build/app.rs:735`) | `cmake --version`, `ninja --version`, `gcc/g++ --version` |
| Configure | `engine.rs` step 6 (`-S/-B/-G/-D...`) | argv vector, never a shell; env from `extra_env` |
| Compile | `engine.rs` step 7 (`--build`, `--config`, `--parallel`) | argv |
| Install | `install/engine.rs` step 4 (`--install`, `--prefix`, `DESTDIR`) | argv; uses `Command::output()` (not `run_logged`), so no timeout or live tee, unlike configure and build |
| Post-conditions | `scan_artifacts`, `validate_build_result`, `has_absolute_dest` | read CMake's on-disk artifacts (`CMakeCache.txt`, `cmake_install.cmake`) |
| Cache | `<builds_root>/<name>/<ver>/<fp>/`, `<install_root>/<name>/<ver>/<fp>/` | fingerprint hex is the directory name |

Two things already outside CMake and reusable as-is: `core::process::run_logged`, `describe_command` and `OutputObserver` (log capture and tee), and `core::filesystem::write_atomically`.

### 2.4 Observations that matter for the design

- **Two independent toolchain-detection stacks.** The package engine's `detect_toolchain` is minimal: it falls back to the literal `gcc`/`g++` and records only the first `--version` line (`ToolchainInfo` in `build/toolchain.rs:13`). The application path uses the richer `detect_compiler` / `CompilerInfo` (family, version, target triple, path, `CC`/`CXX` env, standard probing, sanitizer probing). The package engine ignores `CC`/`CXX`, while CMake itself honours them at first configure. If `CC=clang` is set and no override is passed, CMake builds with clang but the fingerprint records gcc's version. **[unverified: reading of code, not reproduced]**
- **Dependencies of a package are not wired into its build.** `grep` finds no `CMAKE_PREFIX_PATH`, `PKG_CONFIG_PATH` or dependency-fingerprint input anywhere in `build/engine.rs`. `BuildFingerprint::compute` also has no dependency input (the *application* fingerprint does: `AppFingerprintInputs.dependency_fingerprints`). Today this is harmless because no `system = "cmake"` package in `registry/packages/` declares `[dependencies]` (checked), but it is a gap Meson would inherit and must be closed for either system once dependent packages are added.
- **Install resolution is fingerprint-agnostic.** `validate_package_installation` selects the first sorted fingerprint directory matching `(name, version, profile)` (`integrate/validation.rs`, the `break` after the first valid candidate). A stale install with a different fingerprint (changed `cmake_args`, compiler upgrade) can therefore shadow the intended one. This is pre-existing, but a build-system switch for the same `name@version` makes it more likely (§7).
- **`BuildSettings` (`[build]` in `crux.toml`) never applies to dependencies** (`src/project/build_settings.rs` module doc). Sanitizers and custom flags are therefore application-only, which keeps the package fingerprint small.

---

## 3. Core versus CMake-specific responsibilities

| Responsibility | Where today | Classification |
|---|---|---|
| Package identity, resolution, lockfile, registry, source acquisition, checksum verification | `src/package`, `src/resolution`, `src/registry`, `src/source` | **Core**, no build-system knowledge (verified: `Lockfile` records no build info) |
| Cache directory layout `<root>/<name>/<ver>/<fp>/` | `engine.rs`, `install/engine.rs` | **Core** |
| Build-cache validation (metadata parses, success, fingerprint, artifacts exist with size) | `CMakeBuildEngine::cached_result` | **Core logic** hosted in a CMake type |
| Log capture, tee, tail summary, failure formatting | `run_step`, `extract_log_tail`, `core::process`, `cli::report::write_build_failure` | **Core** (generic, but `BuildError::{ConfigureFailed, BuildFailed}` messages say "CMake configuration failed") |
| `BuildProfile` (debug/release) | `build/metadata.rs` | **Core concept**; `as_cmake_build_type()` is the CMake mapping |
| Compiler detection and identity | `build/toolchain.rs` (`CompilerInfo`, `detect_compiler`) | **Core / toolchain**; the package engine bypasses it today |
| Sanitizer probe and flags | `build/sanitizer.rs` | **Core / toolchain** (`-fsanitize=` is compiler-level); applied to apps only |
| Staging, inventory, symlink/traversal guards, atomic publication, `installed.toml` | `install/engine.rs`, `inventory.rs`, `metadata.rs` | **Core**; the metadata has no CMake fields |
| Application link plan | `build/dependency.rs` | **Core**; inventory-driven |
| Installation integrity validation | `integrate/validation.rs` | **Core** |
| Configure/compile command construction | `engine.rs` steps 6-7 | **CMake adapter** |
| `CMakeLists.txt` precondition | `engine.rs`, `BuildError::MissingCMakeLists` | **CMake adapter** |
| `cmake_args` | `BuildConfiguration`, `BuildSection`, `build_configuration_for` | **CMake adapter option** (lives in core types today) |
| Generator selection (Ninja or Unix Makefiles) | `engine.rs::detect_toolchain`, `app.rs::select_generator` | **CMake adapter**; the underlying idea (executor) is shared |
| Build-artifact scan skip list (`CMakeFiles`, `*.ninja`, ...) | `scan_artifacts` | **CMake adapter** |
| `CMakeCache.txt` / `cmake_install.cmake` checks, `has_absolute_dest`, `DESTDIR` / `--prefix /` | `install/engine.rs` | **CMake adapter** |
| `InstalledFileType::CMakeModule` | `install/metadata.rs` | Classification of an *output file kind*; neutral enough to keep |
| `crux integrate cmake` | `src/integrate/cmake.rs` | **Consumer-side CMake**, orthogonal to how a dependency was built |
| Application `CMakeLists.txt` generation | `build/generator.rs` | **Internal app backend**; separate decision (§10) |
| Build-system detection for `crux import` | `project/discovery.rs::DetectedBuildSystem { Cmake, Make, None }` | Informational; independent of package builds |

### Shared concepts that an adapter interface can reasonably express

Everything an adapter needs fits the lifecycle already present: **probe** (which tools, which versions), **validate source** (marker file present), **contribute fingerprint fields**, **configure**, **compile**, **install into a staging root**, **report produced artifacts**. The shared pipeline owns everything else.

---

## 4. Recommended adapter boundary

Draw the line at *"run these tool steps"*, **not** at *"run the pipeline"*:

```
Crux core (shared pipeline)                               Build-system adapter (per system)
------------------------------------------------------    ----------------------------------------
resolve def -> pick adapter by definition.build.system -> source_marker()  (CMakeLists.txt / meson.build)
acquire source                                            probe()          -> ToolInfo (tool + executor)
compute fingerprint = universal fields + adapter fields <- fingerprint_fields()
cache lookup (<root>/<name>/<ver>/<fp>/)
create build/logs dirs, force handling
                                                          configure(ctx)   -> argv + env
                                                          compile(ctx)     -> argv + env
   run each step via core::process::run_logged <--------- (adapter returns the command; core runs it)
scan artifacts (adapter-supplied ignore rules)
write metadata.toml atomically
                                                          install(ctx)     -> argv + env + layout hint
stage, inventory, publish, installed.toml
```

Design rule: **adapters build `Command`s; core executes them.** This keeps timeouts, log capture, `-vv` tee, argv-only execution (no shell) and failure formatting in one audited place, and it is what makes later isolation of "custom" builds tractable (one execution choke point to sandbox).

Adapter-owned (must not leak into core): marker file name; tool names and version probing; option vocabulary (`-D` for CMake, `-D`/`--buildtype` for Meson); how profile and shared/static map to tool flags; install prefix mechanics; the list of tool-internal files to ignore when scanning artifacts; the "build dir is configured" post-condition.

---

## 5. Is a Rust trait justified? Alternatives considered

| Option | Verdict |
|---|---|
| **A. Keep today's `BuildEngine` + `InstallEngine` traits, add `MesonBuildEngine` / `MesonInstallEngine`** | Reject. Wrong granularity: duplicates cache validation, metadata writing, log handling, staging promotion. CMake and Meson would drift. |
| **B. Closed `enum BuildSystem { Cmake, Meson }` with `match` in one shared pipeline** | Sufficient for two adapters. Exhaustive matches, no `dyn`, easy to test. Becomes a wall of `match` arms by the time there are 4. |
| **C. Narrow step-level trait (adapter builds commands, core runs them), selected via enum factory (static dispatch)** | **Recommended** once the second adapter exists. Crux's set of build systems is closed and in-tree, so there is no need for plugin-style `dyn` registration. |
| **D. Data-driven recipes (argv templates in `package.toml`)** | Rejected for CMake/Meson/Autotools (they need probing, tool-specific post-conditions). A restricted form may be the right shape for "custom" later. |
| **E. External adapter executables / plugins** | Non-goal: unbounded trust and ABI surface, conflicts with the single-binary, no-sandbox security posture. |

**Sequencing recommendation**: do the *extraction* (shared pipeline, adapter-owned steps) with CMake as the only adapter, behind byte-identical behaviour, then add Meson. Introduce the trait in the same change that adds the second implementor, so its shape is validated by two real users rather than by speculation (the existing `BuildEngine` traits are a cautionary example of an abstraction fitted to one).

---

## 6. Proposed build context and result contracts

Illustrative only. These are sketches for discussion and must not be treated as final signatures.

### 6.1 Input: `BuildContext` (what core gives an adapter)

| Field | Source today | Notes |
|---|---|---|
| `package_id`, `source_dir`, `source_checksum` | `BuildRequest` | unchanged |
| `profile` | `BuildConfiguration.profile` | adapter maps to `CMAKE_BUILD_TYPE` or `--buildtype` |
| `link_kind` (shared / static / unspecified) | `build_shared_libs: Option<bool>` | neutral name; adapter maps to `BUILD_SHARED_LIBS` or `--default-library` |
| `jobs` | `BuildConfiguration.jobs` | |
| `compilers` (C, C++ path and identity) | `c_compiler` / `cxx_compiler` + `CompilerInfo` | unify with `detect_compiler` (§8) |
| `adapter_options: Vec<String>` | `cmake_args` | opaque to core, validated by adapter |
| `extra_env` | `BuildConfiguration.extra_env` | |
| `dirs` (`build_dir`, `logs_dir`, `staging_root`) | computed in `engine.rs` | core owns layout |
| `observer` | `output_observer` | |
| *(future)* `dependency_prefixes` | **missing today** | needed for packages that depend on packages (§2.4) |

### 6.2 Output: `BuildOutcome` / adapter result

| Field | Notes |
|---|---|
| `tool_versions` (ordered `label -> version string`) | e.g. `cmake`, `ninja`; replaces CMake-only `ToolchainInfo.cmake_version` |
| `configure_log`, `build_log`, `install_log` | all three through `run_logged` (fixes the install-step asymmetry) |
| `artifacts: Vec<BuildArtifact>` | adapter supplies ignore rules; core classifies (`ArtifactType::from_path`) |
| `install_layout` | `PrefixInstall` (tool honours `--prefix` at install) vs `DestdirInstall { prefix }`; core uses existing `promote_unwrapped_prefix` for the latter |
| `adapter` (id string, e.g. `"cmake"`, `"meson"`) | recorded in metadata |

### 6.3 Errors

`BuildError` and `InstallError` embed "CMake" in variant names and Display text (`MissingCMakeLists`, `MissingCMake`, `InstallError::MissingCMakeConfig`, `CMakeExecutionFailed`). Prefer generic variants carrying a `tool: String` (`MissingSourceMarker`, `ToolMissing { tool }`, `StepFailed { step, tool, .. }`) and keep today's wording for CMake so existing diagnostics and tests do not change. These enums are re-exported from `src/lib.rs`, so renames are public-API changes (§9).

---

## 7. Fingerprint and cache implications

### 7.1 Field classification (from `BuildFingerprint::compute`, `build/fingerprint.rs`)

| Preimage field | Class | Note |
|---|---|---|
| `name@version\n` | Universal | |
| source checksum digest | Universal | |
| `profile` | Universal | |
| `toolchain.cmake_version` | **CMake-specific** | becomes `tool.<id>.version` |
| `toolchain.generator` | **Executor / CMake-specific** | CMake: "Ninja" or "Unix Makefiles" |
| c / cxx compiler version line | Universal | first line of `--version` only; no target triple |
| `config.c_compiler` / `cxx_compiler` path overrides | Universal | |
| `shared:0/1` | Universal concept | mapping is per adapter |
| sorted `cmake_args` | **CMake-specific** | note it *sorts*, so order-dependent duplicate `-D` options hash identically; Meson option order must be preserved or sorted deliberately |
| sorted `extra_env` | Universal | |
| *absent* | Missing universals | target triple, host OS/arch (`PlatformInfo` exists but is unused here), dependency fingerprints, a format version, an adapter id |

The application fingerprint (`app_build_fingerprint`) already shows the better pattern: length-prefixed labelled fields and `field("format", b"crux-app-build-v1")`. The package fingerprint predates it.

### 7.2 How Meson participates without breaking CMake caches

1. **Do not touch the CMake preimage.** Pin it first: add a golden-digest test (fixed `PackageId`, `Checksum`, `ToolchainInfo`, `BuildConfiguration` → literal `sha256:...`). Any refactor that moves `compute` must keep that test green.
2. **Give non-CMake adapters a separate, versioned, labelled preimage** (the app-fingerprint style: `format=crux-pkg-build-v2`, `system=meson`, length-prefixed fields). Different preimage structure means a Meson digest can never equal a CMake digest, even for identical sources and compilers, and no migration is needed because the directories are keyed by digest.
3. **Optionally, later, move CMake to v2 at a deliberate, announced release** (it would invalidate all caches once; add the missing universals then). Not a prerequisite for Meson.
4. **Directory layout stays `<name>/<version>/<fp-hex>/`.** Same `name@version` built by two systems (e.g. a registry version flipping from cmake to meson) yields two sibling fingerprint directories. Because `validate_package_installation` takes the first sorted match by `(name, version, profile)`, this can resolve to the wrong one. Fix before shipping a package with two build systems for one version: have the installer record the chosen fingerprint, or have validation match the expected fingerprint. Simplest policy: a registry must never change `system` for an existing `name@version`; the registry linter should enforce it.
5. **Metadata additions use `#[serde(default)]`** (`BuildMetadata`, `InstallMetadata`): `adapter: Option<String>` and a generic `tools` map. Old records parse as "cmake". `ToolchainInfo.cmake_version` and `generator` are non-optional, so keep them for CMake and add the generic field alongside rather than replacing it. `BuildMetadata.format_version` stays 1 if all additions are defaulted.

### 7.3 Staleness inside a build dir

`build_dir_is_stale` (`build/app.rs:862`) parses `CMakeCache.txt` and is application-only. Package builds avoid the problem structurally (fresh fingerprint directory per configuration), so no Meson equivalent is needed for packages. If the application backend is ever made pluggable, it would need its own staleness probe (Meson: `meson setup --reconfigure` / `--wipe`).

### 7.4 Sanitizers

Sanitizer *selection* is universal and lives in `BuildSettings` (`address_sanitizer`, `undefined_sanitizer`), canonicalised by `canonical_bytes()`. The *mapping* is adapter-specific (`-fsanitize=` via compile/link flags today; `-Db_sanitize=address,undefined` in Meson). Because sanitizers apply to applications only, nothing changes for package fingerprints. If Crux later sanitises dependencies, the sanitizer set becomes a universal fingerprint field. Flag this as a deliberate future decision, not part of the Meson work.

---

## 8. Dependency installation and exposure

### 8.1 Can build systems share the install contract?

**Yes, with two caveats.** The contract is: *a directory tree under `<install_root>/<name>/<ver>/<fp>/` containing headers (preferably under `include/`), libraries (`.a`, `.so*`, `.dylib`, `.lib`, `.dll`), optional executables and metadata, plus `installed.toml` listing every file with type and size.* Consumers (`derive_include_dirs`, `derive_link_libraries`, `build_link_entry`) use only that listing:

- headers → nearest `include/` ancestor, else the install root;
- libraries → static preferred if any, else shared with RPATH directories;
- no read of `*.cmake`, `.pc`, `CMakeCache.txt`.

### 8.2 Assumptions that currently require CMake

| Assumption | Location | Impact for Meson |
|---|---|---|
| `CMakeCache.txt` or `cmake_install.cmake` must exist in the build dir | `CMakeInstallEngine::validate_build_result`, `InstallError::MissingCMakeConfig` | adapter-owned post-condition (Meson: `meson-info/`, `build.ninja`) |
| `--prefix <staging>` at install time | `install/engine.rs` step 4 | Meson fixes `--prefix` at *setup* time and supports `meson install --destdir`. Use `DESTDIR` + a fixed prefix and reuse `promote_unwrapped_prefix` |
| Absolute-destination detection by scanning `cmake_install.cmake` text | `has_absolute_dest` | CMake-only workaround |
| Install step bypasses `run_logged` | `install/engine.rs` | normalise (no timeout or tee on install today) |
| Libdir | n/a | Meson defaults `libdir` to a multiarch path (e.g. `lib/x86_64-linux-gnu`) on many Linux distributions; inventory-driven linking tolerates this, but pass `--libdir=lib` for a deterministic layout across hosts **[unverified]** |
| `.pc` files bake `prefix=` | Meson and CMake both generate | `prefix` would be the DESTDIR-relative prefix, not the final Crux install path. Irrelevant to application linking (inventory-driven), relevant to `crux integrate` consumers **[unverified]** |
| `InstalledFileType::CMakeModule` classification | `install/metadata.rs` | harmless; Meson packages simply have none |

### 8.3 Consumer integration is a separate axis

`crux integrate cmake` (`src/integrate/cmake.rs`) injects `CMAKE_PREFIX_PATH`/`CMAKE_MODULE_PATH`. A CMake consumer can `find_package(foo CONFIG)` a Meson-built dependency only if that package ships a CMake config or Find module. Meson projects often ship only a `.pc` file. Nothing in `integrate/` sets `PKG_CONFIG_PATH`, and Crux has no `.pc`-based consumption. This is an *integration-coverage* gap for Meson packages, not a build blocker. It should be documented per package and decided later (a `crux integrate pkg-config` is a separate feature, not a prerequisite).

### 8.4 Application build independence

Apps link by absolute paths emitted into a generated CMake project from the inventory-derived plan. Keeping CMake as the internal application backend (§10, unresolved decision 3) means Meson dependency support requires **zero application-side changes**.

---

## 9. Manifest and registry design

### 9.1 What exists

- `package.toml` `[build]` → `BuildSection { system: String, cmake_args: Vec<String> }` with `#[serde(deny_unknown_fields)]` (`src/registry/metadata.rs`).
- `system` is a free `String`, compared against `"cmake"` in `require_supported_build_system` (`cli/commands/mod.rs:58`). The value `"catalog-only"` (35 of 53 definitions in `registry/packages`; 18 are `cmake`) is overloaded: it is not a build system but a "do not build" sentinel.
- A definition with **no `[build]`** is accepted and treated as CMake (documented in the `require_supported_build_system` doc comment).
- `cmake_args` is validated and sorted in `parse_package_definition` (`-`-prefixed, no NUL or newline).
- Direct sources reuse the same parser through the source tree's own `package.toml` (`src/source/direct.rs:418`), so they inherit any registry-schema change for free.
- Application `crux.toml` has `[build]` for flags and sanitizers (`src/project/build_settings.rs`), explicitly *not* applied to dependencies.

### 9.2 Recommendation (minimal)

1. **Keep `[build].system` as the single selector.** Valid values: `cmake` (default when omitted), `meson`, `catalog-only` (reserved). Unknown values keep today's error (`...supports only 'cmake'; this package is catalog-only`), updated to list supported systems.
2. **Replace the stringly-typed compare with a parsed enum at registry load**, e.g. `{ Cmake, Meson, CatalogOnly, Unsupported(String) }`, so the four `require_supported_build_system` call sites (`install.rs:232`, `install.rs:362`, `build.rs:466`, `build.rs:596`) become one capability check. Preserve existing diagnostics.
3. **Per-system option arrays rather than a generic options bag**: keep `cmake_args`, add `meson_args` with the same validation (argv elements, `-` prefix, no control characters). A generic `options` table is unnecessary until a third system needs it; "do not propose unnecessary manifest complexity".
4. **Do not add anything to the consuming `crux.toml`.** There is no per-project reason to choose how a *dependency* builds. Per-project overrides of dependency build options would also break reproducibility (it is deliberately excluded from the app's `[build]`).
5. **No lockfile change.** `Lockfile` records source identity and checksums only. The build system is a function of the definition; it is covered by the cache fingerprint, not the lock.

### 9.3 Backward / forward compatibility risk (registry)

`BuildSection` and `RawPackageDefinition` use `deny_unknown_fields`. A registry entry using `meson_args` will fail to parse on existing Crux 0.3.x clients (`RegistryError::Toml`). `system = "meson"` alone degrades gracefully (rejected at build with a clear message). Open points:

- whether one unparsable `package.toml` poisons loading of the whole registry or only that package **[unverified; not traced through `registry/git.rs`]**;
- whether to bump `CURRENT_REGISTRY_FORMAT_VERSION` (currently 1, `registry/metadata.rs`) or ship the first Meson packages with `system = "meson"` and no extra keys.

### 9.4 Import (`crux import`)

`DetectedBuildSystem` (`project/discovery.rs:26`) detects `Cmake | Make | None` and emits warnings stating that existing CMake/Make files are ignored in favour of Crux's internal project. Adding a `Meson` variant (detected via `meson.build`) is informational only and independent of package-build work. Do not couple it to the adapter milestones.

---

## 10. Meson feasibility

### 10.1 Lifecycle mapping

| Crux step | CMake (today) | Meson |
|---|---|---|
| Marker | `CMakeLists.txt` | `meson.build` |
| Probe | `cmake --version` | `meson --version` (**[unverified]** exact version floors); ninja (see below); compiler |
| Configure | `cmake -S src -B build -DCMAKE_BUILD_TYPE=.. -G <gen>` | `meson setup <build> <src> --buildtype=<debug\|release> --prefix=<abs> --libdir=lib --default-library=<..> --backend=ninja <-D..>` |
| Compile | `cmake --build build --config .. --parallel N` | `meson compile -C <build> -j N` |
| Install | `cmake --install build --prefix <staging>` | `meson install -C <build> --destdir <staging> [--no-rebuild]` |
| Compiler selection | `-DCMAKE_C_COMPILER=` / `-DCMAKE_CXX_COMPILER=` | `CC` / `CXX` environment at first `setup` (or a native file); argv alone is not enough |
| Reconfigure | stale `CMakeCache.txt` | `meson setup --reconfigure` / `--wipe` (irrelevant: fresh dir per fingerprint) |
| Tests | project-specific `-D..` | project-specific `-D..` (same registry-args mechanism) |

Meson's `--prefix` is a configure-time property, so the staging model differs: either `setup --prefix=<final or staging>` or a fixed prefix plus `DESTDIR`. The existing `DESTDIR` + `promote_unwrapped_prefix` path in `install/engine.rs` already handles "install under a prefix, then hoist it", so a fixed-prefix + `--destdir` approach reuses existing logic.

### 10.2 Required tools and backend choices

- **Required**: `meson` (Python-based), `ninja`, a C/C++ compiler. Optional: `pkg-config`/`pkgconf` (only for packages that need it). This machine has `meson`, `ninja`, `cmake`, `make` on `PATH` (observed via `which`; version numbers not recorded).
- **Backend**: Meson's practical Linux/macOS backend is Ninja. There is no Make backend. Visual Studio/Xcode backends exist but are non-goals. Pass `--backend=ninja` explicitly for determinism.

### 10.3 How Ninja and Make fit

- **Ninja is an executor, not an adapter.** It sits *under* an adapter: CMake emits `build.ninja` (`-G Ninja`), Meson emits `build.ninja` by default. Crux should model it as a tool whose presence and version are probed and fingerprinted, never as a `BuildSystem` variant.
- Today the Ninja decision is duplicated and CMake-flavoured: `engine.rs::has_command("ninja")` and `app.rs::select_generator()` (also `find_in_path("ninja")`); the former yields the generator string stored in `ToolchainInfo.generator`. Collapse to one probe returning `Executor { kind: Ninja | Make, version }`; CMake maps it to `-G`, Meson requires Ninja and fails with an actionable error if absent.
- **Make** is a CMake fallback today (`"Unix Makefiles"`) and is the executor for Autotools; it is not a Meson backend. It stays a CMake generator choice and a future Autotools tool. Recording the *version* of ninja/make in the fingerprint is optional (they do not affect output) and is **not** currently done for ninja; keep it that way to avoid pointless cache churn.

### 10.4 Meson-specific risks

| Risk | Mitigation |
|---|---|
| Meson `subproject()`/wrap downloads at setup time (network) | pass `--wrap-mode=nodownload`; document that CMake `FetchContent` is not blocked either (the security model states there is no sandbox in v0.3.0) |
| Python runtime dependency for `meson` | probe and report; not bundled |
| Multiarch `libdir` | `--libdir=lib` |
| Compiler chosen via env, not argv | set `CC`/`CXX` on the `Command` from `CompilerInfo.path`; include in fingerprint via compiler identity |
| `.pc` files not relocatable to the final prefix | out of scope for linking; documented limitation |
| Feasibility depends on a real package that builds hermetically | pick one pilot package (**[open]**) before committing; do not infer from theory |

**Verdict: feasible, moderate effort, low architectural risk**, because the application-link contract is already build-system-neutral. The risk is concentrated in fingerprint and cache hygiene (§7) and in install-prefix mechanics, not in the consumer side.

---

## 11. Future adapters (requirements only, no design)

**Autotools**
- Preconditions: `configure` present (release tarballs) or regenerated via `autoconf`/`automake`/`libtool`/`m4` (git sources). Regeneration runs arbitrary generator scripts.
- Steps: out-of-tree `<src>/configure --prefix=..`, `make -jN`, `make install DESTDIR=..`. Executor: Make, not Ninja.
- Environment sensitivity is high (`CFLAGS`, `LDFLAGS`, `PKG_CONFIG_PATH`, `config.site`), so fingerprinting must hash an explicit env allow-list, and Crux must scrub ambient env.
- Cross-compilation (`--host`) and in-tree-build pitfalls are real. Needs a real package with a stable release tarball to justify.

**Custom build scripts**
- Arbitrary code execution. Without an OS-level sandbox (explicitly absent in v0.3.0, `docs/reference/security-model.md`), this must not ship before an isolation story: restricted declarative command lists (argv arrays, not shell), working-directory confinement, explicit env, network-off, and the script content included in the fingerprint.
- Must emit the same `installed.toml` contract; inventory/staging already enforce traversal and symlink rules, which is the right backstop.

Neither should influence the first adapter extraction beyond keeping "adapter builds `Command`s, core runs them" (§4), which is the seam any sandbox would attach to.

---

## 12. Compatibility risks and migration strategy

| Risk | Severity | Evidence | Mitigation |
|---|---|---|---|
| Changing the package fingerprint preimage invalidates all user caches | **High** | digest is dir name for builds and installs (`engine.rs`, `install/engine.rs`) | golden test first; CMake preimage frozen; non-CMake gets a versioned preimage (§7.2) |
| Public API break | **Medium** | `src/lib.rs` re-exports `BuildEngine`, `BuildRequest`, `BuildResult`, `CMakeBuildEngine`, `ToolchainInfo`, `BuildFingerprint`, `InstallEngine`, `CMakeInstallEngine`, `BuildError`, `InstallError`; `tests/build.rs` builds `ToolchainInfo { .. }` literally and calls `BuildFingerprint::compute` | keep existing names and signatures as thin shims over the new pipeline for at least one minor release; add, don't rename |
| Old clients reading new registry keys (`deny_unknown_fields`) | Medium | `registry/metadata.rs` | ship `system = "meson"` without new keys first, or bump registry format (§9.3) |
| Wrong install shadowing when two fingerprints exist | Medium (pre-existing) | `validate_package_installation` first-match | forbid `system` flips for an existing version; later make validation fingerprint-aware |
| Package/app toolchain detection divergence (`CC`/`CXX`) | Medium (pre-existing) | `detect_toolchain` vs `detect_compiler` | converge on `detect_compiler`; changes fingerprints, so schedule with the v2 preimage, not before |
| Package-to-package dependencies not wired into builds | Medium (latent) | no `CMAKE_PREFIX_PATH` in engine | required before any dependent package ships, for any adapter |
| Error text/regression | Low | `BuildError::MissingCMake*` messages used by tests | keep CMake strings identical behind generic variants |
| Install step lacks timeout/tee | Low | `Command::output()` in `install/engine.rs` | route through `run_logged` when the install step becomes adapter-driven |
| Behaviour tests need real tools | Low | `tests/build.rs` runs real cmake | Meson integration tests must skip cleanly when `meson`/`ninja` are absent |

**Migration rules**: (1) no behaviour change to any CMake invocation during extraction; (2) compare the argv of configure/compile/install before and after the refactor with a recording test; (3) additive metadata only, defaulted via `serde`; (4) the first release that includes Meson must be able to read caches written by 0.3.0 unchanged.

---

## 13. Recommended incremental milestones

Each milestone is independently shippable and leaves CMake behaviour unchanged. Names are suggestions.

| # | Milestone | Content | Exit criteria |
|---|---|---|---|
| **M0** | Characterise | Golden fingerprint digest test for `BuildFingerprint::compute`; argv-recording test for configure/compile/install; test that old `BuildMetadata`/`InstallMetadata` TOML still parses | tests green on unmodified code |
| **M1** | Typed build-system selection | `BuildSystemKind` parsed at registry load; collapse `require_supported_build_system`'s four call sites; unchanged messages; `catalog-only` modelled explicitly | existing registry and CLI tests unchanged |
| **M2** | Extract shared pipeline | Move cache validation, dir layout, log/metadata/artifact handling out of `CMakeBuildEngine`; CMake-specific steps become a private adapter; keep `CMakeBuildEngine` / `CMakeInstallEngine` as shims; install step through `run_logged` | M0 tests identical; argv identical |
| **M3** | Unify toolchain probe | single compiler/executor probe feeding both package and app paths (`CC`/`CXX`, ninja/make); **fingerprint-affecting, so land with the v2 preimage decision** | documented cache impact |
| **M4** | Adapter seam (trait) | introduce the step-level trait with `CmakeAdapter` as sole impl, enum-factory dispatch; generic errors with CMake wording preserved | no `dyn`; all tests pass |
| **M5** | Meson adapter + pilot package | `meson` probe, setup/compile/install steps, `meson_args`, versioned fingerprint preimage, `--wrap-mode=nodownload`, one pilot registry package; skip-if-absent tests | pilot package installs and links via `crux build` |
| **M6** | Hardening | fingerprint-aware install validation; dependency-prefix wiring for package-to-package deps; `.pc` story documented | multi-fingerprint scenario tested |
| **Later** | Autotools / custom | only when concrete packages demand them; custom requires the isolation design first | separate assessments |

M1 and M2 are valuable even if Meson never ships, because they remove the stringly-typed gating and duplicated pipeline logic.

---

## 14. Explicit non-goals

- Implementing or prototyping any adapter, trait, or fingerprint change as part of this assessment.
- A plugin or external-adapter system (option E).
- Selecting the build system per application from `crux.toml`, or per-project overrides of dependency build options.
- Replacing CMake as the internal *application* backend, or generating Meson/Ninja files for the user's own application.
- Making Ninja or Make `BuildSystem` variants.
- Windows/MSVC build support (the `CompilerFamily::Msvc` variant exists; behaviour is out of scope here).
- Sanitising dependencies, cross-compilation, or binary caching/sharing across machines.
- A pkg-config consumer integration (`crux integrate pkg-config`) as a Meson prerequisite.
- An OS-level sandbox (prerequisite for "custom", not part of this work).

## 15. Unresolved decisions

1. **Application backend**: stay with internal CMake generation for `crux build`/`run` indefinitely (recommended, since it needs no change for Meson *dependencies*), or eventually offer a Meson-generating app backend?
2. **Fingerprint v2 timing**: introduce the labelled, versioned preimage for non-CMake only (recommended for M5), or also migrate CMake at a announced cache-invalidating release (cleaner, adds target triple, platform, dependency fingerprints)?
3. **Registry compatibility**: bump `CURRENT_REGISTRY_FORMAT_VERSION`, or restrict first Meson packages to schema-compatible entries? Needs a trace of how a bad `package.toml` is handled in `registry/git.rs` (not done here).
4. **Meson option key**: `meson_args` (per-system, recommended) versus a generic options field.
5. **Policy on build-system changes for an existing `name@version`**: forbid (recommended) versus support via fingerprint-aware validation.
6. **Pilot Meson package**: which hermetic, offline-buildable package validates the design? Not chosen.
7. **Minimum supported Meson/Ninja versions** and the exact flag set (`--destdir`, `--no-rebuild`, `--wrap-mode`) need confirmation against the versions Crux will document **[unverified]**.
8. **Compiler detection convergence** (package engine vs `detect_compiler`) and its cache consequences (M3).
9. **Package-to-package dependency wiring** semantics (prefix path env, pkg-config path, fingerprint input), common to every adapter.
10. **`.pc` relocation and pkg-config consumption**: whether and when to support.

---

## 16. Inspected files

Core review: `AGENTS.md`; `src/lib.rs`; `src/build/{mod,engine,metadata,fingerprint,toolchain,app,dependency,generator,sanitizer,adhoc}.rs`; `src/install/{mod,engine,metadata}.rs`; `src/integrate/{mod,validation,cmake}.rs`; `src/cli/commands/{mod,build,install,run}.rs`; `src/cli/report.rs` (`build_package`); `src/registry/metadata.rs`; `src/project/{build_settings,discovery}.rs`; `src/source/direct.rs` (definition parsing); `registry/packages/**/package.toml` (system distribution, `cmake_args`, `[dependencies]`); `tests/build.rs`; `docs/reference/{architecture,security-model}.md`; `docs/guides/build-configuration.md`; `docs/reports/package-ecosystem-expansion-report.md`.

Not read in full (cited only via grep): `src/resolution/lockfile.rs`, `src/registry/git.rs`, `src/source/{cache,download,extraction}.rs`, `src/project/{manifest,import}.rs`, `src/cli/args.rs`, `src/install/inventory.rs` internals. Nothing was executed (no build, test, or tool run beyond `which`, `ls`, `grep`, `git status`).
