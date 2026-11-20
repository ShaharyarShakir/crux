# M22 Follow-up Report

**Date:** 2026-11-19  
**Branch:** `main`  
**Working tree:** repository root (`/home/shaharyar/01__git_repos/products/crux`)  
**Base commit:** `5f27615`  
**Status:** Implementation and verification complete; changes remain uncommitted.

---

## 1. Objective

Resolve the remaining M22 review issues before M23:

1. Remove the production test-catalog fallback so fresh projects resolve through real registries.
2. Add manifest-free `crux run` for single- and multi-file C/C++ sources.
3. Add a pure C reference application consuming a verified M22 package.
4. Update CLI, quickstart, architecture, and website documentation to distinguish Verified, Catalog only, and Deferred packages.
5. Run the complete verification matrix and record actual results.

No Git commits, pushes, merges, branch switches, worktree changes, resets, or cleans were performed.

---

## 2. Implementation summary

### 2.1 Production catalog/registry selection

- Removed the production in-memory test-catalog fallback from `src/cli/mod.rs`.
- Unified production resolution through `load_active_registry` / `load_active_catalog`.
- Registry selection order is now explicit:
  1. configured primary registry (`.crux/config.toml`, `CRUX_CONFIG_PATH`, or user config),
  2. `CRUX_REGISTRY_PATH`,
  3. project-local `registry/registry.toml`,
  4. official registry bootstrap.
- Added explicit offline mode via `CRUX_OFFLINE` (`1`, `true`, `yes`, `on`, case-insensitive). In a released binary without a bundled registry, offline bootstrap fails with an actionable diagnostic instead of silently falling back to fixtures. In this development tree, the bundled `<repo>/registry` checkout is reused, so offline bootstrap from a fresh project still succeeds.
- Changed `check_lockfile` to take `&dyn PackageCatalog` explicitly; production callers no longer receive an implicit test catalog.
- Kept `default_dev_catalog()` only for Rust unit tests and local fixture helpers; there are no production call sites.
- Added `tests/common/mod.rs` helpers that build real isolated Git registries for CLI integration tests.
- Migrated `tests/lock_sync.rs` and `tests/resolver.rs` CLI subprocess tests from fixture fallbacks to isolated Git registries.
- Added `tests/catalog_selection.rs` covering fresh-project bootstrap, broken configured registry, invalid direct override, and custom-registry regression behavior.
- Fixed catalog-error reporting: `load_active_catalog` and `load_catalog_for_dependency_edits` now write registry errors to the command's stderr stream before returning. This closes a gap where command-handler errors were returned to `main` but not printed by the top-level `run` dispatcher.

### 2.2 Manifest-free `crux run`

Extended `RunArgs`, `RUN_HELP_MESSAGE`, and `parse_run_args`:

- `--bin <source>`: shorthand for one `.c`, `.cpp`, `.cc`, or `.cxx` source file.
- `--sources <list>`: repeatable, comma-separated, or space-separated source list. A space-separated list ends at the next Crux option or `--`. Header files (`.h`, `.hpp`, `.hxx`, `.hh`, `.inl`, `.ipp`) may be listed so they are copied into the scratch source tree.
- Include directories: `-I <dir>`, `-I<dir>`, `--include-dir <dir>`, `--include-dir=<dir>`, and `--include <dir>`, all repeatable.
- Application arguments must follow `--`, for example `crux run --bin hello.c -- --flag value`.
- `--debug` / `--release`, profile defaults, argument forwarding, stdout/stderr forwarding, environment inheritance, working directory, and exit-status propagation are preserved.

Added `src/build/adhoc.rs`:

- Detects C versus C++ from source extensions and rejects mixed languages.
- Validates all source and include paths stay inside the current workspace.
- Copies ad-hoc sources and headers into `.crux/run/<target>-<hash>/src`.
- Synthesizes `ProjectConfig` / `AppBuildModel` and reuses `AppBuildEngine::execute`; no second build system was introduced.
- Deletes the ad-hoc scratch root before each run, so a failed build cannot leave a stale executable that is later executed.
- Adds `AppBuildModel.extra_include_dirs` in `src/build/app.rs`.

Added `tests/adhoc_run.rs` with 8 integration tests covering C, C++, multi-file/header builds, include syntax, missing sources, compile errors with no stale execution, unsupported extensions, argument forwarding/exit status, and manifest requirements in project mode.

Project-based `crux run` behavior was intentionally left unchanged; `tests/run.rs::build_failure_prevents_execution` still expects a previously built binary to remain on disk.

### 2.3 Pure C reference application

Added `examples/m22/c-zlib-roundtrip/`:

- `[project]` manifest with `language = "c"` and `standard = "c17"`.
- Pinned `crux.lock` using verified `zlib 1.3.1`.
- `.crux/config.toml` pointing at the in-repo official registry.
- `src/main.c` compressing and decompressing a string and printing `roundtrip_ok=true` and `compressed_bytes=39`.

All six M22 reference apps (the five existing C++ apps plus the new pure C app) were re-run successfully.

### 2.4 Documentation and website

Updated:

- `README.md`
- `ROADMAP.md`
- `docs/reference/cli-reference.md`
- `docs/reference/architecture.md`
- `docs/guides/quickstart.md`
- `docs/reports/package-ecosystem-expansion-report.md`
- `crux-site/src/content/docs/reference/cli.md`
- `crux-site/src/content/docs/guides/getting-started.md`
- `crux-site/src/content/docs/registry/packages.md`
- `crux-site/src/content/docs/project/roadmap.md`

Documentation now explicitly distinguishes:

- **Verified** — built, installed, integrated, and consumed by a real application on Linux x86_64.
- **Catalog only** — discoverable with verified source metadata, but rejected by `crux build` / `crux install` because no supported build backend is available.
- **Deferred** — tracked as a target but not registered as a normal linkable package.

Catalog-only entries are no longer presented as fully supported packages.

---

## 3. Verification results

All commands were executed in the Nix development shell unless otherwise noted.

### 3.1 Rust quality gates

| Command | Result |
|---|---|
| `nix develop --command cargo fmt --all -- --check` | pass (no diffs) |
| `nix develop --command cargo clippy --all-targets --all-features -- -D warnings` | pass, 0 warnings |
| `nix develop --command cargo check` | pass |
| `nix develop --command cargo test --all` | pass — 469 tests total, 0 failed |
| `nix develop --command cargo build --release` | pass (20.99 s) |

`cargo test --all` breakdown:

- Library unit tests: 203 passed
- `adhoc_run`: 8 passed
- `app_build`: 21 passed
- `build`: 12 passed
- `catalog_selection`: 4 passed
- `check`: 13 passed
- `cli`: 4 passed
- `cmake_integration`: 23 passed
- `dependency`: 14 passed
- `dependency_management`: 7 passed
- `init`: 16 passed
- `install`: 15 passed
- `lock`: 17 passed
- `lock_sync`: 15 passed
- `official_registry`: 6 passed
- `package`: 10 passed
- `registry`: 17 passed
- `resolver`: 23 passed
- `run`: 12 passed
- `source`: 22 passed
- `toolchain`: 7 passed
- Binary unit tests and doc tests: 0 tests

The targeted catalog-selection suite was also run explicitly after the error-reporting fix: 4/4 passed.

### 3.2 Website production build

Command:

```bash
cd crux-site
nix develop --command pnpm build
```

Result: pass. Astro produced 10 static pages, the Pagefind search index, and `sitemap-index.xml` in `crux-site/dist/`.

Pre-existing non-fatal warnings were emitted:

- `i18n` content collection does not exist or is empty.
- `docs → 404` entry was not found.

### 3.3 Reference applications

Executed with `target/debug/crux`, `CRUX_HOME=/tmp/opencode/m22-followup-crux-home`, and `CRUX_OFFLINE=1`. Install output reported reuse of the locally cached verified archives; no registry bootstrap network access was exercised.

| App | `check` | `install` | `build` | `run` | Runtime evidence |
|---|---|---|---|---|---|
| `examples/m22/c-zlib-roundtrip` | pass | pass (zlib 1.3.1 installed) | pass | pass | `roundtrip_ok=true`, `compressed_bytes=39` |
| `examples/m22/fmt-json-cli` | pass | pass (3 cached deps) | pass | pass (`-- --name Ada`) | JSON output with `"user": "Ada"` |
| `examples/m22/config-tools` | pass | pass (3 cached deps) | pass | pass (`-- --name demo`) | `name=demo`, `threads=4`, `runtime=linux` |
| `examples/m22/data-checks` | pass | pass (3 cached deps) | pass | pass | `cJSON packages=50`, `yyjson packages=50`, `pcre2 matched=true` |
| `examples/m22/test-frameworks` | pass | pass (4 cached deps) | pass | pass | Catch2, GoogleTest, and doctest all passed; benchmark executed with `benchmark_ready=true` |
| `examples/m22/logging-zip` | pass | pass (2 cached deps) | pass | pass | spdlog logged `roundtrip_ok=true`, `compressed_bytes=41` |

Note: application arguments require the `--` separator, e.g. `crux run -- --name Ada`. The earlier M22 report table omitted the separator in its shorthand notation.

---

## 4. Files created and modified

### Created

- `src/build/adhoc.rs`
- `tests/adhoc_run.rs`
- `tests/catalog_selection.rs`
- `tests/common/mod.rs`
- `examples/m22/c-zlib-roundtrip/` (`crux.toml`, `crux.lock`, `.crux/config.toml`, `src/main.c`)
- `docs/reports/registry-resolution-and-adhoc-run-report.md` (this report)
- M22 documentation/registry/reference-app files already present as untracked work in the working tree (50-package registry, `docs/reference/architecture.md`, `docs/reports/package-ecosystem-expansion-report.md`, official catalog guide, contributing docs, and the five original reference apps).

### Modified (follow-up-specific)

- `src/cli/mod.rs`
- `src/cli/args.rs`
- `src/cli/commands/run.rs`
- `src/cli/commands/add.rs`
- `src/cli/commands/build.rs`
- `src/cli/commands/install.rs`
- `src/cli/commands/integrate.rs`
- `src/cli/commands/lock.rs`
- `src/cli/commands/remove.rs`
- `src/cli/commands/resolve.rs`
- `src/build/app.rs`
- `src/build/mod.rs`
- `src/lib.rs`
- `src/registry/config.rs`
- `src/resolution/lockfile.rs`
- `tests/lock_sync.rs`
- `tests/resolver.rs`
- README, roadmap, CLI/architecture/quickstart docs, and website content listed in section 2.4.

`git diff --stat` for tracked files reported **33 files changed, 953 insertions(+), 249 deletions(-)** before this report file was added. Untracked M22 registry/example/docs files are not included in that statistic.

---

## 5. Known limitations and remaining issues

1. **Released offline binaries:** `CRUX_OFFLINE=1` requires an already configured or cached registry. The current development tree contains a bundled registry, so fresh-project offline bootstrap succeeds here; a released binary without that bundled checkout will fail with an actionable offline error.
2. **Catalog-only packages:** 33 entries have verified source metadata but remain non-buildable because Crux's package build engine currently supports CMake-backed definitions only.
3. **Deferred packages:** `musl` and `systemd` are intentionally not registered as normal linkable packages.
4. **Ad-hoc runs are dependency-free:** `--bin` / `--sources` compile local sources only; they do not consult `crux.toml` or resolve Crux dependencies. Include directories outside the source tree must be passed explicitly.
5. **Project-run stale binary behavior:** unchanged by design. Only ad-hoc scratch directories are wiped before a rebuild attempt.
6. **Top-level CLI error reporting is still handler-driven:** catalog-loading paths now write actionable errors to stderr, but `cli::run` does not yet provide a universal error printer for every command handler. A future cleanup should centralize this to prevent silent errors in newly added handlers.
7. **Website warnings:** the empty `i18n` collection and missing `docs → 404` entry are pre-existing, non-fatal Astro warnings.
8. **Benchmark output:** Google Benchmark reports CPU-scaling/ASLR noise warnings in Debug builds; this is expected and does not affect the pass result.
9. **M23 was not started:** no milestone work beyond the requested M22 follow-up was performed.

---

## 6. Git state and handoff

- Branch: `main`
- Base commit: `5f27615`
- No commits, pushes, pull requests, merges, rebases, cherry-picks, resets, cleans, branch switches, or worktree changes were performed.
- The working tree contains many uncommitted and untracked M22 changes, including the 50-package official registry, reference apps, reports, and the follow-up changes listed above.
- No dependency on another worktree or unmerged branch is known.

### Suggested Conventional Commit messages

If the repository owner chooses to split the work:

1. `fix(cli): remove in-memory catalog fallback from production resolution`
2. `feat(run): support manifest-free C/C++ source execution`
3. `test: cover catalog selection and ad-hoc run behavior`
4. `feat(examples): add pure C zlib reference application`
5. `docs: clarify M22 registry tiers and ad-hoc run usage`

### Suggested PR title

`Complete M22 follow-up: real registry resolution, manifest-free run, and pure C reference app`

### Suggested PR description

> Removes the production in-memory catalog fallback, adds explicit offline/registry diagnostics, and migrates CLI resolution tests to real isolated Git registries. Adds manifest-free single- and multi-file `crux run` with include directories, Debug/Release profiles, argument forwarding, and stale-artifact protection. Adds a pure C zlib reference app and updates CLI/quickstart/architecture/website docs to distinguish Verified, Catalog only, and Deferred packages. All Rust gates, the website production build, and all six M22 reference apps pass.
