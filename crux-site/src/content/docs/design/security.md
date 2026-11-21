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

