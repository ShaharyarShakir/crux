# M0 — Build Pipeline Characterisation

Date: 2026-11-27  
Repository state: `main` at `38b1beb` (`Add build system architecture assessment report for second pass`)  
Worktree: repository root, no branch/worktree created  
Scope: tests and documentation only; production source unchanged

## 1. Objective and boundaries

This milestone records the current behaviour of the CMake package-build pipeline before any
refactoring. It adds characterization tests only. No production code, manifest, lockfile, Nix
configuration, website file, or dependency was changed. No Git operation beyond status/diff
inspection was performed.

Initial `git status --porcelain --untracked-files=all` was empty (clean working tree). After this
milestone the only changes are untracked files:

- `tests/m0_fingerprint_golden.rs`
- `tests/m0_build_commands.rs`
- `tests/m0_metadata_compat.rs`
- `docs/reports/m0-build-pipeline-characterisation.md` (this file)

Nothing was staged or committed.

## 2. Baseline implementation inspected

- `src/build/fingerprint.rs` — `BuildFingerprint::compute` and `app_build_fingerprint`.
- `src/build/engine.rs` — `CMakeBuildEngine::build`, `detect_toolchain`, cached-result validation,
  configure/build command construction, metadata write.
- `src/build/metadata.rs` — `BuildConfiguration`, `BuildMetadata`, `BuildProfile`, `ArtifactType`.
- `src/install/engine.rs` — `CMakeInstallEngine::install`, `cmake --install` construction,
  `DESTDIR` handling, staging and atomic publication.
- `src/install/metadata.rs` and `src/install/inventory.rs` — `InstallMetadata`, scanner,
  `promote_unwrapped_prefix`.
- Existing `tests/build.rs` and `tests/install.rs` — real-CMake integration coverage already
  present. The new tests supplement them; they do not replace or alter them.

## 3. Golden fingerprint test

Location: `tests/m0_fingerprint_golden.rs`

Tests:

- `build_fingerprint_matches_golden_digest`
- `build_fingerprint_golden_computation_is_repeatable`

Expected digest:

```text
sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe
```

Fixed inputs (all constructed literals; no compiler/version probe, clock, PATH lookup, or
environment variable):

| Input | Fixed value |
| --- | --- |
| Package | `char-lib@1.2.3` |
| Source checksum | `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef` |
| Toolchain CMake version | `cmake version 3.31.0` |
| Toolchain generator | `Unix Makefiles` |
| C compiler version | `gcc (GCC) 14.2.0` |
| C++ compiler version | `g++ (GCC) 14.2.0` |
| Config profile | `debug` |
| Config generator | `Unix Makefiles` |
| Config CMake args | `-DSECOND=2`, `-DFIRST=1` (declared out of order) |
| Config C compiler path | `/opt/toolchains/char-gcc/cc` |
| Config C++ compiler path | `/opt/toolchains/char-gcc/c++` |
| Config jobs | `8` |
| Shared libraries | `true` |
| Extra environment | `CRUX_M0_A=alpha`, `CRUX_M0_B=beta` |

The expected value was computed independently with Python's `hashlib.sha256` over the canonical
byte stream implemented by `BuildFingerprint::compute` (274 bytes). Repeated computation eight
times in the test produced the same digest.

Observed algorithm details worth preserving as baseline:

- `cmake_args` are sorted before hashing, so declaration order does not affect the fingerprint.
- `extra_env` is a `BTreeMap`; its iteration order is by key.
- `jobs` and `configuration.generator` are not part of the hashed byte stream. The hashed generator
  is `toolchain.generator`.
- `BuildFingerprint::new` validates the `sha256:<64 hex>` shape, but derived deserialization
  bypasses that check (see discrepancies).

## 4. Build command characterization

Location: `tests/m0_build_commands.rs` (Unix-only: the fake tools are `/bin/sh` scripts).

A generated fake `cmake` executable records each invocation as an argv vector, one argument per
line, with record delimiters. The engines receive that binary through their existing
`with_cmake_bin` API. Fake compiler scripts supply deterministic `--version` output, and
`config.generator` is fixed so `detect_toolchain` does not probe `PATH` for Ninja. No real CMake,
compiler, generator, or network is used by these tests.

Tests:

- `configure_and_build_arguments_are_recorded_as_vectors`
- `install_arguments_and_standard_staging_are_recorded`
- `install_arguments_and_destdir_staging_are_recorded_for_absolute_destinations`

### Recorded argv

Toolchain probe (separate from configure):

```text
--version
```

Configure:

```text
-S                                  <source_dir>
-B                                  <build_root>/char-lib/1.2.3/<fingerprint_hex>/build
-DCMAKE_BUILD_TYPE=Debug
-G                                  Unix Makefiles
-DBUILD_SHARED_LIBS=ON
-DCMAKE_C_COMPILER=<fake cc path>
-DCMAKE_CXX_COMPILER=<fake cxx path>
-DCRUX_M0_FLAG=ON
-DCRUX_M0_VALUE=seven
```

`CRUX_M0_ENV_MARKER=present` from `BuildConfiguration::extra_env` is attached to the configure
command only. Generator and profile are separate argv entries where the implementation uses
separate `arg(...)` calls; `"Unix Makefiles"` is one argument, not three.

Build:

```text
--build                             <build_root>/char-lib/1.2.3/<fingerprint_hex>/build
--config                            Debug
--parallel                          4
```

Install, normal staging (cmake_install.cmake has no absolute-destination marker):

```text
--install                           <build_dir>
--prefix                            <staging_root>/<dynamic-staging-id>
--config                            Debug
```

The dynamic staging id contains package name, version, PID, and nanoseconds. The test therefore
asserts its structure (direct child of the configured staging root) rather than a literal value.
After success the staging directory is renamed to
`<install_root>/char-lib/1.2.3/<fingerprint_hex>` and the staging root is empty.

Install, absolute destinations (`cmake_install.cmake` contains
`CMAKE_ABSOLUTE_DESTINATION_FILES`):

```text
--install                           <build_dir>
--prefix                            /
--config                            Debug
```

with environment `DESTDIR=<staging_root>/<dynamic-staging-id>`. The fake install writes to
`usr/local/include/char.h`; `promote_unwrapped_prefix` lifts it to `include/char.h` in the staged
tree before publication.

Assertions are made against `Vec<String>` argv slices, not shell-formatted strings.

## 5. Metadata compatibility

Location: `tests/m0_metadata_compat.rs`

Tests:

- `current_build_metadata_fixture_deserializes`
- `older_build_metadata_uses_existing_serde_defaults`
- `install_metadata_without_optional_installed_at_deserializes`
- `invalid_build_metadata_is_rejected`
- `characterise_unvalidated_build_metadata_fields`

Results:

- The current `metadata.toml` fixture deserializes with all nested `toolchain`, `configuration`,
  and `artifacts` fields intact.
- The older fixture omits every `#[serde(default)]` field in `BuildConfiguration`. Existing
  defaults are applied: `generator = None`, `cmake_args = []`, `c_compiler = None`,
  `cxx_compiler = None`, `jobs = None`, `build_shared_libs = None`, `extra_env = {}`.
- `InstallMetadata` without `installed_at` deserializes with `installed_at = None`.
- Missing required `fingerprint`, unknown `BuildProfile`, unknown `ArtifactType`, and a
  non-numeric `format_version` are rejected with `Err` from `toml::from_str`; no panic occurs.

No migration or serialization-format change was introduced.

## 6. Discovered discrepancies (characterized, not fixed)

1. `BuildMetadata` uses derived `Deserialize`. A malformed fingerprint such as
   `sha256:not-hex` is accepted because `BuildFingerprint::new` validation is not invoked during
   deserialization. The test `characterise_unvalidated_build_metadata_fields` records this.
2. `BuildMetadata.format_version` is not checked during deserialization; `format_version = 999`
   parses successfully. `CMakeInstallEngine::check_existing_cache` does compare
   `InstallMetadata.format_version` against `CURRENT_INSTALL_METADATA_VERSION`, so the two metadata
   types behave differently.
3. `BuildFingerprint::compute` does not hash `BuildConfiguration::jobs` or
   `BuildConfiguration::generator`. A change to either alone will not invalidate a cached build
   directory. This is recorded as baseline behaviour, not changed here.
4. The full suite has two environment-dependent failures in pre-existing
   `tests/build_settings.rs` tests; see section 7. They are unrelated to the new characterization
   tests and are not skipped or suppressed.

## 7. Verification commands and outcomes

All commands were run from the repository root inside the Nix flake shell.

| Command | Outcome |
| --- | --- |
| `nix develop --command cargo test --test m0_fingerprint_golden --test m0_build_commands --test m0_metadata_compat` | 10 passed, 0 failed (3 + 2 + 5 tests) |
| `nix develop --command cargo fmt` | Applied formatting to the three new test files only |
| `nix develop --command cargo fmt --check` | Clean, no diff |
| `nix develop --command cargo clippy --all-targets -- -D warnings` | Clean, no warnings |
| `nix develop --command cargo check` | Clean |
| `nix develop --command cargo test` | Stopped after `tests/build_settings.rs`: 2 failures |
| `nix develop --command cargo test --no-fail-fast` | 33 test binaries: 707 passed, 2 failed; all three M0 test binaries passed |

The two failures were:

- `tests/build_settings.rs::address_and_undefined_sanitizers_instrument_the_binary`
- `tests/build_settings.rs::unchanged_settings_reuse_the_build_tree_and_changes_invalidate_it`

Both fail during the existing sanitizer capability probe because the Nix-provided GCC 16.2.0
cannot link a trivial program with `-fsanitize=address,undefined` or `-fsanitize=undefined`:

```text
ld.bfd: .../librt.so.1: undefined reference to `__pointer_chk_guard@GLIBC_PRIVATE'
collect2: error: ld returned 1 exit status
```

No test was skipped. These two tests did execute and failed for an environment/toolchain runtime
reason; fixing them would require production or environment changes and is outside M0 scope.

## 8. Completion status

Completion criterion met: all 10 new characterization tests pass against the unchanged production
implementation. The full suite result is 707 passed / 2 pre-existing environment-dependent
failures, with no M0 test failure and no skipped test.

Suggested Conventional Commit messages (not applied; Git operations are owner-controlled):

```text
test(build): characterise CMake pipeline baseline

docs(reports): add M0 build pipeline characterisation report
```
