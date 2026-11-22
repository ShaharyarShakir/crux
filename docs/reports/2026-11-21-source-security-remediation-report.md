# Source Security & Correctness Remediation Report

**Date**: 2026-11-21
**Input**: Crux repository audit of 2026-11-21 (working tree on `main`, v0.2.1 hotfix uncommitted)
**Scope**: Remediation items 1–9 of the audit plus the confirmed tar symlink-chain escape. Nothing else: no v0.2.2 compiler-flag work, no unrelated features.
**Repository state**: Dirty working tree on `main`. All pre-existing owner changes are preserved. **No Git state-changing operation** was performed (no branch, worktree, stage, commit, push, PR, merge, rebase, reset, clean, or delete).
**Status**: **READY FOR OWNER REVIEW**. Stopped after the remediation scope.

> **Amendment (2026-11-21, post-remediation):** after reviewing §5, the owner made decisions D1–D3. Their implementation is recorded in **§9 Owner decisions**, which was added after the original remediation. Sections 1–8 are preserved unchanged as the historical record of the original remediation; in particular, §5 lists the decisions as they stood *before* the owner ruled on them. A further owner-approved enhancement, `crux lock --check` checksum-drift reporting, is recorded in **§10**. A later pre-release security-hardening pass is recorded in **§11**.

---

## 1. Verification gates (actual results)

All gates ran inside the Nix flake (`nix develop --command …`) on the final tree.

| Gate | Result |
|---|---|
| `cargo fmt --check` | pass (exit 0) |
| `cargo clippy --all-targets -- -D warnings` | pass (exit 0, no warnings) |
| `cargo test` | **547 passed, 0 failed** across 25 test binaries (522 before; +25 new) |
| `cargo check` | pass |
| `cargo build --release` | pass; `target/release/crux --version` → `crux 0.2.1` |

**Mutation check.** Each fix was reverted on its own in a scratch copy of the repository (outside the working tree), and the suite was re-run to confirm that the new regression tests fail without the fix. Results are in §4.

---

## 2. Findings and fixes

### F0 — CRITICAL: tar extraction symlink-chain escape (confirmed)

**Finding.** `extract_tar_reader` checked symlink targets lexically. A chain of links that are each lexically in-root (`x/s -> ..`, `x/s2 -> s/..`, `x/s3 -> s2/../..`), followed by a regular file `x/s3/PWNED.txt`, wrote that file three directories above the extraction root. A hard link reached through such a path, followed by a same-named file entry, could also truncate and overwrite an outside file.

**Fix** (`src/source/extraction.rs`, `src/core/paths.rs`). Extraction now goes through a single write-boundary type, `ExtractionRoot`, which enforces four invariants for both tar and zip:

1. **Never write through a symlink.** Each ancestor directory of an entry is checked with `symlink_metadata` and created one level at a time with `create_dir`, never `create_dir_all`. Any symlinked ancestor is a `SecurityViolation`, including one planted in the destination before extraction.
2. **Never follow or share on overwrite.** An existing file, symlink, or hard link at the entry path is unlinked, and the new file is created with `create_new` (`O_EXCL`). Replacing a directory with a non-directory is refused.
3. **Physical symlink validation.** Link targets are resolved with the new `core::paths::resolve_physical_path`, which follows existing links the way the kernel does (bounded at 40 hops, so loops fail). Every created link is **re-validated after the whole archive is extracted**, which catches order-dependent chains whose meaning only changes once later links exist.
4. **Hard links** may only reference an already-extracted *regular file*, reached without traversing any symlink. Self-links are rejected.

The same physical resolver now backs `tree.rs` symlink validation. That validator had the same lexical-`..` weakness for Git and path trees: the lexical walk popped `..` without regard to symlinks before canonicalizing.

### F1 — Lockfile checksums were never verified

**Finding.** `ActiveCatalog::acquire_locked` ignored `LockedPackage::checksum`. A registry entry whose archive and checksum changed for an already-locked version would be used silently.

**Fix** (`src/source/direct.rs`, `src/source/mod.rs`). `acquire_locked`, used by `fetch`, `build`, and `install`, now verifies:

- **Archive sources (registry or direct):** the locked checksum must equal the catalog's archive checksum *before any network access*. The download is then verified against that same digest, as before.
- **Git sources:** the locked commit is re-acquired and its v2 tree digest must match. For compatibility, a digest in the legacy **v1** format, as written by released **v0.2.0** and by 0.2.1, is also accepted (see F6).
- **Local path sources:** intentionally **not** verified against the lock (see §5, decision D1).
- Lock entries without a checksum are acquired unchanged.

A mismatch returns the new `AcquisitionError::LockedChecksumMismatch { package, source, locked, actual }`. Its message names the package, source, both digests, and the remedy (`crux lock --update`).

### F2 — Nested `build/` and `target/` directories were discarded

**Finding.** `is_ignored_name` was applied at every depth, so Git and path snapshots silently dropped any nested `build/` or `target/` directory (Boost ships `libs/*/build/`). Those files were also excluded from the digest.

**Fix** (`src/source/tree.rs`). Only `.git` is skipped at every depth. `build/`, `target/`, `cmake-build-*`, `.crux/`, and the completion sentinel are skipped **only at the tree root**.

### F3 — Non-default Git branches could not be resolved

**Finding.** After `clone --no-checkout`, only the default branch exists locally, so `rev-parse dev^{commit}` failed (`fatal: Needed a single revision`).

**Fix** (`src/registry/git.rs`). Branches resolve via `refs/remotes/origin/<branch>` and tags via `refs/tags/<tag>`. Rev and Commit forms are unchanged. The clone passes `--origin origin` explicitly so a user-level `clone.defaultRemoteName` cannot break this. A side benefit: a branch and a tag with the same name can no longer be confused.

### F4 — The download timeout was a whole-transfer deadline

**Finding.** ureq 2's `AgentBuilder::timeout` bounds the entire request, including the body. With a 30 s limit and a 500 MB cap, slow-but-live downloads failed.

**Fix** (`src/source/download.rs`). The agent now uses `timeout_connect`, `timeout_read`, and `timeout_write`, all set from the same 30 s value as **inactivity** limits. It also sets `https_only(true)`, so a redirect to `http://` is refused *before* any request is sent there. The existing post-redirect URL check remains as defense in depth. The `HttpTransport` trait signature is unchanged; only the documented meaning of `timeout` is clarified.

### F5 — Git cache identity: correction to the audit

**Correction.** The audit reported (medium #6) that branch acquisitions were served from a stale ref-keyed cache. **That was wrong.** `acquire_git` only ever *stored* snapshots under a commit-derived key, so the pre-clone ref-keyed lookup could never hit. It was dead code, not a stale cache. The existing test `git_branch_commit_is_stable_until_explicit_update` already showed that updates were picked up.

**Change made anyway** (`src/source/mod.rs`). The dead lookup was replaced by an explicit rule: only an immutable commit, either the locked commit or a full 40-hex `commit =` request, may be answered from the cache before cloning. Mutable refs are always re-resolved, `resolved_git_commit` is always populated, and a clone that resolves a pinned commit to anything else is an error.

### F6 — Path dependencies were built from the live directory

**Finding.** `acquire_path` hashed and copied the tree into the cache, but returned the **live** directory. The copy was unused, and the tree could change between hashing and building.

**Fix** (`src/source/mod.rs`, `src/source/direct.rs`). Path sources now return the **cache snapshot**. The live tree is hashed, copied, and the *copy* is re-hashed. If the two differ (the tree changed mid-copy), acquisition fails with a retry message. A cached snapshot is reused only after its contents are re-hashed and match, so a tampered snapshot is rebuilt rather than trusted. Nested relative `path =` dependencies are still resolved against the package's **real** directory, not the snapshot.

### F7 — Permission hashing depended on umask

**Fix** (`src/source/tree.rs`). The v2 digest hashes only the **executable bit** of each regular file, so `0644`, `0664`, and `0600` all hash the same, as do `0755` and `0775`. Copies still preserve the full mode, so the snapshot stays faithful and legacy v1 verification of a snapshot matches the original checkout.

### F8 — Git subprocess pipe stall and terminal prompts

**Fix** (`src/registry/git.rs`). `GitOps::run_git` now delegates to the existing `core::process::run_with_timeout`, which drains stdout and stderr on reader threads. Output larger than the ~64 KiB pipe buffer no longer stalls git until the timeout. Acquisition commands set `GIT_TERMINAL_PROMPT=0`, because a null stdin does not stop git from opening `/dev/tty`. Credential helpers keep working. The interactive registry commands (`clone_repository`, `update_repository`) were deliberately left unchanged.

### F9 — Non-UTF-8 file names could collide in the digest

**Fix** (`src/source/tree.rs`). v2 hashes paths and symlink targets as raw OS bytes (`OsStrExt::as_bytes` on Unix, UTF-16LE on Windows) instead of `to_string_lossy`.

### Tree digest format change (v1 → v2)

F2, F7, and F9 change the digest, so the domain tag moves from `crux-source-tree-v1` to `crux-source-tree-v2`. The format is documented at the top of `src/source/tree.rs`. **The lockfile format is unchanged**: same schema, same `sha256:<hex>` encoding. What changes is the value recorded for Git and path packages:

- Existing v0.2.0/0.2.1 lockfiles **keep verifying**: v1 digests of Git sources are accepted via `compute_legacy_tree_sha256_v1`.
- The next `crux lock` rewrites Git and path checksums to v2. This is a one-time content diff in `crux.lock`, not a format change.
- Old cache entries record v1 tree digests, so they fail re-validation once and are rebuilt. Archives re-extract from the cached archive with no network access; Git sources re-clone.

---

## 3. Tests added (25)

| Test | Location | Covers |
|---|---|---|
| `test_resolve_physical_path_follows_symlink_chains` | `src/core/paths.rs` | physical resolver, chain, loop limit |
| `ref_expressions_use_fully_qualified_refs` | `src/registry/git.rs` | F3 |
| `noninteractive_command_disables_terminal_prompts` | `src/registry/git.rs` | F8 |
| `run_git_drains_large_output_without_stalling` | `src/registry/git.rs` | F8 (1 MiB `cat-file`) |
| `clone_and_pin_commit_resolves_non_default_branch` | `src/registry/git.rs` | F3, branch/tag separation |
| `rejects_physical_symlink_chain_escape` | `src/source/tree.rs` | F0 (tree validator) |
| `nested_build_and_target_directories_are_kept` | `src/source/tree.rs` | F2 |
| `digest_depends_only_on_executable_bit` | `src/source/tree.rs` | F7 |
| `non_utf8_names_do_not_collide` | `src/source/tree.rs` | F9 |
| `legacy_v1_digest_matches_released_v1_algorithm` | `src/source/tree.rs` | v1 compatibility (independent re-implementation) |
| `slow_but_live_transfer_outlasting_timeout_succeeds` | `src/source/download.rs` | F4 (loopback) |
| `stalled_transfer_fails_with_read_timeout` | `src/source/download.rs` | F4 (loopback) |
| `production_agent_refuses_plain_http` | `src/source/download.rs` | F4 / HTTPS-only |
| `test_reported_symlink_chain_escape_is_rejected` | `tests/source.rs` | F0, exact audit PoC |
| `test_symlink_chain_escape_rejected_in_reverse_order` | `tests/source.rs` | F0, order independence |
| `test_entries_cannot_be_written_through_symlinks` | `tests/source.rs` | F0, in-tree and planted links |
| `test_nested_paths_and_valid_internal_links_are_extracted` | `tests/source.rs` | F0, no false positives |
| `test_overwrite_attempts_never_follow_links` | `tests/source.rs` | F0, symlink, hard-link, and directory overwrite |
| `test_hard_links_cannot_reach_outside_or_through_symlinks` | `tests/source.rs` | F0, hard links |
| `test_symlink_escaping_after_top_level_unwrap_is_rejected` | `tests/source.rs` | F0, unwrap hardening |
| `test_cli_fetch_rejects_lockfile_checksum_drift` | `tests/source.rs` | F1, registry archive, before network |
| `git_non_default_branch_is_locked_to_its_commit` | `tests/direct_sources.rs` | F3, end-to-end `crux lock` |
| `git_cache_is_keyed_by_immutable_commit_only` | `tests/direct_sources.rs` | F5 |
| `git_lockfile_checksum_is_verified_on_fetch` | `tests/direct_sources.rs` | F1 (Git), v1 compatibility |
| `local_path_sources_build_from_verified_cache_snapshots` | `tests/direct_sources.rs` | F6, F2, tamper detection |

The existing test `mixed_direct_and_registry_sources_are_deterministic` covers nested relative path dependencies under the snapshot change.

Network: the download tests use a `127.0.0.1` listener inside the test process, and Git tests use local repositories. No external network is contacted.

---

## 4. Mutation check (each fix reverted alone)

A clean control run of the scratch copy passed all of `--lib` (237), `tests/source.rs` (31), and `tests/direct_sources.rs` (19). Each row then reverts a single fix:

| Fix reverted | New tests that fail |
|---|---|
| F0: extraction reverted to the pre-remediation file | `test_reported_symlink_chain_escape_is_rejected`, `test_symlink_chain_escape_rejected_in_reverse_order`, `test_entries_cannot_be_written_through_symlinks`, `test_overwrite_attempts_never_follow_links`, `test_hard_links_cannot_reach_outside_or_through_symlinks` |
| F0: tree validator made lexical again | `rejects_physical_symlink_chain_escape` |
| F1: `acquire_locked` verification bypassed | `git_lockfile_checksum_is_verified_on_fetch`, `test_cli_fetch_rejects_lockfile_checksum_drift` |
| F2: recursive build/target filter | `nested_build_and_target_directories_are_kept`, `local_path_sources_build_from_verified_cache_snapshots` |
| F3: branch resolved as `<name>^{commit}` | `clone_and_pin_commit_resolves_non_default_branch`, `ref_expressions_use_fully_qualified_refs`, `git_non_default_branch_is_locked_to_its_commit` |
| F4: whole-request `timeout()` | `slow_but_live_transfer_outlasting_timeout_succeeds` |
| F6: live directory returned | `local_path_sources_build_from_verified_cache_snapshots` |
| F6: nested path base = snapshot | `mixed_direct_and_registry_sources_are_deterministic`, `direct_sources_conflict_with_registry_and_each_other` (existing tests) |
| F7: full mode hashed | `digest_depends_only_on_executable_bit` |
| F8: pre-remediation `run_git` (no draining) | `run_git_drains_large_output_without_stalling` (stalls the full 20 s timeout) |
| F9: lossy path hashing | `non_utf8_names_do_not_collide` |

Two new tests deliberately also pass on the old extractor:

- `test_nested_paths_and_valid_internal_links_are_extracted` is a false-positive guard: legitimate archives must keep extracting.
- `test_symlink_escaping_after_top_level_unwrap_is_rejected` is hardening coverage; the old lexical tree check also caught that specific case.

F5 is a behavior clarification (the original finding was incorrect; see §2), and `git_cache_is_keyed_by_immutable_commit_only` pins the intended semantics.

---

## 5. Decisions for owner review

- **D1 — Path sources are not verified against `crux.lock`.** Local paths are mutable working copies, and verifying them would make every local edit a hard failure until a re-lock (Cargo does not checksum path dependencies either). They are instead built from a freshly hashed, self-verified snapshot. If you want strict path pinning, it is a small change in `acquire_locked`.
- **D2 — Legacy v1 acceptance for Git.** A v1 match is accepted silently. Options: emit a warning suggesting `crux lock`, or set a date to remove v1 support.
- **D3 — `crux lock` adopts current registry checksums.** Re-locking (including the automatic re-lock `build` performs when the manifest and lock are out of sync) takes the registry's current checksum. Fetch, build, and install now refuse drift, but the lock step does not yet refuse *changing* a checksum for an unchanged name, version, and source. Cargo-style refusal would be a follow-up.
- **D4 — Tar entries under an in-tree symlinked directory are now rejected**, even when the link stays inside the root. This is the strict interpretation; mainstream source tarballs do not do this.

---

## 6. Remaining risks and limitations

- `run_with_timeout` kills only the direct `git` child. A surviving grandchild such as `git-remote-https` that still holds the pipes could delay the reader-thread join. This was pre-existing in the shared helper.
- SSH host-key or passphrase prompts are not suppressed. `GIT_SSH_COMMAND` was not overridden, to avoid changing user SSH configuration.
- `install/install.sh` checksum grep, action pinning by tag in `release.yml`, the `write_atomically` directory fsync, the `unwrap_single_top_level_dir` `pkg/pkg` collision, and the `http://` / `git://` / `file://` Git URL policy were **out of scope** and left unchanged.
- Adding `AcquisitionError::LockedChecksumMismatch` is a source-level breaking change for downstream code that matches the public enum exhaustively. The crate is a CLI, and no in-tree exhaustive matches exist.
- Windows: the non-Unix branches compile by inspection only. Symlink extraction stays Unix-only, as before.

---

## 7. Files changed by this remediation

Pre-existing owner modifications in these files were preserved. The line counts are this remediation's changes only, measured against snapshots taken before editing.

| File | Pre-existing owner changes? | Change |
|---|---|---|
| `src/core/paths.rs` | no | + `resolve_physical_path`, `MAX_SYMLINK_HOPS`, test |
| `src/core/mod.rs` | no | re-export `resolve_physical_path` (1 line) |
| `src/source/extraction.rs` | no | `ExtractionRoot` write boundary; tar and zip rewired |
| `src/source/tree.rs` | yes | v2 digest, root-only filters, physical validation, legacy v1, tests |
| `src/source/mod.rs` | yes | `LockedChecksumMismatch`; Git cache identity; path snapshots; export |
| `src/source/download.rs` | yes | connect/read/write timeouts, `https_only`, loopback tests |
| `src/source/direct.rs` | no | lock verification in `acquire_locked`; nested path base |
| `src/registry/git.rs` | yes | ref resolution, `--origin`, drained non-interactive `run_git`, tests |
| `tests/source.rs` | yes | 8 regression tests and a tar fixture builder |
| `tests/direct_sources.rs` | no | 4 regression tests |
| `docs/reports/2026-11-21-source-security-remediation-report.md` | new | this report |

No changes to `Cargo.toml` or `Cargo.lock` (no new dependencies), `crux.toml` or `crux.lock` schemas, CLI arguments, or the website and docs content.

---

## 8. Suggested commits (Conventional Commits) — for the owner

```
fix(source): prevent tar symlink-chain and hard-link extraction escapes
fix(lock): verify crux.lock checksums when acquiring locked packages
fix(source): hash source trees in v2 format (exec bit, raw paths, root-only filters)
fix(git): resolve non-default branches and drain git output without prompts
fix(source): use inactivity timeouts and HTTPS-only redirects for downloads
fix(source): build path dependencies from verified cache snapshots
```

**Suggested PR title**: `fix(source): remediate 2026-11-21 source security audit`

---

## 9. Owner decisions (post-remediation, 2026-11-21)

This section was added after the original remediation (§1–§8). It records the owner's rulings on decisions D1–D3 from §5 and their implementation. D4 (rejecting tar entries under an in-tree symlinked directory) received no ruling and is unchanged.

### 9.1 Decisions and implementation

**Decision 1 — Local path dependencies remain mutable** (resolves D1).
The original remediation already exempted path sources from lockfile verification. This step *formalizes* that, extends it to re-locking, and adds tests and documentation:

- Acquisition: `acquire_locked_with_status` reports `LockedChecksumStatus::MutablePath`, and no mismatch error is ever raised for path sources.
- Re-locking: `to_lockfile_checked` refreshes a path dependency's checksum silently, with no integrity error and no `--update` needed.
- Builds still use verified cache snapshots (§2 F6), and nested relative paths still resolve against the real directory.
- Documentation states that path dependencies do not provide the immutability guarantees of pinned Git or archive sources.

**Decision 2 — Legacy Git checksums: warn, don't reject** (resolves D2).

- Detection: `ActiveCatalog::acquire_locked_with_status` returns `LockedChecksumStatus::LegacyFormat` when a Git lock entry matches only the legacy `crux-source-tree-v1` digest.
- Warning: `fetch`, `build`, and `install` print one non-fatal warning per package per command (deduplicated by package id). It states that the checksum is in the legacy format, that the current format hashes consistently across machines, and that `crux lock` migrates it.
- Migration: `ActiveCatalog::is_legacy_git_checksum` recomputes the v1 digest of the Git snapshot acquired while loading the catalog. When it matches, a normal `crux lock` (no `--update`) rewrites the checksum to v2 and prints `Migrated legacy checksum for '<pkg>' (<source>) to the current format: old … new …`. Migration only happens when the legacy digest provably describes the same content.
- Lockfile parsing, resolution, and serialization are unchanged, and output stays deterministic: a second `crux lock` after migration is byte-identical.

**Decision 3 — Re-locking rejects checksum changes by default** (resolves D3).

- New `ResolvedGraph::to_lockfile_checked(existing, &ChecksumPolicy)` returns a `CheckedLockfile` (the lockfile plus a sorted list of `ChecksumChange`s), or `LockError::ChecksumChanged(LockedChecksumChange)`.
- For each package whose **name, version, and source identity** match the existing lock entry:
  - equal checksums are kept;
  - path sources are refreshed (Decision 1);
  - legacy-equivalent Git digests are migrated (Decision 2);
  - any other difference is an **integrity error** unless `accept_changes` is set.
- `accept_changes` is `true` **only** for `crux lock --update`. `crux lock`, and the lockfile rewrites done by `crux add` and `crux remove`, always pass `false`. Builds that auto-re-lock go through `run_lock(update: false)`, so they also refuse.
- The integrity error names the package, source, locked checksum, and current checksum. It states that `crux.lock` was left unchanged and points to `crux lock --update`. Nothing is written: lock writes only after construction succeeds, and add/remove build the lockfile before touching `crux.toml`.
- With `--update`, every accepted change is printed to stderr as a warning (`warning: accepted a changed checksum for '<pkg>' from '<source>' …`). It states that `crux lock --update` was requested explicitly and lists the old and new checksums.
- A different version, or a different source identity (new Git commit or ref, new direct-archive URL or digest), is a new identity and is never treated as an integrity change. Direct Git and archive sources follow the same rule as registry archives.
- Acquisition-time verification (§2 F1) is unchanged and still enforced by `fetch`, `build`, and `install`.

**CLI surface.** No new command or flag was introduced. The existing `crux lock --update`, already the explicit "re-resolve" path, now also expresses "accept changed checksums". Its help text and the CLI references were updated. `ResolvedGraph::to_lockfile_with_existing` is kept unchanged for API compatibility (documented as unchecked) and is no longer used by the CLI.

### 9.2 Tests added (17)

| Test | Location | Covers |
|---|---|---|
| `checked_relock_keeps_unchanged_checksum` | `src/resolution/resolver.rs` | D3: unchanged checksum |
| `checked_relock_rejects_changed_checksum_for_same_version_and_source` | `src/resolution/resolver.rs` | D3: changed checksum → error and message |
| `checked_relock_accepts_and_reports_explicit_update` | `src/resolution/resolver.rs` | D3: explicit update |
| `checked_relock_allows_version_upgrades_and_source_changes` | `src/resolution/resolver.rs` | D3: version, Git commit, and archive identity changes |
| `checked_relock_rejects_changed_git_tree_for_same_commit` | `src/resolution/resolver.rs` | D3: Git |
| `checked_relock_refreshes_mutable_path_checksums_silently` | `src/resolution/resolver.rs` | D1 |
| `checked_relock_migrates_legacy_equivalent_checksums` | `src/resolution/resolver.rs` | D2 |
| `legacy_checksum_warning_is_emitted_once_per_package` | `src/cli/commands/mod.rs` | D2: dedup and wording |
| `normal_lock_with_unchanged_checksum_is_stable` | `tests/lock_integrity.rs` | D3.1 (byte-identical re-lock) |
| `normal_lock_rejects_changed_checksum_and_leaves_lockfile_untouched` | `tests/lock_integrity.rs` | D3.2, registry archive; `fetch` enforcement; `crux add` cannot smuggle changes |
| `explicit_update_accepts_and_reports_changed_checksum` | `tests/lock_integrity.rs` | D3.3 |
| `legitimate_version_upgrade_is_accepted_by_normal_lock` | `tests/lock_integrity.rs` | D3.4 |
| `local_path_edits_do_not_cause_lockfile_checksum_failures` | `tests/lock_integrity.rs` | D1: edit → fetch OK from fresh snapshot → re-lock OK |
| `legacy_git_checksum_warns_once_and_lock_migrates_it` | `tests/direct_sources.rs` | D2 + D3.7: warning, migration, determinism |
| `current_format_git_checksum_produces_no_warning` | `tests/direct_sources.rs` | D2: current format |
| `git_relock_rejects_changed_tree_checksum_unless_explicitly_updated` | `tests/direct_sources.rs` | D3.2/3.3/3.6 for Git |
| `git_source_identity_change_is_a_legitimate_relock` | `tests/direct_sources.rs` | D3.5 |

Direct-archive identity changes (D3.5/3.6) are covered at the resolver level. A CLI-level direct-archive test would need a live HTTPS download during `crux lock`, which the offline policy forbids.

### 9.3 Mutation check

Each decision's logic was reverted alone in a fresh scratch copy. A control run passed (`--lib` 245, `direct_sources` 23, `lock_integrity` 5).

| Reverted | New tests that fail |
|---|---|
| D3: strict refusal (always accept) | `checked_relock_rejects_changed_checksum_for_same_version_and_source`, `checked_relock_rejects_changed_git_tree_for_same_commit`, `normal_lock_rejects_changed_checksum_and_leaves_lockfile_untouched`, `git_relock_rejects_changed_tree_checksum_unless_explicitly_updated` |
| D3: explicit update accepted silently (no report) | `checked_relock_accepts_and_reports_explicit_update`, `explicit_update_accepts_and_reports_changed_checksum`, `git_relock_rejects_changed_tree_checksum_unless_explicitly_updated` |
| D3: identity ignores source | `checked_relock_allows_version_upgrades_and_source_changes` |
| D3: `crux add` re-locks permissively | `normal_lock_rejects_changed_checksum_and_leaves_lockfile_untouched` |
| D2: no legacy detection at lock | `legacy_git_checksum_warns_once_and_lock_migrates_it` |
| D2: warning suppressed | `legacy_checksum_warning_is_emitted_once_per_package`, `legacy_git_checksum_warns_once_and_lock_migrates_it` |
| D2: warning not deduplicated | `legacy_checksum_warning_is_emitted_once_per_package` |
| D1: path checksums enforced at re-lock | `checked_relock_refreshes_mutable_path_checksums_silently`, `local_path_edits_do_not_cause_lockfile_checksum_failures` |
| D1: path checksums enforced at acquisition | `local_path_edits_do_not_cause_lockfile_checksum_failures` |

### 9.4 Verification (final tree, Nix shell)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass |
| `cargo test --all` | **564 passed, 0 failed** (26 test binaries; 547 before plus 17 new) |
| `cargo check` | pass |
| `cargo build --release` | pass; `crux 0.2.1` |
| `crux-site`: `npm run build` | pass, 11 pages |

Runtime smoke tests with `target/release/crux`, in scratch projects outside the repository:

- registry archive: the initial lock succeeds; after the registry checksum is swapped, `crux lock` exits 1 with `Status: Integrity Error` and `crux.lock` is byte-identical; `crux fetch` refuses with `does not match crux.lock`; `crux lock --update` exits 0, warns with the old and new checksums, and records the new one;
- Git: a v1 checksum produces a single warning on `crux fetch` (exit 0); `crux lock` prints the migration and restores the v2 digest; a later `crux fetch` prints no warning;
- `crux lock --help` shows the updated `--update` text and the integrity note.

### 9.5 Documentation updated

- `docs/reference/security-model.md`: v2 digest description, enforcement bullet, and a new **Lockfile Integrity & Re-locking** subsection covering immutable vs mutable sources, `lock` vs `lock --update`, legacy migration, and trust boundaries.
- `docs/guides/direct-sources.md`: path mutability, a **Checksum integrity** section, security bullets, and limitations (including "checksums prove unchanged, not trustworthy").
- `docs/reference/cli-reference.md`: `crux lock` options and behavior.
- `crux-site/src/content/docs/design/security.md`, `crux-site/src/content/docs/guides/direct-sources.md`, and `crux-site/src/content/docs/reference/cli.md`: matching content.
- `src/cli/args.rs`: `crux lock --help` text.

### 9.6 Compatibility

- **Lockfile format: unchanged.** Same schema and the same `sha256:` encoding. Existing v0.2.0 (and pre-release v0.2.1) lockfiles parse, resolve, and verify. Legacy Git digests are migrated on the next `crux lock`.
- **Behavior change (intended):** `crux lock`, `crux add`, `crux remove`, and builds that auto-re-lock now fail when a locked archive or Git package reports a different checksum for an unchanged version and source. Previously the new checksum was silently adopted. Recovery is `crux lock --update`.
- **Public API (additive):** `ResolvedGraph::to_lockfile_checked`, `ChecksumPolicy`, `ChecksumChange`, `ChecksumChangeReason`, `CheckedLockfile`, `LockedChecksumChange`, `ActiveCatalog::acquire_locked_with_status`, `ActiveCatalog::is_legacy_git_checksum`, and `LockedChecksumStatus`. The new `LockError::ChecksumChanged` variant is source-breaking only for downstream exhaustive matches, of which there are none in-tree. `to_lockfile_with_existing` and `acquire_locked` keep their signatures and behavior.

### 9.7 Remaining limitations

- `crux lock --update` is coarse. It accepts checksum changes for **all** packages and also re-resolves versions and refs. A per-package acceptance flag was not added, to avoid inventing CLI syntax without owner approval (see 9.9).
- Legacy migration is offered only for Git sources, since only they have tree digests. A v1 path checksum is simply refreshed (Decision 1).
- Checksums detect *change*; they do not establish trust. A package that was malicious when first locked is not caught, and builds still run with ambient privileges (§6).
- Legacy-v1 matching depends on the snapshot reproducing the original checkout's permission bits. If the lock was made on a machine with a different umask, the v1 digest will not match, and Crux reports a hard mismatch instead of a warning; `crux lock --update` then resolves it explicitly.
- The §6 limitations from the original remediation still apply.

### 9.8 Files changed in this step

| File | Pre-existing owner changes? | Change |
|---|---|---|
| `src/resolution/resolver.rs` | no | `to_lockfile_checked` and policy types; 7 tests |
| `src/resolution/lockfile.rs` | no | `LockError::ChecksumChanged`, `LockedChecksumChange` |
| `src/resolution/mod.rs` | no | re-exports |
| `src/source/direct.rs` | no (changed in §7) | `acquire_locked_with_status`, `LockedChecksumStatus`, `is_legacy_git_checksum`, Git snapshot tracking |
| `src/source/mod.rs` | yes | re-export `LockedChecksumStatus` |
| `src/cli/commands/mod.rs` | no | shared checked-lock, reporting, and warning helpers; 1 test |
| `src/cli/commands/lock.rs` | no | checked lock, integrity status, change reporting |
| `src/cli/commands/add.rs` | no | strict checked re-lock |
| `src/cli/commands/remove.rs` | no | strict checked re-lock |
| `src/cli/commands/fetch.rs` | no | legacy warning |
| `src/cli/commands/build.rs` | no | legacy warning |
| `src/cli/commands/install.rs` | no | legacy warning |
| `src/cli/args.rs` | yes | `crux lock --help` text only |
| `tests/lock_integrity.rs` | new | 5 integration tests |
| `tests/direct_sources.rs` | no (changed in §7) | 4 integration tests |
| docs (6 files, see 9.5) | yes (all six) | documentation |
| this report | (new in §7) | amendment banner and §9 |

**Working-tree note.** During this step, `CHANGELOG.md`, `Cargo.lock`, `Cargo.toml`, `README.md`, and `flake.nix` appeared **staged** in the Git index; they were unstaged at the start of the session. The agent did not edit those files and ran no index-modifying Git command. The index was left exactly as found for the owner to review.

### 9.9 Suggested commits and open owner input

```
feat(lock): reject checksum changes on re-lock unless `crux lock --update`
feat(lock): warn about legacy v1 Git checksums and migrate them on `crux lock`
fix(lock): keep local path dependencies mutable during re-lock and acquisition
docs(security): document lockfile integrity, mutable path sources, and migration
```

Open questions for the owner:

1. Should `--update` stay the single switch for "accept changed checksums", or do you want a narrower, per-package acceptance (new CLI syntax)?
2. Should `crux lock --check` also report checksum drift against the registry? It needs no network for registry archives. Today it checks only manifest/lock synchronization.
3. When should legacy-v1 acceptance be removed (target release)?

---

## 10. `crux lock --check` checksum-drift reporting (owner-approved, 2026-11-21)

This section was added after §9. It answers open question 2 from §9.9.

**Owner decisions for this step:**
- `crux lock --update` remains the single mechanism for accepting changed checksums (§9.9 question 1: answered, no per-package syntax).
- Registry checksum-drift reporting is added to `crux lock --check` (question 2: implemented here).
- Legacy-checksum compatibility and warnings stay. Removal is not part of this task (question 3 remains open).

### 10.1 Architecture and design

- **Data available without downloads.** `crux lock --check` already loads `ActiveCatalog`. Its registry half is the local `GitRegistry` checkout, whose `package.toml` files carry each archive's `sha256` checksum as `CatalogEntry::checksum`. Its direct half holds the Git snapshots already acquired, together with their v2 tree digests. Comparing locked checksums with these entries needs no network access, no extraction, and no build.
- **New audit**, in `src/resolution/lockfile.rs`: `audit_lockfile_checksums(&Lockfile, &dyn PackageCatalog, legacy)` returns a `ChecksumAudit`, one `ChecksumAuditEntry` per locked package in `(name, version)` order. It is generic over `PackageCatalog`, so it stays decoupled from storage, and it only reads.
- **Classification** (`ChecksumFinding`, with `severity()` and a stable `label()`):

  | Finding | Severity |
  |---|---|
  | `Match` (`ok`) | ok |
  | `MutablePath` (`mutable-path`) | ok, not compared |
  | `Drift` (`checksum-drift`) | error |
  | `VersionMissing` (`version-missing`, lists available versions) | error |
  | `PackageMissing` (`package-missing`) | error |
  | `SourceChanged` (`source-changed`) | error |
  | `CatalogChecksumUnavailable` (`checksum-unavailable`) | warning |
  | `LockChecksumMissing` (`lock-checksum-missing`) | warning |
  | `LegacyFormat` (`legacy-checksum`) | warning |

  The registry default-alias rule used during lock regeneration is applied, so an alias is not misreported as a source change.
- **Whole-registry unavailability** (the registry cannot be loaded) is reported as `Status: Unverified (registry metadata unavailable)`, with actionable text and a failing exit. This is distinct from all per-package findings.
- **CLI flow** (`run_lock_check`, `src/cli/commands/lock.rs`):
  - The audit runs whenever `crux.lock` parses, *even if synchronization fails*. A missing registry version or a renamed registry therefore gets its own category, rather than only the resolver's generic sync error.
  - The existing sync messages, `Status:` strings, and error types are preserved.
  - Output is a deterministic table, a summary line, and a scope note saying that only locally available registry metadata was compared, with the `crux registry update <name>` instructions. Error and warning explanations go to stderr.
  - Drift text tells the owner to verify upstream content first and only then use `crux lock --update`, and not to use it merely to silence the error.
  - Failure maps to the new `LockError::ChecksumAuditFailed(n)`.
- **Never writes, never accepts.** `--check` performs no write path and never calls `--update`. Tests assert that `crux.lock` and `crux.toml` stay byte-identical and that the isolated cache receives no archive or source files.

### 10.2 Tests added (9)

| Test | Location | Covers |
|---|---|---|
| `classifies_every_finding_and_is_deterministic` | `src/resolution/lockfile.rs` | all categories, severities, counts, order independence |
| `matching_lockfile_has_no_errors_and_default_alias_is_not_a_source_change` | `src/resolution/lockfile.rs` | match and alias rule |
| `legacy_equivalent_digest_is_a_warning_not_drift` | `src/resolution/lockfile.rs` | legacy |
| `lock_check_reports_matching_registry_checksums` | `tests/lock_integrity.rs` | match, scope note, determinism, byte-identical files, no cache writes |
| `lock_check_reports_registry_checksum_drift_without_modifying_anything` | `tests/lock_integrity.rs` | drift, both digests, remedy, distinct from the other categories, byte-identical files, no cache writes, no acceptance |
| `lock_check_distinguishes_missing_registry_version_and_package` | `tests/lock_integrity.rs` | version-missing (with available list), package-missing |
| `lock_check_reports_changed_registry_source_identity` | `tests/lock_integrity.rs` | source-changed |
| `lock_check_reports_unavailable_registry_metadata` | `tests/lock_integrity.rs` | per-package `checksum-unavailable` warning, whole-registry `Unverified` |
| `lock_check_warns_on_legacy_git_checksum_and_reports_git_drift_read_only` | `tests/direct_sources.rs` | legacy warning (no migration), direct Git drift, byte-identical files |

### 10.3 Mutation check

A clean control run had no failures. Each mutation reverted a single behavior:

| Reverted | Tests that fail |
|---|---|
| Drift classified as `Match` | `classifies_every_finding_and_is_deterministic`, `lock_check_reports_registry_checksum_drift_without_modifying_anything`, `lock_check_warns_on_legacy_git_checksum_and_reports_git_drift_read_only` |
| Drift downgraded to a warning | the same three |
| Missing version reported as missing package | `classifies_every_finding_and_is_deterministic`, `lock_check_distinguishes_missing_registry_version_and_package` |
| Source identity ignored | `classifies_every_finding_and_is_deterministic`, `lock_check_reports_changed_registry_source_identity` |
| Default-alias rule removed | `matching_lockfile_has_no_errors_and_default_alias_is_not_a_source_change` |
| Legacy detection disabled in `--check` | `lock_check_warns_on_legacy_git_checksum_and_reports_git_drift_read_only` (reported as drift instead) |
| Audit skipped when synchronization fails | `lock_check_distinguishes_missing_registry_version_and_package`, `lock_check_reports_changed_registry_source_identity` |
| `Unverified` status for an unloadable registry removed | `lock_check_reports_unavailable_registry_metadata` |

### 10.4 Verification

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass |
| `cargo test --all` | **573 passed, 0 failed** (26 binaries; 564 before plus 9 new) |
| `cargo check` | pass |
| `cargo build --release` | pass; `crux 0.2.1` |
| `crux-site`: `npm run build` | pass, 11 pages |

CLI smoke tests (`target/release/crux`, scratch project and registry outside the repository):

- **Matching registry:** a table with `ok arc 1.0.0` and `1 ok, 0 warning(s), 0 error(s)`, the scope note, `Status: Valid (synchronized)`, exit 0.
- **Registry checksum swapped:** `checksum-drift`, `Status: Invalid (checksum audit failed)`, an error naming both digests and the `--update` guidance, the summary line `crux.lock was not modified`, exit 1.
- **Nonexistent registry path:** `Status: Unverified (registry metadata unavailable)`, exit 1.
- In all three cases, the SHA-256 of `crux.lock` and of `crux.toml` was identical before and after.

### 10.5 Documentation

- `docs/reference/cli-reference.md` and `crux-site/src/content/docs/reference/cli.md`: `--check` audit categories, read-only guarantees, the local-metadata scope, and when to refresh versus `--update`.
- `docs/reference/security-model.md` and `crux-site/src/content/docs/design/security.md`: "Auditing checksums with `crux lock --check`", its finding table, the trust boundary of the audit, and how to respond to each finding.

### 10.6 Limitations and remaining owner input

- **Local metadata only.** Remote registry changes are invisible until the checkout is refreshed. This is stated in every `--check` run and in the docs.
- **Catalog loading side effects (pre-existing, unchanged).** Before the audit, `--check` loads direct sources exactly as before this task: an uncached direct Git source is cloned, local paths are snapshotted into the cache, and an unconfigured project bootstraps the official registry unless `CRUX_OFFLINE` is set. The audit itself downloads nothing and writes nothing. *Owner input:* should `--check` become strictly cache-only for direct sources, reporting `checksum-unavailable` when a source is not cached? That would require an offline acquisition mode.
- Registry entries that record no checksum (Git-typed registry packages) cannot be drift-checked. They are reported as warnings.
- Removal of legacy-v1 acceptance is still unscheduled (§9.9 question 3).

### 10.7 Files changed in this step

| File | Change |
|---|---|
| `src/resolution/lockfile.rs` | `ChecksumFinding`, `FindingSeverity`, `ChecksumAudit(Entry)`, `audit_lockfile_checksums`, `LockError::ChecksumAuditFailed`; 3 tests |
| `src/resolution/mod.rs` | re-exports |
| `src/cli/commands/lock.rs` | `run_lock_check` audit flow, `write_checksum_audit`, `describe_finding` |
| `tests/lock_integrity.rs` | 5 tests and helpers |
| `tests/direct_sources.rs` | 1 test |
| `docs/reference/cli-reference.md`, `docs/reference/security-model.md`, `crux-site/src/content/docs/reference/cli.md`, `crux-site/src/content/docs/design/security.md` | documentation |
| this report | banner sentence and §10 |

**Repository state during this step.** While this step was in progress, the owner created four commits on `main`: `6e3a978` (the five previously staged release files), `6214f9d`, `ba74847`, and `e176794`. HEAD moved from `34a4752` to `e176794`, and the index is now empty. The agent created none of these commits and ran no index-modifying command. Commit `e176794` includes `crux-site/src/content/docs/design/security.md` with the §9 lockfile-integrity documentation, but **not** the source code that implements it (`src/`, `tests/`, which remain uncommitted), nor this step's `--check` audit subsection (still an uncommitted addition to that file). HEAD therefore documents lockfile-integrity behavior that the committed code does not yet contain. Committing the remaining remediation changes together resolves this.

**Suggested commit:**

```
feat(lock): report registry checksum drift in `crux lock --check`
```

---

## 11. v0.2.1 security-hardening pass (2026-11-21)

This section was added after §10. Sections 1–10 are unchanged. This pass re-inspected six areas against the current code: Git subprocess lifecycle, Git URL policy, installer checksum verification, GitHub Actions, archive extraction edge cases, and atomic filesystem operations. Each item was reproduced (or ruled out) before it was changed.

**Baseline before changes:** `cargo test --all` → **573 passed, 0 failed** (26 binaries), matching the expected count. The index was empty and HEAD was `e176794`.

### 11.1 Confirmed findings and fixes

| # | Finding | Severity | Reproduction | Fix |
|---|---|---|---|---|
| H1 | **Subprocess timeout did not bound the lifecycle.** `run_with_timeout` (used by Git and CMake) killed only the direct child and then joined the pipe readers, which blocked for as long as any grandchild (e.g. `git-remote-https`, `ssh`, compilers) held the pipes. Those helpers kept running after the "timeout". | Medium (availability; runaway helper processes) | `sh -c 'sleep 20 & …; wait'` with a 300 ms timeout returned after **20.0 s**, grandchild still alive | `src/core/process.rs`: on timeout, descendants are discovered through `/proc` (Linux) while the child is still alive and SIGKILLed, then the child is killed and reaped. Readers accumulate output incrementally and get a bounded `READER_GRACE_PERIOD` (2 s) after exit. Children stay in the foreground process group, so Ctrl-C delivery is unchanged. No new dependency; one documented `kill(2)` FFI call targeting a specific pid. |
| H2 | **Git option injection in registry clone.** `GitOps::clone_repository` ran `git clone <url> <path>` with no `--` and no URL validation (`crux registry add`, `CRUX_OFFICIAL_REGISTRY_URL`). | Medium (option injection confirmed; code execution not demonstrated) | `crux registry add evil "--upload-pack=touch …;false"` was parsed by Git as an option; Git then treated the target directory as the repository | `clone_registry` validates with `validate_registry_url` (leading `-` rejected) and passes the URL after `--`. |
| H3 | **Git URL policy accepted unsafe forms.** `validate_git_url` accepted `ext::…@host:x` and `fd::…` (via the loose SCP check), plaintext `http://` and `git://`, URLs with whitespace, and **credential-bearing URLs**. Credential URLs are copied verbatim into `crux.lock` and diagnostics. | Medium (credential disclosure); Low (plaintext transports, given commit pinning); defense-in-depth (`ext::` is blocked by Git's default `protocol.ext.allow=never`) | Probe program: all listed forms returned `Ok` | Deliberate policy in `resolution/source.rs` (see 11.2) with actionable errors; `GIT_ALLOW_PROTOCOL=https:ssh:file` on every Git subprocess; credential redaction (`redact_url_credentials`) in all Git error paths. Archive URLs also reject embedded credentials. |
| H4 | **Registry Git operations had no timeout**, and the implicit official-registry bootstrap could prompt for credentials. `is_git_repository`, `head_commit`, `update_repository`, and `clone_repository` used `.output()`. | Low–Medium (a stalled clone hangs any command that bootstraps the registry) | Code inspection | Registry clone/pull use `run_with_timeout` (10 min); local `rev-parse` uses 30 s. The bootstrap is non-interactive (`GIT_TERMINAL_PROMPT=0`), while explicit `registry add`/`update` may still prompt. |
| H5 | **Registry URL credentials printed.** `crux registry list` / `add` echoed configured URLs verbatim (tokens in terminal and CI logs). | Low–Medium | Code inspection plus CLI test | Output is redacted. A new `config.toml` is created `0600`. |
| H6 | **Installer accepted a checksum line for a different file.** `grep "$ARCHIVE_NAME"` is an unanchored regex. A `SHA256SUMS` whose only line named `….tar.gz.sig` passed verification and installed. Missing or empty entries failed closed but **silently**; duplicates gave a misleading "tampered" error. | Low (the digest comparison still guarded integrity; no bypass of the hash check was found) | Fake `file://` release: decoy `.sig` line → exit 0, installed | Strict parser: every line must be `<64 hex> <space><space\|*><name>`, the name is compared literally, exactly one entry is required, and explicit errors are given. Also: strict version-tag validation, `https://`/`file://` base URL only, curl `--proto =https,file --proto-redir =https`, wget `--https-only`, an exclusive `mktemp` destination, and trap cleanup. |
| H7 | **Release workflow shell injection.** `TAG="${{ github.event.inputs.tag }}"` (and later `${{ steps.vars.outputs.tag }}`) were interpolated into `run:` scripts with a `contents: write` token. Tag names pushed by a writer can also contain `$`, `;`, `\|`. | Medium (requires write/dispatch access; would grant arbitrary commands with the release token) | Code inspection; the new test executes the real step with hostile inputs | Values pass via `env:`, are validated with a whole-string bash regex (newline-safe, so no `GITHUB_OUTPUT` injection), and never appear inside `run:`. |
| H8 | **Workflow trust settings too broad.** Actions were pinned to movable tags; CI had no `permissions:` (repository default token scopes); website `pages: write` and `id-token: write` were granted to the build job; `safe.directory "*"` in CI and release; checkout persisted credentials. | Low–Medium (supply chain, token scope) | Inspection | All actions pinned to commit SHAs (with version comments); top-level read-only permissions, with write scopes only on the release and deploy jobs; wildcard `safe.directory` removed (fixture repos are runner-owned); `persist-credentials: false`. |
| H9 | **Archive unwrap edge cases.** A nested root `pkg/pkg/…` failed with `DirectoryNotEmpty`; a top-level hidden file was **silently overwritten** by the wrapper's same-named file. | Low (content stays inside the extraction root; integrity and compatibility issue) | Probe program reproduced both | Move the wrapper aside under a unique name first; refuse any collision instead of overwriting. |
| H10 | **`tar -C dir .` archives rejected.** The `./` root directory entry was treated as an "empty path" security violation. | Compatibility (not security) | Real GNU-tar archive → `SecurityViolation` | Directory entries naming the root (`.`, `./`) are skipped; non-directory entries with an empty path are still rejected. |
| H11 | **Atomic writes reset permissions and were inconsistent.** Rewriting `crux.lock` changed `0600` → `0644`. Five separate writers existed; `save_config`, the build-engine writer, and the CMake integration writer used `File::create` on predictable temp names (following a planted path) without failure cleanup. | Low (unintended permission widening; same-user race) | `chmod 600 crux.lock; crux lock` → `644` | One hardened `core::filesystem::write_atomically[_with_mode]`: exclusive temp creation with per-process unique names, permission preservation, new config files `0600`, file `sync_all`, best-effort parent-directory sync, and guaranteed temp cleanup. Lockfile, config, manifest edits, build metadata, and CMake output all use it. |

### 11.2 Git protocol policy (now enforced and documented)

| Form | Dependencies | Registries |
|---|---|---|
| `https://host/path` | yes, no credentials | yes, credentials allowed (redacted) |
| `ssh://[user@]host[:port]/path` | yes, user name only | yes |
| `user@host:path` | yes | yes |
| `file:///…`, `/abs`, `./rel`, `../rel` | yes | yes |
| `http://`, `git://` | **rejected** | **rejected** |
| `<transport>::…` (`ext::`, `fd::`) | **rejected** | **rejected** |
| leading `-` | **rejected** | **rejected** |

**Compatibility note:** existing manifests or lockfiles that use `http://` or `git://` Git URLs, or that embed credentials, now fail with an actionable error. No in-repo tests or fixtures used these forms.

### 11.3 Theoretical concerns (not reproduced; documented, not changed)

- **Concurrent content-addressed cache stores** (`atomic_replace_dir`): if another writer stores the same key between the existence check and the rename, the old fallback deleted that writer's tree. **Changed defensively** to keep the winner's (identical, content-addressed) tree. There is no deterministic regression test, because the race window cannot be forced without hooks.
- **Cache temp creation** used `File::create` and `create_dir_all` on pid+time names. **Changed defensively** to exclusive creation with unique names, covered by a concurrency uniqueness test. Exploiting it would require write access to the user's own cache directory.
- **Git in attacker-controlled directories:** running `git pull` in a registry checkout whose `.git/config` an attacker controls (for example `core.fsmonitor`) executes code. Crux only runs Git in checkouts it cloned or that the user configured, and Git cannot transport a nested `.git` directory, so this is out of reach of package content. Recommendation: document for users who point registries at shared directories.
- **SSH prompts:** `GIT_TERMINAL_PROMPT=0` does not stop `ssh` from asking to confirm an unknown host key or a passphrase on the terminal. Such a prompt now ends at the 60 s acquisition timeout with the whole process tree killed. Forcing `BatchMode=yes` would change user SSH behavior, so it is deferred.
- **Control characters in archive file names** are accepted inside the extraction root; they could only affect terminal rendering of diagnostics. Deferred.
- **Signed/query-string tokens in archive URLs** (`?token=`) are not detected; only `userinfo@` credentials are. Deferred.

### 11.4 Regression tests added (39)

| Area | Tests (location) |
|---|---|
| H1 | `test_timeout_kills_descendants_and_returns_promptly`, `test_lingering_pipe_holder_does_not_block_successful_exit`, `test_large_output_is_fully_captured` (`src/core/process.rs`) |
| H2–H5 | `registry_clone_rejects_option_injection_before_running_git`, `git_transport_allow_list_is_enforced_by_git`, `git_commands_carry_the_transport_allow_list`, `registry_clone_errors_redact_credentials`, `registry_clone_still_supports_local_repositories` (`src/registry/git.rs`); `git_url_policy_accepts_supported_forms`, `git_url_policy_rejects_unsafe_forms_with_actionable_errors`, `registry_urls_follow_the_same_policy_but_may_carry_credentials`, `credentials_are_redacted_from_diagnostics`, `credential_bearing_sources_are_rejected_in_manifests_and_lockfiles` (`src/resolution/source.rs`); `test_registry_list_redacts_url_credentials` (`tests/registry.rs`) |
| H6 | `valid_checksum_entries_install`, `missing_checksum_entry_fails_closed_with_a_diagnostic`, `entries_for_other_file_names_never_match`, `duplicate_or_ambiguous_entries_are_refused`, `malformed_entries_and_wrong_digests_fail_closed`, `unsafe_inputs_are_rejected_before_downloading` (`tests/install_script.rs`; real `install.sh` against a `file://` release, no network) |
| H7–H8 | `all_actions_are_pinned_to_commit_shas`, `no_expression_interpolation_inside_run_scripts`, `no_wildcard_safe_directory`, `permissions_are_least_privilege`, `checkout_does_not_persist_credentials`, `release_tag_validation_rejects_hostile_tags` (executes the real step) (`tests/workflow_security.rs`) |
| H9–H10 and edge cases | `test_nested_same_named_root_unwraps_correctly`, `test_unwrap_collision_with_hidden_root_entry_is_refused`, `test_dot_rooted_tarball_is_accepted_but_empty_file_paths_are_not`, `test_truncated_and_malformed_archives_fail_without_leaving_state`, `test_special_file_entries_are_rejected`, `test_conflicting_and_duplicate_entries_are_handled_deterministically`, `test_traversal_and_absolute_variants_are_rejected_in_tar_and_zip`, `test_unusual_but_valid_filenames_stay_inside_the_root` (`tests/source.rs`) |
| H11 and cache | `test_write_atomically_preserves_existing_permissions_and_sets_new_mode`, `test_write_atomically_never_follows_a_planted_temp_symlink_and_cleans_up_on_error` (`src/core/filesystem.rs`); `save_config_creates_owner_only_file_and_preserves_existing_mode` (`src/registry/config.rs`); `test_cache_temp_paths_are_unique_under_concurrency`, `test_lockfile_rewrite_preserves_permissions` (`tests/source.rs`) |

### 11.5 Mutation check

Each fix was reverted alone in a scratch copy of the repository (outside the working tree), and the affected suites were re-run.

| Reverted | New tests that fail |
|---|---|
| H1: descendants not killed on timeout | `test_timeout_kills_descendants_and_returns_promptly` |
| H1: unbounded reader join | `test_lingering_pipe_holder_does_not_block_successful_exit` (blocked the full 30 s) |
| H3: `http://` / `git://` accepted | `git_url_policy_rejects_unsafe_forms_with_actionable_errors` |
| H3: `::` transport check removed | `git_url_policy_rejects_unsafe_forms_with_actionable_errors`. The SCP grammar still rejects the URL (defense in depth); the test detects the lost diagnostic. |
| H3: credentials allowed in dependency URLs | `git_url_policy_rejects_unsafe_forms_with_actionable_errors`, `credential_bearing_sources_are_rejected_in_manifests_and_lockfiles` |
| H3: `GIT_ALLOW_PROTOCOL` removed | `git_commands_carry_the_transport_allow_list`, `git_transport_allow_list_is_enforced_by_git` |
| H2: registry URL validation removed | `registry_clone_rejects_option_injection_before_running_git` |
| H3/H5: redaction disabled | `credentials_are_redacted_from_diagnostics`, `registry_clone_errors_redact_credentials`, `git_url_policy_rejects_unsafe_forms_with_actionable_errors`, `test_registry_list_redacts_url_credentials` |
| H5: `registry list` / `add` output unredacted | `test_registry_list_redacts_url_credentials` |
| H6: original `install.sh` restored | all 6 installer tests. The valid-entry test also fails, because the old script rejects CRLF and uppercase-hex `SHA256SUMS`. |
| H7/H8: original workflows restored | all 6 workflow tests |
| H9: original unwrap restored | `test_nested_same_named_root_unwraps_correctly`, `test_unwrap_collision_with_hidden_root_entry_is_refused` |
| H10: `./` root skip removed | `test_dot_rooted_tarball_is_accepted_but_empty_file_paths_are_not` |
| H11: permissions not preserved | `test_write_atomically_preserves_existing_permissions_and_sets_new_mode`, `save_config_creates_owner_only_file_and_preserves_existing_mode`, `test_lockfile_rewrite_preserves_permissions` |
| H11: config not created `0600` | `save_config_creates_owner_only_file_and_preserves_existing_mode` |

All 15 mutations were caught. One case (`::` transport) was initially inconclusive because its run hit the scratch timeout under load; it was re-run alone and caught.

### 11.6 Verification

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass (one lint in a new test was fixed before the final run) |
| `cargo test --all` | **612 passed, 0 failed** (28 binaries; baseline 573, plus 39 new) |
| `cargo check` | pass |
| `cargo build --release` | pass; `crux 0.2.1` |
| `crux-site`: `npm run build` | pass, 11 pages |
| Installer tests | 6/6 (real `install.sh`, `file://` release, no network) |
| Workflow tests | 6/6 (including executing the release tag-validation step) |

Runtime smoke tests (`target/release/crux`, scratch `CRUX_HOME`):

- `crux registry add evil "--upload-pack=touch …"` is rejected ("cannot start with '-'") and no marker file is created.
- `crux add util --git` with `http://…`, `git://…`, or `ext::…` is rejected with the policy message.
- `crux add util --git` with `https://user:hunter2@…` is rejected, and the message shows `https://***@…`, so the secret is not echoed.
- `crux.toml` is unchanged in all cases.

Not run: ShellCheck and actionlint/zizmor, which are not available offline; workflow YAML was reviewed by diff and checked by the policy tests.

### 11.7 Documentation updated

- `docs/reference/security-model.md`: Git transport policy, bounded subprocess termination, and the atomic-write guarantees, with durability stated precisely.
- `docs/guides/direct-sources.md` and `crux-site/src/content/docs/guides/direct-sources.md`: supported Git URL forms.
- `crux-site/src/content/docs/design/security.md`: same policy and subprocess bullets.
- `docs/reference/cli-reference.md`: `--git` / `--url` URL policy; `registry add` URL policy, redaction, and timeouts.
- `docs/guides/installation.md` and `crux-site/src/content/docs/guides/installation.md`: how the installer verifies downloads, and the trust boundary of a same-origin `SHA256SUMS`.

The sandbox disclosure is unchanged: Crux still runs CMake and compilers with the user's full privileges.

### 11.8 Deferred / recommended later

- **SSH:** `BatchMode` for non-interactive acquisition (needs an owner decision on SSH behavior).
- **Release authenticity:** signing releases (e.g. Sigstore/minisign) so the installer can check authenticity, not only integrity.
- **CI checks:** `actionlint`/`zizmor` in CI; ShellCheck for `install/install.sh`. Neither tool is available offline here; the YAML was reviewed by diff and checked by the policy tests.
- **Archive URLs:** rejecting or redacting query-string tokens.
- **Process groups:** dedicated process groups with signal forwarding, which would also cover descendants that re-parent via `setsid`.

### 11.9 Files changed in this pass

Changed by this pass (all uncommitted, none staged):

| File | Purpose |
|---|---|
| `src/core/process.rs` | H1 |
| `src/registry/git.rs` | H2–H4, tests |
| `src/resolution/source.rs`, `src/resolution/mod.rs` | H3 policy, redaction, tests |
| `src/registry/config.rs` | bootstrap non-interactive, redaction, `0600` config via `write_atomically_with_mode`, test |
| `src/cli/commands/registry.rs` | H5 |
| `src/core/filesystem.rs`, `src/core/mod.rs` | H11 |
| `src/resolution/lockfile.rs` | lockfile writes through the hardened writer |
| `src/build/engine.rs`, `src/integrate/cmake.rs` | hardened writer |
| `src/source/cache.rs` | exclusive temp creation, race-safe store |
| `src/source/extraction.rs` | H9, H10 |
| `install/install.sh` | H6 (the owner's `v0.2.1` default change is preserved) |
| `.github/workflows/ci.yml`, `release.yml`, `website.yml` | H7, H8 |
| `tests/source.rs`, `tests/registry.rs` | tests |
| `tests/install_script.rs`, `tests/workflow_security.rs` | **new** test suites |
| `docs/reference/security-model.md`, `docs/reference/cli-reference.md`, `docs/guides/direct-sources.md`, `docs/guides/installation.md`, `crux-site/src/content/docs/design/security.md`, `crux-site/src/content/docs/guides/direct-sources.md`, `crux-site/src/content/docs/guides/installation.md` | documentation |
| this report | §11 and a banner sentence |

No dependencies were added (`Cargo.toml`/`Cargo.lock` untouched).

### 11.10 Git state

Inspected read-only at the end of the pass; the agent did not change it:

- HEAD: `e176794` (unchanged; no commits, amends, or history changes).
- Index: empty (nothing staged or unstaged by the agent).
- Working tree: 59 entries. Untracked: `docs/releases/v0.2.1-release-notes.md` (owner), this report, `tests/lock_integrity.rs`, `tests/install_script.rs`, `tests/workflow_security.rs`; all others are unstaged modifications.
- The §9.8/§10.7 note still applies: HEAD's site security page documents lockfile-integrity behavior whose code is uncommitted.

