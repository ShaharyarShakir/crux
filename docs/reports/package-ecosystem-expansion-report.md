# M22 Package Ecosystem Report

**Milestone:** M22 – Official Package Ecosystem Expansion  
**Date:** 2026-11-19  
**Base commit:** `55dafbb` (`main`)  
**Primary platform:** Linux x86_64 (Nix dev environment)

## 1. Summary

M22 expands Crux's in-tree official registry from the M21 seed set to **50 canonical package identities** and **53 concrete versions**, while preserving M21 bootstrap/custom-registry behavior and source-first integrity.

- **Registry:** 50 packages, 53 package definitions loaded and validated by `crux registry list`.
- **Verified tier (15):** `benchmark 1.10.0`, `cJSON 1.7.19`, `CLI11 2.7.2`, `catch2 3.8.0`, `cxxopts 3.3.1`, `doctest 2.5.3`, `fmt 11.1.1`, `googletest 1.18.0`, `nlohmann-json 3.12.0`, `pcre2 10.49.0`, `spdlog 1.17.0`, `tomlplusplus 3.4.0`, `yaml-cpp 0.9.0`, `yyjson 0.13.0`, `zlib 1.3.1`.
- **Reference applications (6):** `examples/m22/fmt-json-cli`, `examples/m22/config-tools`, `examples/m22/data-checks`, `examples/m22/test-frameworks`, `examples/m22/logging-zip`, and the pure C application `examples/m22/c-zlib-roundtrip` — each exercised with `crux check`, lock/install flow, `crux build`, and `crux run` (with `crux add` used where appropriate).
- **Compatibility preserved:** legacy versions retained (`fmt 11.0.2`, `zlib 1.3.0`, `catch2 3.7.1`), official bootstrap still works, custom registries remain supported, and no fake checksums or invented URLs were introduced.

## 2. Verified package details (15)

All checksums were computed from the actual downloaded upstream archives (`sha256sum`) and recorded as `sha256:<lowercase-hex>` in `registry/packages/<name>/<version>/package.toml`.

| Package | Version | Source URL | SHA-256 | Build backend | Notes |
|---|---|---|---|---|---|
| benchmark | 1.10.0 | https://github.com/google/benchmark/archive/refs/tags/v1.10.0.tar.gz | `acc5c7c0fbed855c4aaa2d7c3c36fb82b0c15c28f4e151e3f679c2fc0aa404af` | CMake (`-DBENCHMARK_ENABLE_TESTING=OFF`) | Disables bundled/test targets so Crux can build without fetching Google Test sources. |
| cJSON | 1.7.19 | https://github.com/DaveGamble/cJSON/archive/refs/tags/v1.7.19.tar.gz | `7fa616e3046edfa7a28a32d5f9eacfd23f92900fe1f8ccd988c1662f30454562` | CMake (`-DCMAKE_POLICY_VERSION_MINIMUM=3.5`) | Upstream CMake minimum is old; policy floor is required on CMake 4.x. |
| CLI11 | 2.7.2 | https://github.com/CLIUtils/CLI11/archive/refs/tags/v2.7.2.tar.gz | `46eef3101da70852ec7af026e09d485ccee81813331c8c6052d39344443b83da` | CMake | Header-only consumer integration. |
| catch2 | 3.8.0 | https://github.com/catchorg/Catch2/archive/refs/tags/v3.8.0.tar.gz | `1ab2de20460d4641553addfdfe6acd4109d871d5531f8f519a52ea4926303087` | CMake | Legacy `3.7.1` retained for M21 compatibility. |
| cxxopts | 3.3.1 | https://github.com/jarro2783/cxxopts/archive/refs/tags/v3.3.1.tar.gz | `3bfc70542c521d4b55a46429d808178916a579b28d048bd8c727ee76c39e2072` | CMake | Header-only. |
| doctest | 2.5.3 | https://github.com/doctest/doctest/archive/refs/tags/v2.5.3.tar.gz | `174ebc4e769928959614789c5b4e9c3d0a0f81a62bb608756b127bfebfb21331` | CMake | Header-only. |
| fmt | 11.1.1 | https://github.com/fmtlib/fmt/releases/download/11.1.1/fmt-11.1.1.zip | `a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45` | CMake | Legacy `11.0.2` retained. |
| googletest | 1.18.0 | https://github.com/google/googletest/archive/refs/tags/v1.18.0.tar.gz | `6e3191c1455468b3fc35a417fb565c1c5071aee1b7e7f85e30cf48a98d37d8b5` | CMake | — |
| nlohmann-json | 3.12.0 | https://github.com/nlohmann/json/archive/refs/tags/v3.12.0.tar.gz | `4b92eb0c06d10683f7447ce9406cb97cd4b453be18d7279320f7b2f025c10187` | CMake | Header-only. |
| pcre2 | 10.49.0 | https://github.com/PCRE2Project/pcre2/archive/refs/tags/pcre2-10.49.tar.gz | `69d04dcddcf9e038aeae8f58b0c07ee5e08abb835ee8daa2bd6acee433dc112c` | CMake | Verified as C library consumer (`PCRE2_CODE_UNIT_WIDTH 8`). |
| spdlog | 1.17.0 | https://github.com/gabime/spdlog/archive/refs/tags/v1.17.0.tar.gz | `d8862955c6d74e5846b3f580b1605d2428b11d97a410d86e2fb13e857cd3a744` | CMake | — |
| tomlplusplus | 3.4.0 | https://github.com/marzer/tomlplusplus/archive/refs/tags/v3.4.0.tar.gz | `8517f65938a4faae9ccf8ebb36631a38c1cadfb5efa85d9a72e15b9e97d25155` | CMake | Header-only. |
| yaml-cpp | 0.9.0 | https://github.com/jbeder/yaml-cpp/archive/refs/tags/yaml-cpp-0.9.0.tar.gz | `25cb043240f828a8c51beb830569634bc7ac603978e0f69d6b63558dadefd49a` | CMake | — |
| yyjson | 0.13.0 | https://github.com/ibireme/yyjson/archive/refs/tags/0.13.0.tar.gz | `34e0f62a2bc11ab20d601e8ca1cc2b2079503aa45119a19133d89d19b94a0fae` | CMake | — |
| zlib | 1.3.1 | https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz | `9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23` | CMake | Legacy `1.3.0` retained. |

**Verification evidence:** debug and release installations completed for all 15 packages in the verification project (`/tmp/opencode/m22-verification-release.log`, `/tmp/opencode/m22-verification-debug.log`), and each package was consumed by at least one real reference application below.

## 3. 50-package catalog and status

Full per-package status is documented in [`docs/guides/official-package-catalog.md`](../guides/official-package-catalog.md).

| Status | Count | Notes |
|---|---:|---|
| Verified | 15 | CMake-buildable, installed (debug+release), and consumed by reference apps. |
| Catalog only | 33 | Exact upstream archive URL and SHA-256 recorded; build/install rejected by Crux because `[build].system = "catalog-only"` or upstream is not currently buildable through Crux's CMake-only engine. |
| Deferred | 2 (`musl`, `systemd`) | Tracked as target catalog entries but not registered as normal linkable library definitions because they are toolchain/runtime or environment packages. |

**Canonical naming:** directory names are canonical and case-sensitive (`catch2` lowercase; `CLI11`, `OpenSSL`, `mbedTLS`, `cJSON`, `nlohmann-json`, `paho-mqtt` retain upstream capitalization).

## 4. Dependency graphs

No dependency edges were invented for M22. Every verified package declares an empty `[dependencies]` table in its `package.toml`.

- **Verified tier:** flat graph of 15 independent nodes (confirmed by `tests/official_registry.rs`: resolving all 15 with `*` yields 15 packages).
- **Reference applications:** each app declares only direct dependencies in `crux.toml`; resolved lockfiles contain only those package nodes (no transitive registry edges).

## 5. Reference applications and runtime results

| App | Dependencies | `crux add` used | `check` | `lock`/install flow | `build` | `run` | Outcome |
|---|---|---|---|---|---|---|---|
| `examples/m22/fmt-json-cli` | `fmt`, `nlohmann-json`, `CLI11` | N/A (manifest predeclared) | pass | `crux lock`, `crux install` pass; build auto-installed debug deps | pass | pass (`--name Ada`) | Printed JSON with milestone/package/user. |
| `examples/m22/config-tools` | `tomlplusplus`, `cxxopts`, `yaml-cpp` | N/A (predeclared; registry bootstrapped via `.crux/config.toml`) | pass | `crux lock`, `crux install` pass; build auto-installed debug deps | pass | pass (`--name demo`) | Parsed TOML/YAML and CLI args. |
| `examples/m22/data-checks` | `cJSON`, `yyjson`, `pcre2` | N/A (predeclared; registry bootstrapped via `.crux/config.toml`) | pass | `crux lock`, `crux install` pass; build auto-installed debug deps | pass | pass | `cJSON packages=50`, `yyjson packages=50`, `pcre2 matched=true`. |
| `examples/m22/test-frameworks` | `catch2`, `googletest`, `doctest`, `benchmark` | N/A | pass | `crux lock`, `crux install` pass; build auto-installed debug deps | pass | pass | All four frameworks executed (1/1 tests or successful run, benchmark executed). |
| `examples/m22/logging-zip` | `spdlog`, `zlib` | `crux add spdlog@^1.17` | pass | add updated manifest/lock and installed deps; `crux install` pass | pass | pass | `roundtrip_ok=true`, compressed/decompressed with zlib and logged via spdlog. |

All runs were executed with the locally built `target/debug/crux` and `CRUX_HOME=/tmp/opencode/m22-crux-home` to use the official in-repo registry.

## 6. Implementation changes

- **Registry data:** generated 50 package directories / 53 versions under `registry/packages/` from real upstream archives and checksums.
- **Build system guard:** added `require_supported_build_system` and `build_configuration_for` in `src/cli/commands/mod.rs`; wired into package build/install flows in `src/cli/commands/build.rs` and `src/cli/commands/install.rs`. Catalog-only/non-CMake definitions fail explicitly before acquisition.
- **Registry schema (backward-compatible):** added optional `[build].cmake_args` in `src/registry/metadata.rs` with validation/sorting in `src/registry/git.rs`. This preserves existing definitions, enables deterministic per-package CMake options, and was required for real verification of `cJSON` and `benchmark` on the current toolchain.
- **Tests:** added offline-safe `tests/official_registry.rs` (catalog size/names, archive checksum validity, supported-tier resolution, and rejection of invalid/missing checksums and invalid dependency requirements).
- **Docs/site:** updated `README.md`, `ROADMAP.md`, `docs/reference/architecture.md`, `docs/reference/cli-reference.md`, `docs/guides/quickstart.md`, `docs/guides/official-package-catalog.md`, `docs/contributing/official-registry.md`, `crux-site/src/content/docs/registry/packages.md`, `crux-site/src/content/docs/project/roadmap.md`, and `crux-site/src/content/docs/reference/cli.md`.

## 7. Platform scope and limitations

- **Primary platform:** Linux x86_64 only. No cross-compilation claims.
- **Build engine:** CMake only. Packages requiring Autotools/Meson/custom configure flows remain catalog-only and are rejected with an explicit diagnostic.
- **Hardware/runtime:** embedded/hardware-facing libraries are not runtime-verified on hardware.
- **Catalog-only entries:** discoverable with verified source metadata but not buildable by Crux today (e.g. non-CMake or unsupported upstream build requirements).
- **Deferred/rejected as normal packages:** `musl` and `systemd` are not registered as standard linkable library entries in this milestone.
- **Dependency modeling:** no invented inter-package dependencies; no binary cache, remote index service, or new resolver/build engine introduced.

## 8. Quality gates and tests

Executed in this session:

- `nix develop --command cargo fmt --all -- --check` — **pass**
- `nix develop --command cargo clippy --all-targets --all-features -- -D warnings` — **pass**
- `nix develop --command cargo check` — **pass**
- `nix develop --command cargo test --test official_registry` — **pass** (6/6)
- Verified-tier install matrix: release and debug installs completed for all 15 packages (`/tmp/opencode/m22-verification-release.log`, `/tmp/opencode/m22-verification-debug.log`)
- Reference applications: all 5 passed `check`/install/build/run as listed above

Pending/full-gate runs recorded separately in the session: `cargo test --all`, `cargo build --release`, and `crux-site` build (see completion summary).

## 9. Known issues

- Upstream CMake incompatibilities required deterministic registry build options for `cJSON` and `benchmark`; similar catalog-only-to-verified promotions may need per-package options in future milestones.
- Resolved in the M22 follow-up: `crux lock`/`crux resolve` no longer fall back to the in-memory test catalog. Fresh projects bootstrap the official registry automatically; `CRUX_OFFLINE=1` prevents network bootstrap and produces an actionable error when no local registry is available. Integration tests use isolated Git registries instead of the production fixture.
- `fmt` upstream archive is `fmt-11.1.1.zip` and unpacks with a root `CMakeLists.txt` in practice, but `build.system` classification and verification remain the source of truth for support claims.

## 10. Final status

**M22 implementation completed on `main` (uncommitted).** The official registry now provides a real, checksum-verified 50-package catalog with 15 end-to-end verified CMake packages and five consuming reference applications, without breaking M21 behavior or introducing unauthorized build/registry infrastructure.
