# M1: Current Installation Selection and Path Dependency Rebuild

Date: 2026-11-27
Base: `main` at `38b1beb`, repository root (no branch or worktree created)
Git: nothing staged or committed. The untracked M0 files were not modified.

## 1. Root causes

**Stale installation selected.** `validate_package_installation` sorted the fingerprint
directories of a package version and returned the first valid one for the profile. A
fingerprint is a hash, so name order says nothing about which installation is current.

**Edited path dependencies not rebuilt.** `ensure_app_dependencies`, shared by `crux build`
and `crux run`, only installed when validation failed. An old installation stayed valid after
its source changed, and `crux lock` did not alter that.

## 2. Current-installation record

Location: `.crux/installed/<package>/<version>/current-<profile>.toml`

```toml
format_version = 1
package_name = "greet"
package_version = "1.0.0"
profile = "debug"
fingerprint = "sha256:<64 hex>"
source_checksum = "sha256:<64 hex>"   # optional
```

Decisions:

- **A separate file, one per profile.** `installed.toml`, `crux.lock` and both fingerprint
  algorithms are unchanged. Debug and release never rewrite each other's record.
- **Written by the install engine** (`CMakeInstallEngine::install`), so every install path
  records it: `crux install`, `crux add`, and the automatic installs from `build`/`run`.
- **Written on cache hits too.** Reverting a source reuses an older fingerprint directory,
  which must become current again.
- **Atomic**, through `core::filesystem::write_atomically` (temporary file, fsync, rename).
- **Removed before a new installation is published, written after.** An interruption between
  the two leaves no record, not one naming an older installation.
- **Not trusted blindly.** An unreadable or truncated record, an unknown format version, a
  malformed fingerprint (it is used as a directory name, and deserialization does not validate
  it, as M0 noted), a name, version or profile mismatch, or a missing or invalid target is an
  error. No other directory is substituted.

Old fingerprint directories are never deleted by this change.

## 3. Selection and legacy migration

| State | Behaviour |
| --- | --- |
| No installation | `PackageNotInstalled`; normal install |
| Valid record | That installation, whatever else exists |
| Record unreadable, stale or pointing to an invalid install | Error; `build`/`run` warn and reinstall, which rewrites the record |
| No record, exactly one valid install for the profile | Accepted; recorded by `build`, `run` or `integrate cmake` |
| No record, several valid installs | Error "none is recorded as current"; `build`/`run` reinstall |

Invalid sibling directories do not count towards ambiguity. A security violation is still
returned immediately.

Recovery goes through the normal install pipeline, so an unchanged package is normally a build
and install cache hit. If its source is no longer in the source cache, a locked Git or archive
dependency has to be fetched again.

`validate_package_installation` itself writes nothing.

## 4. Path dependency change detection

For each locked package with a `path+` source, `build`/`run` compute the tree digest of the
live directory (`compute_tree_sha256`, the function acquisition uses) and compare it with
`source_checksum` in the record.

| Result | Output | Action |
| --- | --- | --- |
| Equal | `Dependencies:  N installed [profile]` | None |
| Different | `changed: <name> <version> (path+...)` | Rebuild and reinstall |
| No recorded checksum (legacy) | `unrecorded: <name> <version> (path+...)` | Run the pipeline once; a cache hit if unchanged |

- The recorded checksum is that of the snapshot actually built. A source edited while building
  is detected on the next run.
- When only source edits are pending, only the edited packages are processed. Registry, Git
  and archive packages are not acquired, built or re-recorded.
- When any installation is missing or invalid, the whole lockfile is installed, as before.
- `crux.lock` is not rewritten for a source edit. Its path checksum stays stale until the next
  `crux lock`, which matches the existing mutable-source semantics.
- The application is relinked because its build fingerprint already includes the dependency
  fingerprints.
- If the rebuild fails, the command fails and the application is not run. The next run retries.

## 5. Changes

| File | Change |
| --- | --- |
| `src/install/current.rs` (new) | Record schema, read, write, remove; unit test |
| `src/install/engine.rs` | Record maintenance; `InstallRequest::source_checksum` and `with_source_checksum` |
| `src/install/mod.rs` | Module declaration |
| `src/integrate/validation.rs` | Record-based selection; legacy adoption; per-directory checks moved into `validate_candidate` |
| `src/cli/commands/build.rs` | Change detection and diagnostics |
| `src/cli/commands/install.rs` | Passes source checksums; crate-private `run_install_selected` |
| `src/cli/commands/integrate.rs`, `mod.rs` | Legacy migration |
| `tests/current_installation.rs` (new) | 16 regression tests |
| `docs/guides/direct-sources.md`, `docs/reference/architecture.md`, `CHANGELOG.md` | Behaviour documented |

Most of the line count in `validation.rs` is re-indentation: the per-directory checks keep
their logic and messages. No dependency, Nix or website change.

## 6. Tests

`tests/current_installation.rs` covers:

- Selection: no installation; one legacy installation (also beside a broken one); several with
  a record; record naming a missing installation; record naming an invalid one; several without
  a record; empty, truncated, mismatched and path-traversing records; a leftover temporary
  file; per-profile selection.
- End to end with real CMake: six successive source values, an unchanged rerun, a revert and an
  unchanged lockfile; `crux build`; debug and release tracking; recovery from a missing,
  ambiguous, truncated and stale record; a failing rebuild; a nested path dependency beside a
  pinned Git dependency whose branch moved; `integrate cmake` migration and refusal.
- The assessment's section 3.1 reproduction (`lock`, `install`, `run`, six times).

The end-to-end tests check the application's printed value, which comes from the linked
library, so they verify which installation was linked.

Registry-source behaviour has no new end-to-end test: the fixture registry points at an
unreachable URL. It takes the same non-path branch as the pinned Git case, and the existing
registry, install and integration tests pass.

### Results

All commands ran in `nix develop --command`.

| Command | Outcome |
| --- | --- |
| `cargo test --test current_installation` | 16 passed, 0 failed |
| `cargo test --lib current` | 2 passed (1 new, 1 existing test matching the filter) |
| `cargo test --test direct_sources --test cmake_integration --test install --test app_build --test run --test dependency_management --test m0_build_commands --test m0_fingerprint_golden --test m0_metadata_compat` | 112 passed, 0 failed |
| `cargo fmt --check` | Clean |
| `cargo clippy --all-targets -- -D warnings` | Clean |
| `cargo check` | Clean |
| `cargo test --no-fail-fast` | 724 passed, 2 failed, 0 ignored |

The two failures are in `tests/build_settings.rs`
(`address_and_undefined_sanitizers_instrument_the_binary`,
`unchanged_settings_reuse_the_build_tree_and_changes_invalidate_it`). M0 recorded the same two
on unchanged code: the Nix GCC cannot link sanitizer binaries
(`undefined reference to __pointer_chk_guard@GLIBC_PRIVATE`).

Baseline check: the new test file, run against a copy of unmodified `38b1beb`, gave 2 passed
and 14 failed. That run predates the final `unrecorded:` assertion.

## 7. Remaining limitations

- A package depending on an edited path package is not rebuilt. Package builds cannot see their
  Crux dependencies yet (assessment 3.4, M3).
- Toolchain, CMake, recipe and Crux changes still do not trigger a rebuild of an installed
  dependency (assessment 3.2 and 3.3; M2 and later).
- Files outside the dependency directory, and the ignored root directories, are not tracked.
- Each path dependency is hashed once more per `build`/`run`; large trees will notice.
- Every distinct source state leaves an installation, build tree and source snapshot behind.
- There is no inter-process lock. Concurrent installs of one project are not coordinated.
- An older Crux used on the same project does not update records, so a newer one would keep
  following the record for registry and Git packages.
- `crux integrate cmake` does not detect source edits, and a previously generated
  `CruxDependencies.cmake` keeps naming the old directory until regenerated.
- `crux build <package>` and legacy `[package]` builds only build; they do not change what is
  current.
- The existing "Run 'crux install --force'" hint does not name the profile, so for debug it
  should be `crux install --profile debug`.

## 8. Decisions for your approval

1. **Public API.** `InstallRequest` gained the public field `source_checksum`. Code using
   `InstallRequest::new` is unaffected; a struct literal would no longer compile. The
   alternative is a separate method on the install engine.
2. **Error variant.** Ambiguous and stale records reuse
   `IntegrationError::CorruptInstallation` to avoid adding a variant to a public enum.
3. **Stricter `integrate cmake`.** It now fails on an ambiguous or stale record where it
   previously picked a directory. It does not reinstall by itself.
4. **Pruning.** Not implemented. A later explicit command could remove non-current
   directories.
5. **Rebuilding dependents** of an edited path package is left to M3.

M2 has not been started.

## 9. Suggested commit and PR text

```text
fix(install): select the recorded current installation and rebuild edited path dependencies
```

PR title: `fix: use the current dependency installation and rebuild edited path dependencies`

PR description: Records the current installation per package version and profile instead of
taking the first fingerprint directory by name, and makes `crux build`/`run` rebuild local path
dependencies whose source changed. Adds `InstallRequest::source_checksum`. Two sanitizer tests
in `tests/build_settings.rs` fail in the Nix shell, as they did before this change.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

No dependency on other unmerged branches. The M0 files are independent and still untracked.
