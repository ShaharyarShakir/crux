# M24 — Direct Package Sources and Release Readiness Report

**Milestone**: M24 — Direct Package Sources & v0.2.0 Release Preparation  
**Date**: 2026-11-20  
**Target Release**: v0.2.0  
**Repository State (at report authoring time)**: Dirty working tree on `main`; no Git state mutations performed  
**Scope**: Git, SHA-256-verified archive, and local-path dependencies through the existing Crux resolver, lockfile, acquisition, build, install, and CMake integration pipeline; Phase B stabilization and release documentation.

---

## Amendment — v0.2.1 Hotfix (2026-11-20)

> **Read this first**: the body of this report is a historical repository-state snapshot **at report authoring time** (immediately before the v0.2.0 tag). It is preserved for provenance; the statuses, gate results, and file lists below describe that snapshot, not the current tree.

**What happened after authoring**: the repository owner created and pushed the `v0.2.0` tag, and the installer successfully installed the v0.2.0 release binary. **v0.2.0 remains tagged and published**; no history was moved or rewritten.

**Audit verdict for v0.2.0**: **NOT READY FOR RELEASE — BLOCKERS REMAIN** with respect to the follow-up security/documentation work identified by a read-only audit of the released tree: high-severity symlink escape in direct/path source tree validation; high-severity `--output` documentation/CLI mismatch; invalid registry Git `rev` could panic; HTTP archives were accepted despite TLS-only documentation; release notes/roadmap/checklist inconsistencies; missing direct-source security coverage; and this report not being indexed.

**Final status: READY FOR OWNER RELEASE REVIEW.** A forward security/documentation hotfix has been prepared in the working tree for v0.2.1. All required verification gates passed: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo check`, `cargo test --all` (**522 passed, 0 failed**), `cargo build --release`, and the Astro site build (**11 pages built successfully**). The release binary reports `crux 0.2.1`. Smoke tests confirmed that a root-level escaping symlink in a direct local-path source is rejected by `crux lock`, and that `crux integrate cmake --output <path>` writes the custom file and prints the matching CMake `include()` path.

All v0.2.1 changes remain uncommitted in the primary working tree on `main`. No branch, worktree, stage, commit, push, tag, release, publication, merge, rebase, cherry-pick, reset, or discard operation was performed by the agent. Creating the `v0.2.1` tag, publishing artifacts, and creating the GitHub release remain owner-only actions.

**Hotfix scope**: version metadata and documentation updates were tracked alongside the corresponding source and test changes. The historical report body below is retained unchanged except for explicit historical labelling.

---

## 1. Final Status (at report authoring time — historical)

**READY FOR OWNER RELEASE REVIEW** *(historical status at report authoring time; superseded by the v0.2.1 amendment above)*

At report authoring time, all required Rust quality gates passed, all 515 automated tests passed, the v0.2.0 release binary built, and the documentation website built. Release tagging, artifact publication, and GitHub release creation remained exclusively under repository-owner control and had not been performed. The owner subsequently created and pushed the v0.2.0 tag and confirmed the installer succeeded.

---

## 2. Feature Branch, Worktree, and Base

| Item | Value |
|---|---|
| Feature branch | None created. Work remained on the existing branch. |
| Active branch | `main` |
| Worktree location | `/home/shaharyar/01__git_repos/products/crux` (primary working tree) |
| Base branch | `main` |
| Base commit | `f135524d05a924d0fa7be9f199ad7916623a1509` |
| Git operations performed | Read-only inspection only. No branch, worktree, stage, commit, push, pull request, merge, rebase, cherry-pick, reset, tag, release, or publish operation was executed. |

---

## 3. Files Created and Modified

### 3.1 New implementation and test files

- `src/resolution/source.rs` — canonical source grammar, direct-source parsing/validation, Git ref validation, identity keys, and immutable commit attachment.
- `src/source/direct.rs` — `ActiveCatalog` and `DirectCatalog` overlay; source conflict/cycle detection; lockfile commit propagation.
- `src/source/tree.rs` — deterministic source-tree hashing and validated tree copying.
- `tests/direct_sources.rs` — 15 offline integration tests covering direct-source parsing, Git commit pinning, archive verification, path handling, conflicts, moved projects, and the full build/install/integrate pipeline.
- `docs/guides/direct-sources.md` — repository direct-source guide.
- `crux-site/src/content/docs/guides/direct-sources.md` — website direct-source guide.
- `docs/releases/v0.2.0-release-notes.md` — v0.2.0 release notes.
- `docs/reports/m24-direct-sources-and-release-report.md` — this report.

### 3.2 Modified implementation files

- `src/cli/args.rs`, `src/cli/mod.rs`, `src/lib.rs`
- `src/cli/commands/add.rs`, `build.rs`, `fetch.rs`, `install.rs`, `integrate.rs`, `lock.rs`, `remove.rs`, `resolve.rs`
- `src/project/manifest.rs`
- `src/registry/git.rs`, `metadata.rs`, `mod.rs`
- `src/resolution/lockfile.rs`, `mod.rs`
- `src/source/cache.rs`, `mod.rs`
- `tests/source.rs`, `tests/official_registry.rs`

### 3.3 Modified release and documentation files

- `Cargo.toml`, `Cargo.lock`, `flake.nix`, `install/install.sh`, `.github/workflows/release.yml`
- `README.md`, `CHANGELOG.md`, `ROADMAP.md`
- `docs/README.md`
- `docs/guides/installation.md`
- `docs/reference/architecture.md`, `cli-reference.md`, `security-model.md`
- `docs/releases/release-checklist.md`
- `docs/reports/project-import-migration-report.md`
- `crux-site/astro.config.mjs`
- `crux-site/src/components/SiteFooter.astro`, `SiteHeader.astro`, `Terminal.astro`
- `crux-site/src/content/docs/design/architecture.md`, `design/security.md`
- `crux-site/src/content/docs/guides/installation.md`
- `crux-site/src/content/docs/project/changelog.md`, `project/roadmap.md`
- `crux-site/src/content/docs/reference/cli.md`
- `crux-site/src/pages/index.astro`

---

## 4. Implementation Summary

### 4.1 One resolution pipeline

Direct sources were normalized into the existing domain model rather than implemented as a parallel package manager:

- Manifest direct-source tables are parsed into the existing `Dependency` type and its `source` field.
- Direct packages are converted into `CatalogEntry` values by `DirectCatalog`.
- `ActiveCatalog` composes the Git registry with direct sources and implements `PackageCatalog`, so `Resolver` remains completely storage-agnostic.
- `PackageDefinition` was extended to represent direct Git, archive, and path sources.
- `ResolvedGraph::to_lockfile` records canonical `PackageSource` identities exactly as it does for registry packages.

The resolver still enforces a single version per package name, preserving C/C++ ODR safety.

### 4.2 Canonical source identity and validation

`src/resolution/source.rs` provides:

- Canonical forms:
  - `git+<url>#tag=<tag>`
  - `git+<url>#branch=<branch>`
  - `git+<url>#rev=<rev>`
  - `git+<url>#commit=<sha>`
  - legacy `git+<url>?rev=<sha>`
  - `archive+<url>#sha256=<digest>`
  - `path+<path>`
- `GitRef::validate` rejects leading-dash, control-character, traversal, and other unsafe Git ref forms before any Git subprocess executes.
- `DirectSource::with_resolved_commit` attaches the immutable full commit after Git acquisition.
- `identity_key()` separates dependency identity from acquisition state, so a Git commit change does not invalidate lockfile identity while the URL and requested ref remain unchanged.
- Path normalization and archive URL/digest validation are centralized.

### 4.3 Git immutability

- Tags, branches, arbitrary revisions, and `HEAD` are resolved to a full 40-character commit.
- `crux.lock` records both the requested ref and the resolved commit.
- Re-running `crux lock` reuses the locked commit; upstream movement does not silently change the dependency.
- `crux lock --update` is the only supported explicit refresh path.
- Pinned sources are checked out with `GitOps::clone_and_pin_commit`; Git arguments are discrete argv entries with null stdin and a hard timeout.

### 4.4 Archives and source trees

- Remote archives require SHA-256. Verification occurs before extraction using the existing download/cache/extraction subsystem.
- Existing Zip Slip, symlink escape, path traversal, file-count, and decompression-bomb guards remain active.
- Git and local-path sources receive real deterministic SHA-256 source-tree digests through `src/source/tree.rs`; no placeholder checksums are generated.
- Nested `path` dependencies inside direct sources are rebased relative to the consuming project.
- Local paths are canonicalized and validated; missing directories and missing `package.toml` files fail with actionable diagnostics.

### 4.5 Conflict and cycle detection

- A package name may come from the registry or from one direct source, never both.
- Two direct sources providing the same package name with different identities are rejected before writes.
- Direct-source dependency cycles are detected and reported.
- Invalid, ambiguous, and unsafe source declarations are rejected during `crux check`, `crux lock`, and `crux resolve`.

### 4.6 CLI and manifest behavior

- `crux add` accepts `--git` with `--tag`, `--branch`, `--rev`, or `--commit`; `--url` with mandatory `--sha256`; and `--path`.
- Manifest dependencies use minimal one-line inline tables, for example `{ git = "...", tag = "..." }`.
- Plain string registry dependencies remain valid and unchanged.
- `crux add` and `crux remove` remain transactional: the lockfile is constructed before writing, and the manifest is restored on failure.
- All source-aware commands (`lock`, `resolve`, `fetch`, `build`, `install`, `integrate cmake`) load `ActiveCatalog` through the shared CLI helper.

### 4.7 Documentation and release engineering

- Added repository and website direct-source guides, including lockfile semantics, moved-project behavior, and limitations.
- Expanded CLI, architecture, and security references for direct sources and the `ActiveCatalog` overlay.
- Added v0.2.0 release notes and expanded `CHANGELOG.md`.
- Updated version metadata to 0.2.0 across `Cargo.toml`, `Cargo.lock`, `flake.nix`, installer defaults, release workflow examples, documentation, and website badges/copies.
- Updated `ROADMAP.md` and website roadmap/changelog entries through M24 / v0.2.0.

---

## 5. Phase B Audit Results

| Audit | Result | Evidence |
|---|---|---|
| Direct-source architecture | PASS | Resolver remains decoupled; `ActiveCatalog` implements `PackageCatalog`; no parallel package-management path exists. |
| Deterministic serialization | PASS | Repeated `crux lock` runs are byte-for-byte identical for local-path, Git, and mixed registry/direct graphs. |
| Source integrity | PASS | Archive SHA-256 mismatch fails before extraction; Git/local sources use real tree digests. |
| Git immutability | PASS | Locked commits survive upstream changes; `crux lock --update` explicitly advances them. |
| Conflict and cycle safety | PASS | Registry/direct conflicts, direct/direct conflicts, and direct-source cycles are rejected. |
| Transactional edits | PASS | `crux add` updates manifest and lockfile together; parser rejects ambiguous options without writes. |
| M23 `.gitignore` verification | PASS | `.gitignore` uses anchored `/build/`; `git check-ignore -v src/build/*.rs` produced no match. Stale prose in `docs/reports/project-import-migration-report.md` was corrected. |
| Version metadata audit | PASS | `target/release/crux --version` reports `crux 0.2.0`; `Cargo.lock`, `flake.nix`, installer, workflow examples, docs, and website are aligned. Historical v0.1.0 references were intentionally retained in old reports and compatibility notes. |
| Documentation/site audit | PASS | Direct-source guide is linked from `README.md`, `docs/README.md`, the website sidebar, CLI docs, architecture docs, and security docs. |
| Release notes/changelog audit | PASS | `CHANGELOG.md`, `docs/releases/v0.2.0-release-notes.md`, website changelog, and release checklist reflect v0.2.0. |
| Owner-only release boundary | PASS | No Git branch/worktree/stage/commit/tag/push/PR/merge/release operation was performed. |

---

## 6. End-to-End Scenarios A–H

All scenarios below were executed offline through the real `crux` binary using temporary local Git repositories, generated archives, and local package directories. No external network service was contacted.

| Scenario | Coverage | Test(s) | Result |
|---|---|---|---|
| **A — Git tag source is resolved and locked** | Tag resolves to a full commit; lockfile is deterministic and unchanged after upstream movement. | `git_tag_source_is_resolved_to_an_immutable_commit` | PASS |
| **B — Git branch stability and explicit update** | Branch commit stays locked; `crux lock --update` advances version and commit only when requested. | `git_branch_commit_is_stable_until_explicit_update` | PASS |
| **C — Git revision/commit pinning and ref failures** | Short revision and full commit lock to the same immutable SHA; missing refs produce actionable errors. | `git_rev_and_commit_sources_are_locked`, `git_ref_failures_are_actionable` | PASS |
| **D — Archive SHA-256 verification** | Matching digest extracts a valid package; mismatched digest returns `ChecksumMismatch` before extraction. | `archive_source_requires_matching_sha256_before_extraction` | PASS |
| **E — Local path source validation and determinism** | Path source locks, checks, resolves, and serializes deterministically; missing paths, missing manifests, ambiguous tables, and unsafe refs are rejected. | `local_path_source_is_locked_and_deterministic`, `invalid_and_ambiguous_direct_sources_are_rejected`, `direct_source_parser_identity_and_validation_rules` | PASS |
| **F — Source conflicts, cycles, and mixed graphs** | Registry/direct and direct/direct conflicts fail; path cycles fail; mixed registry/direct lockfiles are deterministic. | `direct_sources_conflict_with_registry_and_each_other`, `path_source_cycles_are_rejected`, `mixed_direct_and_registry_sources_are_deterministic` | PASS |
| **G — `crux add` direct-source flows** | Git, archive, and path flags parse; ambiguous/missing options fail; path and tag adds update manifest and lockfile transactionally. | `add_flags_support_git_and_path_sources_and_reject_ambiguous_options`, `add_git_and_path_direct_sources_update_manifest_and_lockfile` | PASS |
| **H — Moved workspace and full pipeline** | Moving the project with its local package preserves `lock --check` and `build`; full `lock → build → install --locked → integrate cmake --locked` works and emits correct dependency data. | `moved_project_preserves_relative_path_sources`, `direct_source_build_install_and_integrate_pipeline_works` | PASS |

The dedicated `tests/direct_sources.rs` target reports **15 passed; 0 failed**.

---

## 7. Tests Executed and Actual Results

### 7.1 Required Crux quality gates

| Command | Result | Actual output summary |
|---|---|---|
| `nix develop --command cargo fmt --all -- --check` | PASS | No formatting divergence. |
| `nix develop --command cargo clippy --all-targets --all-features -- -D warnings` | PASS | `Checking crux v0.2.0`; `Finished dev profile`. Zero warnings and zero errors. |
| `nix develop --command cargo test --all` | PASS | **515 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out.** Includes 218 library tests and all integration targets. |
| `nix develop --command cargo check` | PASS | `Finished dev profile`. |

### 7.2 Additional verification

| Command | Result | Actual output summary |
|---|---|---|
| `nix develop --command cargo fmt --all` | PASS | Used to normalize formatting before the check gate. |
| `nix develop --command cargo check --all-targets` | PASS | Clean check across all targets. |
| `nix develop --command cargo build --release` | PASS | `Finished release profile [optimized] target(s)`. |
| `target/release/crux --version` | PASS | `crux 0.2.0` |
| `nix develop --command pnpm build` (in `crux-site`) | PASS | 11 static pages built; Pagefind index and sitemap generated. |

The website build emitted two non-fatal Astro content warnings: the empty `i18n` collection and a missing `docs → 404` entry. Both predate M24 and did not fail the build.

---

## 8. Git Diff Summary

At report authoring time, excluding this newly created report:

- **47 tracked files modified**
- **1,435 insertions**
- **490 deletions**
- **7 new implementation/documentation/test files** (`source.rs`, `direct.rs`, `tree.rs`, `tests/direct_sources.rs`, two direct-source guides, and v0.2.0 release notes)
- **1 additional new file**: this M24 report

`git diff --check` reports trailing whitespace on four Markdown metadata lines where two spaces are intentionally used as hard line breaks. This does not affect any required Rust gate and matches the existing documentation style.

---

## 9. Known Issues and Limitations

- HTTPS and SSH Git sources are supported where the host Git executable and credentials permit them, but M24 tests deliberately use local repositories. No external Git service was contacted.
- No OS-level sandbox is active: CMake, compilers, and Git run with the ambient privileges of the invoking user. This is disclosed in the security model and is unchanged by M24.
- Absolute local-path sources remain absolute and are intentionally not rewritten when a project moves. Relative paths are the portable form and are covered by the moved-workspace test.
- Direct sources are validated from their local metadata and acquisition result rather than through a remote registry index.
- Release archives and `SHA256SUMS` are generated by the owner-controlled GitHub Actions workflow after tag creation; no release artifact checksums are claimed in this source-tree report.
- The website build warnings described in Section 7 are pre-existing and non-blocking.

At report authoring time, none of these limitations blocked owner review of the v0.2.0 release candidate. See the v0.2.1 amendment above for the post-audit status.

---

## 10. Dependencies on Other Worktrees

None. M24 was implemented entirely in the primary working tree. M23 project-import behavior was already present in the current tree and was verified rather than merged from another worktree. No unmerged branch dependency was introduced.

---

## 11. Suggested Conventional Commit Messages

Preferred single squashed commit:

```text
feat: add direct package sources and prepare v0.2.0 release
```

Optional split commit series:

```text
feat(resolution): add canonical direct source identities
feat(source): normalize direct sources through the active catalog
feat(cli): add direct source flags to crux add
test: cover offline direct-source Git, archive, and path pipelines
docs: document direct sources and v0.2.0 release material
chore(release): set version metadata to 0.2.0
```

No commit was created by this work.

---

## 12. Suggested Pull Request

**Title**:

```text
feat: direct package sources and v0.2.0 release readiness
```

**Description**:

```markdown
## Summary
- Add Git, SHA-256-verified archive, and local-path dependencies to the existing resolver, lockfile, acquisition, build, install, and CMake integration pipeline.
- Lock Git tags, branches, revisions, and commits to immutable full SHAs; refresh only with `crux lock --update`.
- Add source conflict, cycle, path, ref, and archive-integrity validation.
- Add `crux add` direct-source flags and minimal inline-table manifest syntax while preserving registry string dependencies.
- Add repository and website documentation, v0.2.0 release notes, changelog, roadmap, and version metadata updates.

## Verification
- `cargo fmt --all -- --check`: pass
- `cargo clippy --all-targets --all-features -- -D warnings`: pass
- `cargo test --all`: 515 passed, 0 failed
- `cargo check`: pass
- `cargo build --release`: pass
- `pnpm build`: pass (11 pages; two pre-existing non-fatal warnings)

## Owner Actions
- Review and approve this change.
- Create the v0.2.0 tag and release artifacts through the owner-controlled workflow.
- Publish the website after approval.
```

---

## 13. Release Readiness Decision (at report authoring time — historical)

M24 is complete against the stated scope:

- Direct sources use the existing dependency pipeline.
- Git sources are immutable and stable across repeated locks.
- Remote archives require and verify SHA-256 before extraction.
- Local paths, conflicts, cycles, and unsafe refs are validated.
- Deterministic, reproducible lockfile output is preserved.
- All required quality gates pass.
- Phase B audits, M23 `.gitignore` verification, E2E scenarios A–H, version/release documentation, and website updates are complete.
- No Git or release-state mutation was performed by the implementation work.

At report authoring time, the repository was ready for the owner's review and release decision. Formal tagging, pushing, release publication, and website deployment remained owner-only actions.

**Historical status at report authoring time: READY FOR OWNER RELEASE REVIEW.** The current status is recorded in the v0.2.1 amendment at the top of this report.
