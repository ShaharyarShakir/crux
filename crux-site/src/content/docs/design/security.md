---
title: Security Model & Threat Assessment
description: Trust boundaries, process privileges, direct-source validation, and archive extraction security in Crux.
---

**Version**: 0.2.1  
**Last Updated**: 2026-11-20  

---

## Process Privileges & Sandbox Reality

> **Important Disclosure**: In Crux v0.2.1, external build tools (`cmake`, `make`, `ninja`, compilers) execute with the ambient privileges of the user running `crux`. No operating-system containerization or namespace sandbox (seccomp/bubblewrap) is active. This applies to **untrusted direct sources** (Git, archive, and local-path dependencies) as well as registry packages: their build trees can still execute external build tools with ambient privileges. Build only sources you trust, or run Crux inside your own isolated environment.

### Mitigations in Place
- **Zero Shell Interpolation**: All external commands (`git`, `cmake`, compilers) are executed with discrete argument vectors (`std::process::Command::arg`). Crux never invokes shell wrappers (`sh -c` or `cmd.exe /C`).
- **Working Directory Confinement**: Subprocesses are strictly confined to dedicated working directories.
- **Path Escaping**: Generated CMake scripts escape all user-controlled paths and reject embedded newlines or null bytes.
- **Validated Direct Sources**: Git URLs and refs are restricted to safe forms and passed as discrete argv entries; Git commands use null stdin and a hard timeout. Git tags, branches, and revisions are pinned to a full commit in `crux.lock`.
- **Verified Archives and Trees**: Remote archives require a SHA-256 digest and are verified before extraction. Git and local-path sources receive real deterministic source-tree digests, with symlink and path validation during tree copying.

---

## Direct-Source Security

Direct Git, archive, and local-path dependencies use the same resolver, lockfile, build, install, and CMake integration pipeline as registry packages. v0.2.1 hardens their specific trust boundaries:

- **Git argv-only execution**: Git is invoked with discrete arguments (no shell), null stdin, and a hard timeout. Leading-dash, traversal, control-character, and malformed `rev` values are rejected before any subprocess starts.
- **Immutable commit resolution**: tags, branches, revisions, and `HEAD` resolve to a full 40-character commit recorded in `crux.lock`; locked commits are not silently advanced. `crux lock --update` is the explicit refresh path.
- **HTTPS-only archives with SHA-256 before extraction**: plain `http://` archive URLs are rejected; the digest is verified before the archive is cached or unpacked.
- **Real deterministic tree digests**: Git and local-path trees receive actual SHA-256 source-tree digests (`crux-source-tree-v2`), not placeholders. `.git` is excluded at every depth; `.crux`, `build`, `target`, `cmake-build-*`, and cache sentinels only at the tree root.
- **Symlink-escape validation**: direct/path source trees are checked so symlinks resolving outside the source root are rejected before hashing, copying, or building.
- **Conflict and cycle checks**: a package name may come from the registry or from one direct source, never both. Registry-vs-direct and direct-vs-direct conflicts, and direct-source cycles, are rejected.
- **Git transport policy**: dependency URLs must use `https://`, `ssh://`, SCP-style `user@host:path`, `file://`, or a local path. Plain `http://`/`git://`, remote-helper transports (`ext::`, `fd::`, …), and embedded credentials are rejected. Registry URLs (`crux registry add`) follow the same transport policy but may embed credentials, which are always redacted from diagnostics. Every Git subprocess also runs with `GIT_ALLOW_PROTOCOL=https:ssh:file`, so Git refuses other transports even through redirects, and URLs are passed after `--` so they can never be parsed as options.
- **Bounded Git and build subprocesses**: Git and build tools run with hard timeouts covering the whole lifecycle. On timeout the process **and its descendants** (for example `git-remote-https`, `ssh`, or compilers) are terminated with `SIGKILL` and reaped; descendants are found through `/proc` on Linux. Output pipes are drained continuously and readers get a bounded grace period, so a lingering process holding a pipe cannot hang Crux. Source-acquisition Git commands never prompt (`GIT_TERMINAL_PROMPT=0`); the explicit `crux registry add` and `crux registry update` commands may still prompt for credentials.

See [Direct Package Sources](/crux/guides/direct-sources/) for the manifest grammar and operational details.

---

## Archive Extraction Defenses

Crux enforces defense-in-depth protections during `.zip`, `.tar`, and `.tar.gz` extraction:
1. **Zip Slip Protection**: Destination paths are canonicalized and verified to reside strictly within the target root directory.
2. **Symlink Escape Jail**: Symlink targets pointing outside extraction or staging roots are rejected.
3. **Decompression Bomb Ceilings**: Maximum 1 GB uncompressed size, maximum 50,000 files, and maximum 100:1 compression expansion ratio.

---

## Cryptographic Digest Verification

- All remote package downloads require **HTTPS**. Unencrypted `http://` endpoints are rejected for registry packages and direct archives.
- Downloaded bytes are streamed through SHA-256 hashers in real time.
- Digests are normalized and compared with the declared checksum before cache persistence or extraction. Corrupted or mismatched archives are immediately purged; no constant-time comparison is claimed.

## Lockfile Integrity & Re-locking

`crux.lock` records an integrity identity (`checksum = "sha256:..."`) for every package:

| Source kind | What the checksum measures | Guarantee |
|---|---|---|
| Registry archive / direct archive | SHA-256 of the archive file | **Immutable.** Verified before any download and again while streaming. |
| Git (pinned commit) | Deterministic source-tree digest of the checked-out commit | **Immutable.** The commit and its tree digest are re-verified on every acquisition. |
| Local path | Source-tree digest of the working copy at lock time | **Mutable by design** (see below). |

**Enforcement during acquisition.** `crux fetch`, `crux build`, and `crux install` compare the acquired content with the locked checksum. A mismatch for an archive or Git source is a hard error (`AcquisitionError::LockedChecksumMismatch`), reported before any archive is downloaded.

**Re-locking never silently replaces a checksum.** When a normal `crux lock`, or a lockfile rewrite caused by `crux add`/`crux remove`, regenerates `crux.lock`, it keeps the existing checksum of every package whose **name, version, and source identity are unchanged**. If the registry or source now reports a *different* checksum for that same version and source, Crux stops with an integrity error that names the package, the source, and both checksums. `crux.lock` is left untouched.

**Accepting a changed checksum on purpose.** After you have verified the new content (for example, an upstream re-tagged release), run:

```bash
crux lock --update
```

`--update` re-resolves versions and Git refs *and* explicitly accepts changed checksums for already-locked packages. Every accepted change is printed as a warning with the package, the old checksum, and the new checksum, so the decision is visible in CI logs and code review.

**What is not an integrity change.** Upgrading to a different version, or changing a dependency's source identity (a new Git commit or ref, a new archive URL or digest in `crux.toml`), creates a new package identity. Its checksum is recorded normally, without `--update`.

**Mutable local path dependencies.** `path = "..."` dependencies are working copies meant for active development. Their checksum is refreshed by `crux lock` and is **not** enforced during acquisition, so editing a local dependency never causes a checksum failure. Builds still never read the live directory: every acquisition hashes the tree, copies it into the content-addressed cache, re-hashes the copy, and builds from that verified snapshot. Path dependencies therefore do **not** provide the immutability guarantees of pinned Git or archive dependencies. Do not rely on them for reproducible release builds.

**Legacy checksum format.** Source-tree digests use the `crux-source-tree-v2` format. It hashes only the executable bit (not umask-dependent permission bits), hashes file names as raw bytes, and filters build output only at the tree root. Lockfiles written by Crux v0.2.0 (and pre-release v0.2.1 builds) record Git digests in the older `v1` format. Those digests are still verified and accepted, and Crux prints a one-time warning per package explaining that the current format hashes consistently across machines. Running `crux lock` migrates them to v2. The migration is reported, needs no `--update`, and is only performed when the legacy digest provably describes the same content.

**Auditing checksums with `crux lock --check`.** `crux lock --check` compares every checksum in `crux.lock` with the metadata Crux already has locally. The audit itself never downloads or extracts a package archive, never builds, and never writes to the project: `crux.toml` and `crux.lock` stay byte-identical, and it never runs `--update` for you. Like every catalog-backed command, though, `--check` first loads the direct sources declared in `crux.toml`. A direct Git source missing from the local cache is cloned, local path sources are snapshotted into the cache, and a project with no configured registry bootstraps the official registry unless `CRUX_OFFLINE` is set. Each locked package gets one finding:

| Finding | Meaning | Result |
|---|---|---|
| `ok` | Registry (or cached direct source) records the same checksum | pass |
| `checksum-drift` | Same version and source, but a **different** checksum | **fail** |
| `version-missing` | Registry provides the package, but not the locked version | **fail** |
| `package-missing` | Registry no longer provides the package | **fail** |
| `source-changed` | The locked version is now provided from a different source identity | **fail** |
| `checksum-unavailable` | Registry metadata records no checksum for this version, so drift cannot be detected | warning |
| `lock-checksum-missing` | `crux.lock` records no checksum; `crux lock` will record it | warning |
| `legacy-checksum` | Legacy `v1` Git digest of the same content; `crux lock` migrates it | warning |
| `mutable-path` | Local path dependency, intentionally not compared | pass |

If the registry metadata cannot be loaded at all, the check reports `Status: Unverified (registry metadata unavailable)` and fails, because nothing could be compared.

**Trust boundary of the audit.** The comparison uses only the **locally available** registry checkout. A change published to the remote registry is invisible until that checkout is refreshed (`crux registry update <name>`, or updating the checkout yourself when using `CRUX_REGISTRY_PATH`). To audit against the latest registry state, refresh first, then run `crux lock --check`.

**Responding to findings.**
- For `checksum-drift`, treat it as a possible supply-chain event: verify the new upstream content first. Run `crux lock --update` only if the change is legitimate, never just to silence the error.
- For `version-missing` or `package-missing`, refresh the registry first, in case your local metadata is stale. If the version or package really was withdrawn, choose a replacement and re-lock.
- For `source-changed`, confirm the new source is intended, then re-lock with `crux lock`.

