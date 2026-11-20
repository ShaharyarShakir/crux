# Direct Package Sources

**Document Status**: Current  
**Version**: 0.2.0  
**Applies To**: `crux add`, `crux lock`, `crux fetch`, `crux build`, `crux install`, `crux integrate cmake`

---

## 1. Overview

Crux normally resolves dependencies from the configured Git-backed package registry. Starting in v0.2.0, a dependency can instead declare a **direct source**: a Git repository, a remote archive, or a local package directory.

Direct sources are normalized into the same `Dependency`, `CatalogEntry`, `PackageSource`, build, and install models used by registry packages. There is no second package-management pipeline. The resolver sees direct packages as ordinary catalog candidates, and the generated lockfile records their canonical source identity.

### Supported source kinds

| Kind | Manifest form | `crux add` form |
|---|---|---|
| Git | `{ git = "...", tag = "..." }` | `--git <URL> --tag <TAG>` |
| Git branch | `{ git = "...", branch = "..." }` | `--git <URL> --branch <BRANCH>` |
| Git revision | `{ git = "...", rev = "..." }` | `--git <URL> --rev <REV>` |
| Git commit | `{ git = "...", commit = "<40-hex>" }` | `--git <URL> --commit <SHA>` |
| Archive | `{ url = "...", sha256 = "<64-hex>" }` | `--url <URL> --sha256 <DIGEST>` |
| Local path | `{ path = "../relative-dir" }` | `--path <PATH>` |

---

## 2. Adding direct sources

### Git sources

```bash
crux add util --git https://example.invalid/util.git --tag v1.2.3
crux add util --git git@github.com:org/util.git --branch main
crux add util --git https://example.invalid/util.git --rev 9abc123
crux add util --git https://example.invalid/util.git --commit 0123456789abcdef0123456789abcdef01234567
```

The generated `crux.toml` entry is a one-line inline table:

```toml
[dependencies]
util = { git = "https://example.invalid/util.git", tag = "v1.2.3" }
```

Crux resolves tags, branches, revisions, and `HEAD` to a full 40-character commit SHA. The requested ref stays in `crux.toml`; the immutable commit is written to `crux.lock`.

```toml
[[package]]
name = "util"
version = "1.2.3"
source = "git+https://example.invalid/util.git#tag=v1.2.3#commit=0123456789abcdef0123456789abcdef01234567"
```

A locked Git commit is never silently upgraded. Running `crux lock` again reuses the locked commit, even if the upstream branch or tag moves. Use `crux lock --update` to explicitly re-resolve direct Git refs.

### Archive sources

```bash
crux add util --url https://example.invalid/util-1.2.3.tar.gz \
  --sha256 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

```toml
[dependencies]
util = { url = "https://example.invalid/util-1.2.3.tar.gz", sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }
```

Remote archives require a SHA-256 digest. Crux verifies the archive before extraction and reuses the content-addressed cache only when the digest matches. Supported formats are `.zip`, `.tar`, `.tar.gz`, and `.tgz`.

### Local path sources

```bash
crux add util --path ../local-util
```

```toml
[dependencies]
util = { path = "../local-util" }
```

The path is resolved relative to `crux.toml`. The directory must exist, must be a directory, and must contain a valid `package.toml`. Paths are lexically normalized; unsafe Git refs, traversal fragments, control characters, and ambiguous source declarations are rejected before resolution.

---

## 3. Package definitions in direct sources

A direct source directory or archive must contain a `package.toml`. Its `[source]` section is optional because the dependency declaration is authoritative.

```toml
[package]
name = "util"
version = "1.2.3"

[build]
system = "cmake"

[dependencies]
fmt = "^11.1"
```

Nested `path` dependencies inside a direct source are rebased relative to the consuming project. This allows a local package tree to be moved together with the project without rewriting `crux.toml`.

---

## 4. Determinism and lockfile synchronization

- Direct sources use canonical strings: `git+`, `archive+`, and `path+`.
- Lockfiles contain no timestamps, random nonces, or machine-specific absolute paths for relative path sources.
- Repeated `crux lock` runs are byte-for-byte identical.
- `crux lock --check` compares source identity. A Git commit change alone does not make the lockfile invalid because the commit is acquisition state; the URL and requested ref define dependency identity.
- `crux resolve` prints the source identity next to each resolved package.

A dependency name may be provided by the registry **or** by a direct source, never both. Two direct sources that provide the same package name with different identities are rejected with a source-conflict error. Direct-source dependency cycles are also rejected.

---

## 5. Moved projects

Relative `path` sources are interpreted from the project root. Moving the whole workspace — the project plus its sibling local packages — preserves the relationship and keeps `crux.lock --check`, `build`, `install`, and `integrate cmake` working.

If the local package is moved independently, or if a nested local dependency can no longer be resolved, Crux fails with an actionable path-resolution error. It never guesses a replacement directory or silently falls back to the registry.

Absolute path sources remain absolute and are intentionally not rewritten when a project moves. Prefer relative paths for portable projects.

---

## 6. Security model

- Git and path arguments are validated and passed as discrete argv entries; no shell interpolation is used.
- Git commands use explicit arguments, null stdin, and a hard timeout.
- Git refs are restricted to a safe character set, and leading-dash, traversal, and control-character forms are rejected.
- Git sources are checked out at the pinned commit. A locked checkout cannot silently advance to a newer commit.
- Archive digests are verified before extraction. Existing extraction guards against Zip Slip, path traversal, symlink escape, excessive file counts, and decompression bombs remain active.
- Local and Git sources receive deterministic SHA-256 source-tree digests, excluding `.git`, `.crux`, build output, and Crux cache sentinels. These digests are real content measurements, not placeholders.
- CMake execution still runs with ambient user privileges. See the [Security Model](../reference/security-model.md) for the current sandbox disclosure.

---

## 7. Limitations

- Direct sources are validated at lock/resolve time rather than by a separate registry index.
- Git tags, branches, and revisions are resolved locally by the Git CLI; HTTPS and SSH URLs work where the host Git and credentials permit them. Tests use local repositories only, so no external network service is required.
- A direct source cannot override a package name that already exists in the active registry.
- `--url` archives must be HTTP/HTTPS and must include `--sha256`.
- `crux add` still rejects duplicate direct dependencies; use `crux remove <name>` before changing a source kind.
