---
title: Official Package Registry
description: Curated C and C++ packages available from the default Crux registry.
---

Crux includes a curated, Git-backed registry of **50 packages** and **53 concrete versions**. The registry is maintained in the `registry/` directory of the Crux repository and is configured automatically for new projects.

Every entry records an exact upstream archive URL and a SHA-256 digest computed from the downloaded archive. Crux never invents versions, URLs, or checksums.

---

## Verification tiers

| Tier | Meaning |
|---|---|
| **Verified** | Built, installed, integrated, and consumed by a real C/C++ application on Linux x86_64. |
| **Catalog only** | Discoverable with verified source metadata, but Crux cannot build it yet because the upstream build system is not CMake. `crux build` and `crux install` reject these entries explicitly. |
| **Deferred** | Tracked in the target catalog but not registered because it is a toolchain/runtime replacement or environment package rather than a normal linkable library. |

The verified CMake tier for M22 is:

`benchmark`, `cJSON`, `CLI11`, `catch2`, `cxxopts`, `doctest`, `fmt`, `googletest`, `nlohmann-json`, `pcre2`, `spdlog`, `tomlplusplus`, `yaml-cpp`, `yyjson`, and `zlib`.

---

## Category coverage

| Category | Packages |
|---|---:|
| Foundational utilities | 10 |
| Systems programming | 10 |
| Networking | 8 |
| Embedded Linux | 8 |
| Data formats | 7 |
| Testing tools | 7 |

---

## Using a package

```bash
crux init demo --language cpp
cd demo
crux add fmt
crux run
```

`crux add` resolves the dependency graph, updates `crux.toml` and `crux.lock`, then installs the dependency for the selected profile. `crux build` and `crux run` reuse the validated installation and generate the required include/library paths automatically.

For a complete pure C consumer using the verified `zlib` package, see `examples/m22/c-zlib-roundtrip` in the Crux repository. It exercises `crux check`, `crux install`, `crux build`, and `crux run` with real `<zlib.h>` includes and linker integration.

---

## Source data and contributions

The full package-by-package catalog, including exact versions and archive references, is maintained in `docs/guides/official-package-catalog.md` in the Crux repository. Intake rules and the verification matrix are in `docs/contributing/official-registry.md`.

Custom registries remain fully supported. Crux's automatic bootstrap never overwrites a registry that you configured yourself.
