# Contributing to the Official Registry

Crux's official registry is maintained as data in [`registry/`](../../registry/). No separate registry repository or binary cache is required.

## Entry layout

```text
registry/packages/<canonical-name>/<X.Y.Z>/package.toml
```

The directory name is the package identity. `package.toml` must repeat the same `name` and `version` strings exactly; `v1.2.3` is not accepted as a version directory because Crux uses strict SemVer.

A minimal entry looks like this:

```toml
[package]
name = "example"
version = "1.2.3"
description = "One-line description of the upstream project"
license = "MIT"
homepage = "https://example.org"
repository = "https://github.com/example/example"

[source]
type = "archive"
url = "https://github.com/example/example/archive/refs/tags/v1.2.3.tar.gz"
checksum = "sha256:<64 lowercase hex characters>"

[build]
system = "cmake"

[dependencies]
```

## Rules

1. **Never invent source data.** Download the exact archive from the declared URL and compute its SHA-256 with `sha256sum`. Reject redirect-only or generated URLs that can change content without changing the URL.
2. **Never invent dependencies.** Add a `[dependencies]` entry only when the upstream build actually links to another registered package. Each requirement uses the same SemVer syntax as `crux.toml`.
3. **Declare the real backend.** `system = "cmake"` is reserved for packages that pass the verification matrix below. Catalog-only entries use `system = "catalog-only"` so `crux build` fails with an actionable diagnostic instead of a generic CMake error.
4. **Keep identity stable.** Package names are case-sensitive. Do not rename an existing package without a migration plan.
5. **Do not add timestamps, generated checksums, or local paths.**

## Verification matrix

A package may be marked **Verified** in [`docs/guides/official-package-catalog.md`](../guides/official-package-catalog.md) only after all of the following pass on Linux x86_64:

- `crux check` in a consumer project that declares the dependency;
- `crux install <package> --profile debug` and `--profile release`;
- `crux build <package> --profile debug` and `--profile release` (including cache reuse);
- `crux integrate cmake`;
- a real C/C++ consumer built and executed with `crux build` and `crux run`;
- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all`, and `cargo check`.

Hardware-specific packages may be build-verified without claiming hardware runtime verification. Cross-compilation and hardware runtime claims are out of scope for M22.

## Registry tests

`tests/official_registry.rs` is offline-safe and checks:

- exactly 50 catalog packages and their canonical names;
- deterministic enumeration;
- archive-only sources with valid lowercase SHA-256 digests;
- explicit build-system classification;
- resolution of the 15-package verified tier;
- rejection of missing/invalid checksums and invalid dependency requirements.

Run it with:

```bash
nix develop --command cargo test --test official_registry
```
