# Official Package Catalog

This guide lists the M22 target catalog for the official Crux registry. The registry is maintained in [`registry/`](../../registry/) and is bootstrapped automatically for new projects.

M22 registers **50 distinct packages** and **53 concrete versions**. The three additional versions are retained v0.1.0 compatibility entries (`fmt 11.0.2`, `zlib 1.3.0`, and `catch2 3.7.1`).

> **Verification scope:** Linux x86_64, Debug and Release profiles, upstream CMake backend only. "Catalog only" means the exact archive, URL, and SHA-256 are registered, but Crux cannot yet build the package because its upstream build system is not CMake or it has not completed the M22 verification matrix.

| Category | Package | Version(s) Added | Status |
|---|---|---|---|
| **Foundational utilities** | zlib | 1.3.0, 1.3.1 | Verified |
| | zstd | 1.5.7 | Catalog only |
| | bzip2 | 1.0.8 | Catalog only |
| | xz | 5.8.4 | Catalog only |
| | lz4 | 1.11.1 | Catalog only |
| | libarchive | 3.8.11 | Catalog only |
| | fmt | 11.0.2, 11.1.1 | Verified |
| | spdlog | 1.17.0 | Verified |
| | CLI11 | 2.7.2 | Verified |
| | tomlplusplus | 3.4.0 | Verified |
| **Systems programming** | libuv | 1.53.0 | Catalog only |
| | libevent | 2.1.13 | Catalog only |
| | liburing | 2.15.0 | Catalog only |
| | libcap | 2.78.0 | Catalog only |
| | libseccomp | 2.6.1 | Catalog only |
| | libudev | 262.2.0 | Catalog only |
| | elfutils | 0.197.0 | Catalog only |
| | libelf | 0.197.0 | Catalog only |
| | musl | 1.2.5 | Deferred |
| | systemd | 262.2.0 | Deferred |
| **Networking** | libcurl | 8.23.0 | Catalog only |
| | OpenSSL | 4.1.0 | Catalog only |
| | mbedTLS | 4.3.0 | Catalog only |
| | libssh2 | 1.11.1 | Catalog only |
| | nghttp2 | 1.71.0 | Catalog only |
| | paho-mqtt | 1.3.16 | Catalog only |
| | zeromq | 4.3.5 | Catalog only |
| | c-ares | 1.34.9 | Catalog only |
| **Embedded Linux** | libgpiod | 2.3.2 | Catalog only |
| | libi2c | 4.4.0 | Catalog only |
| | libserialport | 0.1.2 | Catalog only |
| | libusb | 1.0.30 | Catalog only |
| | libevdev | 1.14.0 | Catalog only |
| | libdrm | 2.4.134 | Catalog only |
| | libinput | 1.32.0 | Catalog only |
| | protobuf | 36.2.0 | Catalog only |
| **Data formats** | nlohmann-json | 3.12.0 | Verified |
| | yyjson | 0.13.0 | Verified |
| | cJSON | 1.7.19 | Verified |
| | libxml2 | 2.15.4 | Catalog only |
| | sqlite | 3.54.1 | Catalog only |
| | lmdb | 1.0.2 | Catalog only |
| | yaml-cpp | 0.9.0 | Verified |
| **Testing tools** | catch2 | 3.7.1, 3.8.0 | Verified |
| | googletest | 1.18.0 | Verified |
| | doctest | 2.5.3 | Verified |
| | benchmark | 1.10.0 | Verified |
| | cxxopts | 3.3.1 | Verified |
| | pcre2 | 10.49.0 | Verified |
| | xxhash | 0.8.4 | Catalog only |

*Status legend*:

- **Verified** — discoverable, downloaded from the exact upstream archive, SHA-256 verified, CMake-configured, built, installed, integrated, and consumed by at least one M22 reference application.
- **Catalog only** — discoverable with an exact upstream URL and verified SHA-256 digest, but not buildable by Crux today. `crux build` and `crux install` reject these entries with an explicit diagnostic.
- **Deferred** — recorded in the target catalog but intentionally not registered because the source is a toolchain/runtime replacement or a build environment rather than a normal linkable C/C++ library.

## Canonical package names

Registry directory names are canonical. `catch2` is lowercase; `CLI11`, `OpenSSL`, `mbedTLS`, and `cJSON` retain their upstream capitalization because package identity is case-sensitive.

## Dependency graph policy

M22 does not invent dependency edges. Every verified package declares an empty `[dependencies]` table in its `package.toml`; the resolver therefore produces a flat, deterministic graph with one node per selected package. Reports in [`docs/reports/package-ecosystem-expansion-report.md`](../reports/package-ecosystem-expansion-report.md) record exact versions, archive references, digests, and verification results.

## Adding a package

See [`docs/contributing/official-registry.md`](../contributing/official-registry.md) for the intake checklist. At minimum, a package entry must contain:

- a strict `X.Y.Z` version directory;
- a real upstream archive URL;
- a SHA-256 digest computed from that exact downloaded archive;
- the upstream repository/homepage where available;
- `system = "cmake"` only after the package passes the full verification matrix.
