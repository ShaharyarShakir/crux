# M2: Toolchain Fingerprinting and Header-Only Build Caching

Date: 2026-11-27
Base: `main` at `38b1beb`, repository root, on top of the uncommitted M0 and M1 work
Git: nothing staged or committed; no branch or worktree created.

## 1. Root causes

- **Wrong compiler identity.** `detect_toolchain` queried literal `gcc`/`g++`, while CMake,
  given no `-DCMAKE_<LANG>_COMPILER`, honoured `CC`/`CXX`. A Clang build was stored under the
  GCC fingerprint. Even with a clean environment CMake picked `cc`/`c++`, which need not be GCC.
- **Ignored flags.** CMake initializes its flags from `CFLAGS`, `CXXFLAGS` and `LDFLAGS`; the
  fingerprint did not include them.
- **Header-only rebuilds.** Cache validation required `!metadata.artifacts.is_empty()`.

## 2. Compiler resolution

C and C++ are resolved independently, once per request. The same result feeds the
fingerprint, the CMake command and the metadata.

1. `-DCMAKE_<LANG>_COMPILER=` in the package's CMake arguments (CMake would apply it over
   anything Crux passes, so it is resolved and fingerprinted; Crux adds no second definition).
2. Explicit override: `BuildConfiguration`, then `with_c_compiler` / `with_cxx_compiler`.
3. `CC` / `CXX` (`extra_env` first, then the process environment). Empty means unset.
4. `gcc` / `g++`.
5. Fallback when GCC is unavailable: `cc`, then `clang` (`c++`, then `clang++`). This keeps
   CMake's earlier choice of `cc`/`c++`. If none works, nothing is passed and CMake searches.

Rules:

- A compiler must be executable and succeed with `--version`. A failing entry from steps 1 to 3
  is a `ToolchainDetectionFailed` error naming its source; there is no substitution.
- Paths are made absolute without following symlinks. No shell is used.
- `CC`/`CXX` may be a whole path (spaces allowed) or a program plus whitespace-separated
  arguments such as `ccache gcc`, passed as `-DCMAKE_C_COMPILER=<path>;gcc`. Quoting is not
  interpreted.
- With a CMake toolchain file, Crux does not force `CC`/`CXX` or the default, so the file stays
  in control. Explicit overrides are still passed, as before.

## 3. Fingerprint changes

`BuildFingerprint::compute` is unchanged. A length-prefixed, tagged section is appended only
when there is something to add:

- A compiler that is not the resolved `gcc`/`g++`: its path and arguments. (An absolute explicit
  override is already hashed by the existing format and is not added twice.)
- Each of `CFLAGS`, `CXXFLAGS`, `LDFLAGS` that is set, byte for byte. Unset adds nothing;
  explicitly empty adds an empty field, so it differs from unset. A value in `extra_env` is
  already hashed and takes precedence over the ambient one.

The same values are set (or removed) on the configure and build commands. No format-version bump.

**Compatibility.** The M0 golden digest
`sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe` is unchanged
(`tests/m0_fingerprint_golden.rs` passes unmodified). A new test shows that a clean default
environment produces the same digest through the engine as the old algorithm. `CC=gcc`, as set
by `nix develop`, resolves to the default and also keeps it.

## 4. Poisoned and legacy cache entries

An old entry built with `CC=clang` sits under the clean-environment GCC fingerprint, and its
metadata claims GCC. The metadata cannot be trusted, so the CMake tree is inspected.

- New builds record `selected_toolchain` (compiler paths, arguments, ambient flags) in
  `metadata.toml`. The field is optional, so old files still parse.
- **Entry with the record:** reused only if the recorded compiler is the selected one.
- **Entry without it:** `CMakeCache.txt` and `CMake*Compiler.cmake` must name the selected
  compiler, and CMake's compile and linker flags must be empty unless the request explains
  them (set variable, `extra_env` or CMake argument).
- "Same compiler" means the same path and arguments, or aliases of one binary reporting the
  same version after the program name, so healthy entries built via `cc` are kept.
- A mismatch discards the build tree and rebuilds. Missing evidence for a language is accepted
  (the project did not enable it).
- Entries made under a non-default compiler or flags are simply no longer addressed.

A project that sets CMake flags itself looks like a poisoned legacy entry: it is rebuilt once,
then has a record.

**Installations.** The install engine would reuse an installation made from the poisoned tree,
because the fingerprint is the same. The build engine therefore leaves `reinstall-required` in
the build root; the install engine then bypasses its cache and removes the marker after
publication. This also works if `crux build <pkg>` and `crux install` run separately.

**Not covered automatically:** `crux build` / `crux run` of an application accept an already
valid installation without invoking the build engine. Run `crux install` (add
`--profile debug` for debug) once with a clean environment to check existing installations.

## 5. Header-only caching

A cached build is reused when metadata parses, `success` is true, the format version and exit
codes are valid, the fingerprint matches, the toolchain check passes, the build directory
exists, and every recorded artifact exists with its recorded size. With no artifacts,
`CMakeCache.txt` must also exist. No placeholder artifacts are created.

`metadata.toml` is now removed before any build starts. Without this, a failed `--force`
rebuild would have left an earlier success record that an artifact-free package could reuse.

The format-version and exit-code checks are new and apply to compiled packages too.

## 6. Changes

| File | Change |
| --- | --- |
| `src/build/selection.rs` (new) | Resolution, environment capture, stale-tree detection |
| `src/build/engine.rs` | Uses the selection; cache rules; `with_environment` |
| `src/build/fingerprint.rs` | Optional environment section |
| `src/build/metadata.rs` | `SelectedCompiler`, `SelectedToolchain`, new optional field |
| `src/build/mod.rs`, `src/lib.rs` | Exports |
| `src/install/engine.rs` | Honours and clears the reinstall marker |
| `tests/toolchain_cache.rs` (new) | 22 regression tests |
| `docs/reference/architecture.md`, `CHANGELOG.md` | Behaviour documented |

No new dependency, manifest, lockfile, Nix or website change. Application builds are untouched.

## 7. Tests and verification

`tests/toolchain_cache.rs` uses fake compilers and a fake CMake with an injected environment,
so it needs no Clang and does not mutate the process environment. Three `real_` tests use real
CMake and GCC: header-only build and install; a wrapper path with spaces and `CC="env gcc"`;
and legacy-entry reuse and rejection. Real Clang was not exercised.

All commands ran inside `nix develop --command`.

| Command | Outcome |
| --- | --- |
| `cargo test --no-fail-fast --test toolchain_cache --test build --test install --test m0_build_commands --test m0_fingerprint_golden --test m0_metadata_compat --test terminal_output` | First run: 1 failure in the new test harness (`Text file busy` running a just-written fake script); all other tests passed. Fixed in the test helper. |
| `cargo test -q --test toolchain_cache` (8 consecutive runs) | 22 passed, 0 failed each |
| `cargo fmt --check` | Clean |
| `cargo clippy --all-targets -- -D warnings` | Clean |
| `cargo check` | Clean |
| `cargo test --no-fail-fast` | 35 suites: 746 passed, 2 failed, 0 ignored |

The two failures are the known ones in `tests/build_settings.rs`
(`address_and_undefined_sanitizers_instrument_the_binary`,
`unchanged_settings_reuse_the_build_tree_and_changes_invalidate_it`): the Nix GCC cannot link
sanitizer binaries (`undefined reference to __pointer_chk_guard@GLIBC_PRIVATE`).

## 8. Remaining limitations

- A CMake toolchain file, or a project that sets the compiler itself, can still choose a
  compiler the fingerprint does not describe. The toolchain file's contents are not hashed.
- Other variables are not tracked (`ASMFLAGS`, `NIX_CFLAGS_COMPILE`, `CMAKE_PREFIX_PATH`,
  `CPATH`, `LIBRARY_PATH`, and so on).
- Default compilers are identified by version line, not path, to keep the digest. Two different
  GCCs with identical lines share a fingerprint directory; switching between them (for example
  inside and outside `nix develop`) now rebuilds and reinstalls rather than reusing.
- An application tree is keyed by dependency fingerprints, which a poisoned-entry rebuild does
  not change. Use `crux build --force` afterwards to be certain it is relinked.
- Artifact validation is still by existence and size. Outputs the scanner does not classify
  are not checked.
- A compiler without a successful `--version` (such as MSVC `cl`) is rejected as an override.
- `CC`/`CXX` are validated even if the package does not enable that language.
- No inter-process locking, as before.

## 9. Decisions for your approval

1. **Public API:** `BuildMetadata.selected_toolchain` (breaks exhaustive struct literals), new
   types `SelectedCompiler` and `SelectedToolchain`, and `CMakeBuildEngine::with_environment`.
   `detect_toolchain` can now fail on an invalid override.
2. **Explicit default:** CMake now receives `gcc`/`g++` where it previously chose `cc`/`c++`.
3. **Install engine:** the small marker check in section 4.

M3 has not been started.

## 10. Suggested commit and PR text

```text
fix(build): fingerprint the compiler and flags CMake uses, and cache header-only builds
```

PR title: `fix: accurate toolchain fingerprints and header-only build caching`

PR description: Resolves package compilers before fingerprinting and passes them to CMake,
fingerprints `CFLAGS`/`CXXFLAGS`/`LDFLAGS`, invalidates cache entries known to be wrong, and
reuses successful builds with no artifacts. Clean-environment fingerprints are unchanged.
Adds `BuildMetadata.selected_toolchain`. Two sanitizer tests in `tests/build_settings.rs`
fail in the Nix shell, as they did before this change.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Depends on the uncommitted M1 work only in that both touch `src/install/engine.rs`; the M0
tests are used as the compatibility baseline.
