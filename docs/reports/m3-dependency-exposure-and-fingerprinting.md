# M3: Dependency Exposure and Dependency Fingerprinting

Date: 2026-11-27
Base: `main` at `38b1beb`, repository root, on top of the uncommitted M0, M1 and M2 work
Git: nothing staged or committed; no branch or worktree created.

Updated 2026-11-28 with the reviewed decisions (section 9).

## 1. Original problem

From section 3.4 of the 2026-11-27 assessment:

- `BuildRequest` carried no dependencies, and no CMake search path was set for package builds.
  With `mid` depending on `leaf`, `crux lock` succeeded and `leaf` installed, then `mid` failed
  with `Could not find LEAF_LIB`.
- `BuildFingerprint` had no dependency input, so a changed dependency left its dependents'
  cache entries and installations valid.

The 11 tests first written for `tests/dependency_exposure.rs` all failed against a copy of the
pre-M3 tree, mostly with `package configuration failed`. The tests added on review were not
run that way.

## 2. Architecture

| Piece | Responsibility |
| --- | --- |
| `build/package_dependencies.rs` (new, crate-private) | Dependency closure, validated installations, search paths, fingerprint fields |
| `build/engine.rs` | `build_with_dependencies`; `BuildEngine::build` delegates with none |
| `build/fingerprint.rs` | Optional dependency section |
| `install/engine.rs`, `install/current.rs` | Record what an installation was built against |
| `cli/commands/` | Supply dependencies in every package build path; rebuild dependents |

Lifecycle per package, in topological order: collect the locked closure, validate each
dependency's current installation, fingerprint, configure and build, install, record. A
dependency without a usable installation stops the command before CMake runs. Validating a
dependency reads only its own installation, so there is no cycle with fingerprint calculation.

No CMake, `.pc` or cache file of a dependency is parsed. No new crate, manifest or lockfile
change, installation layout change or resolver change.

## 3. How dependency prefixes are selected and exposed

The closure is every direct and transitive dependency:

- Lockfile flows (`crux install`, `crux build`, application `build`/`run`/`add`): from
  `crux.lock`.
- `crux install <pkg>` and `crux build <pkg>`: from package definitions, preferring locked
  versions. Two versions of one package in a closure are now an error.

Each dependency's installation is the one `validate_package_installation` selects (M1's current
record), and paths come only from that prefix and its `installed.toml` inventory.

| Setting | Content | Passed as |
| --- | --- | --- |
| `CMAKE_PREFIX_PATH` | Every dependency prefix, by package name | Environment, configure and build |
| `PKG_CONFIG_PATH` | Inventory directories holding `.pc` files | Environment, configure and build |
| `CMAKE_MODULE_PATH` | Inventory directories holding `Find*.cmake`, or `.cmake` files in `Modules/` | `-D` argument |

- `CMAKE_MODULE_PATH` is an argument because CMake does not read it from the environment.
- Crux entries come first. Existing values (`extra_env`, else the process environment) are kept
  after them, since builds already inherited those; Crux adds no host path of its own.
- A `CMAKE_MODULE_PATH` definition in the recipe's CMake arguments is kept, ahead of Crux's.
- An empty category adds nothing. A package with no dependencies gets no Crux entry at all.
- The inherited values are captured once, then both fingerprinted and passed to CMake.

`crux build <pkg>` and `crux install <pkg>` share one per-package installation step
(`PackageInstaller`). `build` runs it over the dependencies in dependency order, then only
builds the requested package. Unchanged dependencies are cache hits, and edited path
dependencies are picked up. Dependencies listed in `crux.lock` are acquired with checksum
verification, and an unreadable lockfile is an error.

## 4. Dependency fingerprints

For each dependency in the closure, ordered by name: name, version and the fingerprint of the
selected installation. The fields are length-prefixed in a section headed
`crux-package-deps-v1`, appended after the M2 section, only when dependencies exist.

- Not included: dependency paths, timestamps, sizes or other installation metadata.

Inherited `CMAKE_PREFIX_PATH` and `PKG_CONFIG_PATH` join the M2 environment section, byte for
byte, so order matters and an empty value differs from an unset one. This applies to every
package, with or without dependencies, because it affects what any of them can discover. A
value supplied through `extra_env` is already hashed and takes precedence.

- Dependency-free digests with neither variable set are unchanged. The M0 golden passes unmodified, and the M2
  clean-environment test still passes. No format bump.
- A second golden pins the new encoding. Its value was computed outside Rust.
- Packages with dependencies get new digests, so no pre-M3 cache entry of theirs is reused.
- Where either variable is set, every package gets a new digest once. This applies to the
  development machine used here, where the Nix shell has `PKG_CONFIG_PATH=/usr/lib/pkgconfig`.

## 5. How transitive changes propagate

Two mechanisms:

1. **Fingerprints.** A changed `leaf` changes `leaf`'s fingerprint, which changes `mid`'s, which
   changes `top`'s. Each also hashes its whole closure directly.
2. **Deciding to rebuild.** `crux build`/`run` of an application otherwise accept any valid
   installation without invoking the engine. The current record now has an optional
   `dependencies` table (`name = "<version>@sha256:<hex>"`). A package is reprocessed when it
   differs from the current dependency installations, or when any dependency of it is being
   reprocessed. These are reported as `outdated: <name> <version>`.

Because the comparison is persisted, it also works after a failed dependent rebuild, or after
`crux install <dependency>` on its own. A missing table on a package that has dependencies
means "unknown" and is not trusted. Reverting a dependency selects the older dependent build
again from cache. Packages that do not depend on anything changed are left alone.

The record keeps `format_version = 1`. The field is omitted when empty, and an older Crux
ignores it.

## 6. Public API compatibility

**No public type, field, signature or export changed in M3.** `BuildRequest` is untouched, so
struct literals still compile. All new entry points are crate-private. M2's types and APIs are
unchanged.

The consequence: library callers cannot supply dependencies. `BuildEngine::build` and
`InstallEngine::install` behave as before and treat a package as having none. Exposing this
would be a separate API decision.

Behaviour changes visible to users:

- `crux build <pkg>` now builds and installs the declared dependencies before building the
  package. Before, it built without them, possibly against system copies. This changes their
  current records.
- `crux build` with a lockfile only, or a legacy `[package]` manifest, now **installs** packages
  that other locked packages depend on, which changes their current record. Others are still
  only built.
- Application `build`/`run` also reprocess dependents, including registry and Git ones. This
  may acquire their sources again if the source cache lost them.
- After upgrading, each installed package that has dependencies is processed once.
- `BuildEngine::build` now includes the two inherited variables in the fingerprint, and
  `CMakeBuildEngine::with_environment` covers them too. No signature changed.

## 7. Tests

`tests/dependency_exposure.rs` (13 tests, real CMake, GCC and pkg-config, local path packages
only):

- `app -> mid -> leaf`: header discovery and linking (including a check executable inside
  `mid`'s build), exact search paths, install order, unchanged rerun, changed `leaf`, revert.
- A three-level graph where `top` declares only `mid` but also links `leaf`.
- Discovery by prefix, by find module, by pkg-config (with CMake's prefix fallback off) and by
  an `install(EXPORT)` config package.
- Two header-only dependencies: ordering, preserved existing values, absent optional
  directories, dependency-free packages untouched.
- A failed dependent rebuild; a dependency installed on its own; a record without dependencies
  and a missing record.
- `crux build <pkg>` installing, reusing and updating its dependencies; refusal of a locked
  dependency with a wrong checksum or an unreadable lockfile; lockfile-only `crux build`.
- Changed, reordered, emptied and removed inherited paths against the build cache.

One test added to `tests/toolchain_cache.rs`: each inherited variable unset, empty and
reordered, and `extra_env` precedence.

The checksum test was confirmed to fail with the locked verification temporarily disabled.

Unit tests (5): both goldens; each dependency input changes the digest and order does not;
install location is not hashed; module and pkg-config directories from an inventory.

**One existing test changed.** `only_edited_path_packages_are_reinstalled` (M1) asserted that a
dependent of an edited package was *not* reinstalled, which M1 deferred to M3. It is renamed
`only_edited_path_packages_and_their_dependents_are_reinstalled` and now expects the dependent
to be rebuilt. Its assertions that the unrelated pinned Git package is untouched remain.

## 8. Verification

All commands ran inside `nix develop --command`.

| Command | Outcome |
| --- | --- |
| `cargo test --no-fail-fast` before M3 changes | 746 passed, 2 failed |
| `cargo fmt --check` | Clean |
| `cargo clippy --all-targets -- -D warnings` | Clean |
| `cargo check` | Clean |
| `cargo test --no-fail-fast` | 36 suites: 765 passed, 2 failed, 0 ignored |
| `cargo test -q --test dependency_exposure --test current_installation --test toolchain_cache` (8 consecutive runs) | 13, 16 and 23 passed, 0 failed each |

The two failures, present before and after, are in `tests/build_settings.rs`
(`address_and_undefined_sanitizers_instrument_the_binary`,
`unchanged_settings_reuse_the_build_tree_and_changes_invalidate_it`): the Nix GCC cannot link
sanitizer binaries (`undefined reference to __pointer_chk_guard@GLIBC_PRIVATE`).

Only Linux with GCC was exercised. Shared-library dependencies of a package build have no
dedicated test.

## 9. Decisions and limitations

### Reviewed decisions

| # | Question | Decision | State |
| - | --- | --- | --- |
| 1 | `crux build <pkg>` with missing dependencies | Install them, through the existing path | Implemented |
| 2 | Lockfile-only `crux build` installing dependencies | Accept: `build` ensures prerequisites then builds; `install` also installs the requested package | Documented |
| 3 | Dependency API visibility | Keep internal until a real library use case exists | No change |
| 4 | Inherited CMake and pkg-config paths | Preserve and fingerprint | Implemented |
| 5 | Same fingerprint, different installed content | Defer, tracked below | Open |

### Open correctness issue: same fingerprint, different content

A dependency reinstalled under an unchanged fingerprint (`--force` after an untracked input
changed, or M2's stale-tree rebuild) does not invalidate its dependents. Until fixed, use
`crux build --force` or `crux install --force`.

Proposed fix: a deterministic digest of the installed content, computed from the inventory and
file contents, including relevant file metadata (such as the executable bit) and symlink
targets. Record it once at installation so later builds do not rehash the files, and add it to
the dependency fingerprint fields and records. Open points: where to store it without changing
public structs, how to treat installations that have none, and outputs that are not
reproducible. This report is the only record of it; no tracker issue was filed.

### Limitations

- **Inherited paths are not hermetic.** Their values are fingerprinted, not what they point to.
  A library replaced in place under the same path is not detected.
- **Application `build`/`run` does not notice a changed inherited path.** As with `CC` and
  `CFLAGS`, an installed dependency stays valid. `crux install` or `crux build <pkg>` does
  notice.
- Other discovery inputs are not tracked, among them `PKG_CONFIG_LIBDIR`,
  `PKG_CONFIG_PATH_FOR_TARGET` (Nix), `<Package>_ROOT`, `<Package>_DIR`, `CMAKE_INCLUDE_PATH`,
  `CMAKE_LIBRARY_PATH` and `PATH`.
- The application's own CMake build is unchanged: these variables are not in its fingerprint.
- **Checksum verification is uneven.** `crux install <pkg>` still does not check any source
  against `crux.lock`, and neither does `crux build <pkg>` for the requested package itself
  (assessment 3.5, predating M3). Worth a separate security change.
- **`.pc` files are used as installed.** One that records the configure-time prefix instead of
  being relocatable points outside the Crux prefix.
- A recipe or toolchain file can override discovery: `CMAKE_MODULE_PATH` set without appending,
  its own `-DCMAKE_PREFIX_PATH`, `HINTS`, or `CMAKE_FIND_ROOT_PATH_MODE_* = ONLY`.
- System libraries stay visible. Crux prefixes are searched early but this is not isolation.
- `crux install <dependency>` does not rebuild installed dependents; the next application
  `build`/`run` does. `crux integrate cmake` does not detect outdated dependents.
- Runtime lookup of shared dependencies by installed package binaries was not addressed.
- Each application `build`/`run` validates each dependency closure, which grows quadratically
  with deep graphs.
- No inter-process locking, as before.

M4 has not been started.

## 10. Changes made by M3

| File | Change |
| --- | --- |
| `src/build/package_dependencies.rs` | New |
| `src/build/{engine,fingerprint,mod,selection}.rs` | Dependency-aware build and digest; inherited search paths; `cmake_definition` made crate-visible |
| `src/install/{engine,current}.rs` | Recorded dependencies |
| `src/integrate/validation.rs` | Empty field when adopting a legacy installation |
| `src/cli/commands/{build,install,mod}.rs`, `src/cli/report.rs` | Wiring and rebuild of dependents |
| `tests/dependency_exposure.rs` | New |
| `tests/current_installation.rs` | One test updated (section 7) |
| `tests/toolchain_cache.rs` | One test added |
| `docs/reference/{architecture,cli-reference}.md`, `docs/guides/direct-sources.md`, `CHANGELOG.md` | Behaviour documented |

M3 builds on the uncommitted M1 record and M2 fingerprint section, so it cannot be committed
ahead of them.

## 11. Suggested commit and PR text

```text
feat(build): expose dependencies to package builds and fingerprint them
```

PR title: `feat: dependency exposure and dependency-aware package fingerprints`

PR description: Package builds now see their direct and transitive Crux dependencies through
`CMAKE_PREFIX_PATH`, `PKG_CONFIG_PATH` and `CMAKE_MODULE_PATH`, taken from installed
inventories. Dependency installations are part of the package fingerprint, and dependents of a
changed package are rebuilt. Inherited `CMAKE_PREFIX_PATH` and `PKG_CONFIG_PATH` are
fingerprinted when set; clean-environment dependency-free fingerprints are unchanged, and no
public API changes. `crux build <pkg>` now installs its dependencies first, and lockfile-only
`crux build` installs depended-on packages. Two sanitizer tests in `tests/build_settings.rs` fail in the Nix
shell, as they did before this change.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
