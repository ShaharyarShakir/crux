---
title: Direct Package Sources
description: Use Git, archive, and local-path dependencies through the normal Crux resolution and build pipeline.
---

Direct sources let a `crux.toml` dependency point at a Git repository, a SHA-256-verified remote archive, or a local package directory. They use the same resolver, lockfile, cache, build, install, and CMake integration pipeline as registry packages.

> **Security note (v0.2.1+)**: Use v0.2.1 or later for untrusted direct sources. Remote archives must use **HTTPS**, and direct sources are **not sandboxed** — their build scripts still run with the ambient privileges of the invoking user.

## Add a direct source

```bash
# Git tag
crux add util --git https://example.invalid/util.git --tag v1.2.3

# Git branch (resolved commit is locked)
crux add util --git git@github.com:org/util.git --branch main

# Archive (SHA-256 is mandatory)
crux add util --url https://example.invalid/util.tar.gz \
  --sha256 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef

# Local package directory
crux add util --path ../local-util
```

Equivalent manifest syntax:

```toml
[dependencies]
util = { git = "https://example.invalid/util.git", tag = "v1.2.3" }
archive_util = { url = "https://example.invalid/archive.tar.gz", sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }
local_util = { path = "../local-util" }
```

## Immutable Git locking

A requested Git ref stays in `crux.toml`; the resolved full commit is written to `crux.lock`:

```toml
[[package]]
name = "util"
version = "1.2.3"
source = "git+https://example.invalid/util.git#tag=v1.2.3#commit=0123456789abcdef0123456789abcdef01234567"
```

Repeated `crux lock` runs reuse the locked commit. Use `crux lock --update` to re-resolve a branch or tag deliberately.

## Checksum integrity

- `crux fetch`, `crux build`, and `crux install` verify archive and Git content against the checksum in `crux.lock`.
- A normal `crux lock` never replaces the checksum of a package whose version and source are unchanged. Different content for the same version and source is an integrity error, and `crux.lock` is left untouched.
- After verifying the new content, accept it explicitly with `crux lock --update`. Each accepted change is reported with the package, old checksum, and new checksum.
- A version upgrade, a new Git commit or ref, or a new archive URL/digest is a new identity and needs no `--update`.
- **Local path dependencies are mutable**: edit them freely. Their checksum is refreshed by `crux lock` and never enforced, and builds use a verified snapshot copied into the Crux cache. They do not provide the immutability of pinned Git or archive sources.
- Git checksums written by Crux v0.2.0 use a legacy tree-digest format. They are still accepted, with a one-time warning; `crux lock` migrates them to the current format, which hashes consistently across machines.

## Direct package metadata

A direct source directory or archive needs a `package.toml`, but its `[source]` section is optional:

```toml
[package]
name = "util"
version = "1.2.3"

[build]
system = "cmake"

[dependencies]
fmt = "^11.1"
```

Nested `path` dependencies are rebased relative to the consuming project, so a workspace and its local packages can be moved together.

## Safety rules

- A package name may come from the registry or from a direct source, never both.
- Conflicting direct sources for the same name and direct-source cycles are rejected.
- Git refs and URLs are validated and passed to Git as discrete arguments without shell interpolation.
- **Supported Git URL forms.**
  - **Accepted:** `https://…`, `ssh://[user@]host/…`, SCP-style `user@host:path`, `file:///…`, and local paths (`/abs`, `./rel`, `../rel`).
  - **Rejected:** plain `http://` and `git://` (unauthenticated and unencrypted), and `<transport>::<address>` remote-helper forms such as `ext::`.
  - **No embedded credentials** in dependency URLs (for example `https://user:token@host/…`). They would be recorded in `crux.toml`, `crux.lock`, and diagnostics; use a Git credential helper or SSH keys instead.
  - **Enforced by Git too.** Git subprocesses run with `GIT_ALLOW_PROTOCOL=https:ssh:file`, so Git itself refuses any other transport, including through HTTP redirects.
- Archive digests are verified before extraction; existing Zip Slip, symlink, and decompression-bomb protections remain active.
- Git and local sources receive real deterministic SHA-256 source-tree digests (`crux-source-tree-v2`).
- HTTPS archives require `--sha256`; local paths must exist and contain a valid `package.toml`.
- Direct Git and local-path trees are checked for symlink escapes, and Git `rev` values are validated before subprocess execution.

See the repository [direct sources guide](https://github.com/ShaharyarShakir/crux/blob/main/docs/guides/direct-sources.md) for the full manifest grammar, moved-project behavior, and limitations.
