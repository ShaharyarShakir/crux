---
title: Direct Package Sources
description: Use Git, archive, and local-path dependencies through the normal Crux resolution and build pipeline.
---

Direct sources let a `crux.toml` dependency point at a Git repository, a SHA-256-verified remote archive, or a local package directory. They use the same resolver, lockfile, cache, build, install, and CMake integration pipeline as registry packages.

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
- Archive digests are verified before extraction; existing Zip Slip, symlink, and decompression-bomb protections remain active.
- Git and local sources receive real deterministic SHA-256 source-tree digests.
- HTTP/HTTPS archives require `--sha256`; local paths must exist and contain a valid `package.toml`.

See the repository [direct sources guide](https://github.com/ShaharyarShakir/crux/blob/main/docs/guides/direct-sources.md) for the full manifest grammar, moved-project behavior, and limitations.
