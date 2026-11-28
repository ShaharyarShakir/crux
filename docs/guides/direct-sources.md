# Direct Package Sources

**Document Status**: Current  
**Version**: 0.2.1  
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

Local path dependencies are **intentionally mutable**: you can keep editing them without re-locking, and their lockfile checksum is never enforced. Each build uses a fresh, verified snapshot of the tree copied into the Crux cache, not the live directory. `crux lock` refreshes the recorded checksum.

`crux build` and `crux run` detect edits automatically. Each compares the tree digest of every path dependency with the one its current installation was built from, and rebuilds and reinstalls the packages that differ, and every package that depends on them, before building the application. Neither `crux lock` nor `crux install` is needed after a source edit. The comparison covers what the snapshot covers, so `.git`, and `build/`, `target/`, `cmake-build-*` and `.crux/` at the root of the package, do not trigger a rebuild, and neither do files outside the package directory. `crux integrate cmake` does not rebuild anything; run it again after a rebuild so the generated file points at the new installation.

Because the content can change at any time, path dependencies do not provide the immutability guarantees of pinned Git or archive dependencies.

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

### Checksum integrity

- `crux fetch`, `crux build`, and `crux install` verify archive and Git content against the checksum in `crux.lock`; a mismatch is a hard error.
- A normal `crux lock` keeps the existing checksum of every package whose version and source are unchanged. If the source now reports different content for that same version and source, `crux lock` fails with an integrity error and leaves `crux.lock` untouched.
- `crux lock --update` is the explicit way to accept such a change after you have verified it. Each accepted change is reported with the package, old checksum, and new checksum.
- A version upgrade, a new Git commit or ref, or a new archive URL/digest is a new identity, not an integrity change. It needs no `--update`.
- Local path dependencies are mutable, so their checksums are refreshed by `crux lock` and are never enforced.
- Lockfiles from Crux v0.2.0 (and pre-release v0.2.1 builds) may record Git checksums in the legacy `v1` tree-digest format. They are still accepted, with a one-time warning per package; run `crux lock` to migrate them to the current format, which hashes consistently across machines.

A dependency name may be provided by the registry **or** by a direct source, never both. Two direct sources that provide the same package name with different identities are rejected with a source-conflict error. Direct-source dependency cycles are also rejected.

---

## 5. Moved projects

Relative `path` sources are interpreted from the project root. Moving the whole workspace — the project plus its sibling local packages — preserves the relationship and keeps `crux.lock --check`, `build`, `install`, and `integrate cmake` working.

If the local package is moved independently, or if a nested local dependency can no longer be resolved, Crux fails with an actionable path-resolution error. It never guesses a replacement directory or silently falls back to the registry.

Absolute path sources remain absolute and are intentionally not rewritten when a project moves. Prefer relative paths for portable projects.

---

## 6. Security model

> [!WARNING]
> Use **v0.2.1 or later** when consuming direct sources you do not control. v0.2.1 added symlink-escape validation for direct Git and local-path source trees, HTTPS-only archive enforcement, and Git `rev` validation. Direct sources are still **not sandboxed**: their CMake/build scripts execute with the ambient privileges of the invoking user.

- Remote archives must use **HTTPS**; unencrypted `http://` URLs are rejected, and the SHA-256 digest is verified before extraction.
- Git and path arguments are validated and passed as discrete argv entries; no shell interpolation is used.
- **Supported Git URL forms.**
  - **Accepted:** `https://…`, `ssh://[user@]host/…`, SCP-style `user@host:path`, `file:///…`, and local paths (`/abs`, `./rel`, `../rel`).
  - **Rejected:** plain `http://` and `git://` (unauthenticated and unencrypted), and `<transport>::<address>` remote-helper forms such as `ext::`.
  - **No embedded credentials** in dependency URLs (for example `https://user:token@host/…`). They would be recorded in `crux.toml`, `crux.lock`, and diagnostics; use a Git credential helper or SSH keys instead.
  - **Enforced by Git too.** Git subprocesses run with `GIT_ALLOW_PROTOCOL=https:ssh:file`, so Git itself refuses any other transport, including through HTTP redirects.
- Git commands use explicit arguments, null stdin, and a hard timeout.
- Git refs are restricted to a safe character set, and leading-dash, traversal, and control-character forms are rejected.
- Git sources are checked out at the pinned commit. A locked checkout cannot silently advance to a newer commit.
- Archive digests are verified before extraction. Existing extraction guards against Zip Slip, path traversal, symlink escape, excessive file counts, and decompression bombs remain active.
- Local and Git sources receive deterministic SHA-256 source-tree digests (`crux-source-tree-v2`). `.git` is excluded at every depth; `.crux`, `build`, `target`, `cmake-build-*`, and Crux cache sentinels only at the tree root. These digests are real content measurements, not placeholders.
- Locked archive and Git checksums are enforced on acquisition and are never silently replaced by `crux lock`; see [Checksum integrity](#checksum-integrity).
- CMake execution still runs with ambient user privileges. See the [Security Model](../reference/security-model.md) for the current sandbox disclosure.

---

## 7. Limitations

- Direct sources are validated at lock/resolve time rather than by a separate registry index.
- Git tags, branches, and revisions are resolved locally by the Git CLI; HTTPS and SSH URLs work where the host Git and credentials permit them. Plain `http://`/`git://` remotes are not supported; SSH host-key trust is governed by your SSH configuration. Tests use local repositories only, so no external network service is required.
- A direct source cannot override a package name that already exists in the active registry.
- `--url` archives must use HTTPS and must include `--sha256`.
- `crux add` still rejects duplicate direct dependencies; use `crux remove <name>` before changing a source kind.
- Local path dependencies are mutable and are not pinned by their lockfile checksum.
- Checksums prove that content is unchanged since it was locked. They do not prove the content was trustworthy when it was first locked: review new or updated sources before locking them.
