# Crux Milestone 18 Completion Report: Project Configuration & Toolchain Detection

**Milestone**: M18 — Project Configuration & Toolchain Detection  
**Crux Version**: v0.2.0-dev  
**Date**: 2026-11-15  
**Author**: Lead Rust Systems Engineer  
**Status**: Completed & Verified  

---

## 1. Purpose & Scope

Crux Milestone 18 establishes project configuration validation and compiler/toolchain detection for Crux v0.2.0. Following the establishment of standardized project conventions in M17, M18 provides the necessary configuration and toolchain inspection foundation for the upcoming Milestone 19 (Application Build Engine).

### Milestone Objectives:
- **Project Configuration**: Parse and validate project language (`c`, `cpp`, `other`) and language standard settings (`c17`, `c23`, `c++20`, `c++23`).
- **Compiler Detection**: Automatically discover host C and C++ compilers or inspect explicit toolchains configured via `CC` and `CXX`.
- **Compiler Classification**: Classify compiler families (`GCC`, `Clang`, `AppleClang`, `MSVC`), parse clean semantic versions, and identify host target triples.
- **Capability Probing**: Validate whether the detected compiler supports the project's requested language standard using isolated, non-destructive probe compilations.
- **Other Project Safety**: Ensure that `other` projects bypass C/C++ compiler requirements entirely.
- **CLI Commands**: Extend `crux check` to report toolchain status alongside manifest validity, and introduce the dedicated `crux toolchain` command.

### Milestone Boundaries:
- **No Application Compilation**: M18 does **not** compile user applications or generate CMake build configurations. Application compilation and build artifact discovery belong exclusively to Milestone 19.

---

## 2. Repository & Architectural Context

Crux adheres to a strict single-crate modular subsystem architecture. M18 changes are strictly organized into designated subsystems:

1. **`src/project/config.rs`**: Reusable `ProjectConfig` abstraction decoupling configuration consumption from raw file paths and TOML parsing.
2. **`src/project/manifest.rs`**: Manifest extension supporting `[project]` schema alongside backward-compatible `[package]` schemas, with standard validation and normalization.
3. **`src/build/toolchain.rs`**: Toolchain detection engine, compiler family classification, version parsing, target triple discovery, and standard capability probing.
4. **`src/cli/commands/check.rs`**: Integration of toolchain diagnostics into `crux check`.
5. **`src/cli/commands/toolchain.rs`**: Dedicated `crux toolchain` subcommand implementation.
6. **`src/cli/args.rs` & `src/cli/mod.rs`**: Command-line argument parsing and CLI error propagation.

### Crux-Managed CMake (Option A) Architecture:
Under Option A, Crux is the primary project and build manager. Users configure projects via `crux.toml`, Crux detects toolchains internally, and in M19 Crux will generate CMake build files under `.crux/build/generated/`. Users are never expected to maintain or configure a root-level `CMakeLists.txt`.

---

## 3. Configuration Data Models

M18 introduces `ProjectConfig` to encapsulate validated project configuration:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    pub name: String,
    pub version: semver::Version,
    pub language: ProjectLanguage,
    pub standard: Option<String>,
    pub source_dir: PathBuf,
}
```

### Key Properties:
- **`language`**: Strongly typed `ProjectLanguage` enum (`C`, `Cpp`, `Other`).
- **`standard`**: Normalized language standard string (`c17`, `c23` for C; `c++20`, `c++23` for C++; `None` for Other).
- **`source_dir`**: Defaulted to `src/` under the project root, adhering to the discovery rule: *Crux recursively discovers application source files under `src/` only.*
- **Factory Methods**:
  - `ProjectConfig::from_manifest(manifest, project_root)`: Builds a validated configuration from an in-memory `Manifest`.
  - `ProjectConfig::load_from_dir(project_root)`: Reads and parses `crux.toml` from disk, returning `Result<ProjectConfig, ConfigError>`.

---

## 4. Manifest & Lockfile Integration

### Manifest Support:
The manifest parser (`src/project/manifest.rs`) accepts both the modern `[project]` section and the legacy `[package]` section:

```toml
[project]
name = "demo-cpp"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
```

- When `language` is omitted, defaults to `cpp`.
- When `standard` is omitted, defaults to `c++20` (for C++) or `c17` (for C).
- Incompatible combinations (e.g. `language = "c"` with `standard = "c++20"`, or `standard` specified for `other`) are rejected during validation with structured `ManifestValidationError` diagnostics.

### Lockfile Boundary:
Lockfile generation and synchronization (`crux.lock`) remain strictly atomic and byte-for-byte deterministic. `crux check` and `crux toolchain` do not mutate `crux.lock` or `crux.toml`.

---

## 5. Toolchain Detection Engine

The toolchain detection engine (`src/build/toolchain.rs`) provides comprehensive discovery for host C and C++ compilers:

### Compiler Resolution Order:
1. **Environment Variables**:
   - For C: `$CC`
   - For C++: `$CXX`
2. **Standard Candidates**:
   - For C: `gcc`, `clang`
   - For C++: `g++`, `clang++`

### Information Extracted:
- **Compiler Family**: Detected via `--version` signature analysis:
  - `CompilerFamily::Gcc`: GCC / GNU Compiler Collection
  - `CompilerFamily::Clang`: LLVM Clang
  - `CompilerFamily::AppleClang`: Apple Clang
  - `CompilerFamily::Msvc`: Microsoft Visual C++
- **Clean Semantic Version**: Extracted via `-dumpversion` or parsed from the first line of `--version` (e.g. `15.3.0`).
- **Target Triple**: Extracted via `-dumpmachine` or parsed from `Target:` line in `clang -v` (e.g. `x86_64-unknown-linux-gnu`).
- **Executable Path**: Absolute canonical path resolved via `which` or standard path search.

---

## 6. Standard Capability Probing

Rather than assuming compiler capability from version numbers alone, Crux performs non-destructive capability probing:

1. **Standard Flag Mapping**:
   - C: `c17` -> `-std=c17`, `c23` -> `-std=c2x` / `-std=c23`
   - C++: `c++20` -> `-std=c++20`, `c++23` -> `-std=c++23`
2. **Safe Probe Execution**:
   - Creates an isolated temporary directory.
   - Writes a minimal translation unit: `int main(void) { return 0; }`.
   - Invokes `<compiler> -std=<flag> -c <probe_source> -o <probe_obj>`.
   - Cleans up all temporary files immediately.
3. **Diagnostics**:
   - If the compiler exits with 0, the standard is supported.
   - If the compiler fails, the standard is classified as unsupported, providing clear attribution.

---

## 7. Other Language Handling

For projects initialized with `--language other`:
- `ProjectLanguage::Other` indicates that the project does not use C or C++ compilers.
- `crux check` reports `Toolchain: Not applicable (Other project)` and marks the project as `Status: Valid`.
- `crux toolchain --language other` reports that compiler detection is not applicable.
- No subprocesses are spawned, and no missing-compiler errors are triggered.

---

## 8. CLI Commands & Output

### 8.1 `crux check`
`crux check` inspects both the manifest validity and the toolchain availability:

```text
Checking Crux project...
Manifest: crux.toml
Package: my-app
Version: 0.1.0
Language: C++
Standard: c++20

Toolchain:
  Compiler: GCC
  Version: 15.3.0
  Target: x86_64-unknown-linux-gnu
  Status: Available

Status: Valid
```

If the requested standard is unsupported or the compiler is missing:
```text
Manifest: crux.toml
Status: Invalid
error: compiler 'gcc' does not support requested language standard 'c23'
```

### 8.2 `crux toolchain`
The dedicated `crux toolchain` command inspects host compilers:

```bash
# Default: inspects both C and C++ toolchains
crux toolchain
```

Output:
```text
Crux Toolchain Information

Platform:
  OS: linux
  Architecture: x86_64

C Toolchain:
  Compiler: GCC
  Version: 15.3.0
  Target: x86_64-unknown-linux-gnu
  Executable: /run/current-system/sw/bin/gcc
  Standards: C17 (supported), C23 (supported)

C++ Toolchain:
  Compiler: GCC
  Version: 15.3.0
  Target: x86_64-unknown-linux-gnu
  Executable: /run/current-system/sw/bin/g++
  Standards: C++20 (supported), C++23 (supported)
```

Filtered execution:
```bash
crux toolchain --language c
crux toolchain --language cpp
crux toolchain --language other
```

---

## 9. Verification Results

All quality gates were executed and passed cleanly:

| Check / Command | Exit Code | Result | Details |
|---|---|---|---|
| `cargo fmt --all -- --check` | 0 | Passed | 0 formatting discrepancies |
| `cargo clippy --all-targets -- -D warnings` | 0 | Passed | 0 warnings, 0 errors |
| `cargo test --all` | 0 | Passed | All unit, integration, and CLI tests passing |
| `cargo check` | 0 | Passed | Clean compilation check |
| `tests/init.rs` | 0 | Passed | 16/16 tests passing |
| `tests/check.rs` | 0 | Passed | 13/13 tests passing |
| `tests/toolchain.rs` | 0 | Passed | 7/7 tests passing |

---

## 10. Git Status & Diffs

In accordance with autonomous agent rules (`AGENTS.md`), no git commits or remote pushes were performed. All changes remain clean in the working tree for maintainer review.

### Modified Files:
- `src/build/mod.rs`: Re-exports toolchain types (`CompilerFamily`, `CompilerInfo`, `PlatformInfo`, `ProjectToolchain`).
- `src/build/toolchain.rs`: Implements toolchain detection, parsing, and capability probing.
- `src/cli/args.rs`: Adds `toolchain` arguments and help messages.
- `src/cli/commands/check.rs`: Integrates toolchain reporting into `crux check`.
- `src/cli/commands/init.rs`: Implements minimal initialization.
- `src/cli/commands/mod.rs`: Registers `toolchain` command module.
- `src/cli/commands/toolchain.rs`: Subcommand handler for `crux toolchain`.
- `src/cli/mod.rs`: Dispatches `Action::Toolchain` and adds argument tests.
- `src/project/config.rs`: Reusable `ProjectConfig` data model.
- `src/project/init.rs`: Minimal project scaffolding ("Start small").
- `src/project/manifest.rs`: Dual schema `[project]` / `[package]` support and validation.
- `src/project/mod.rs`: Re-exports `ProjectConfig`.
- `src/lib.rs`: Exposes `ProjectConfig` and toolchain types.
- `tests/init.rs`: Integration tests for minimal project structure.
- `tests/check.rs`: Integration tests for toolchain check diagnostics.
- `tests/toolchain.rs`: Integration tests for `crux toolchain`.
- `README.md`: Updated project conventions and CLI usage.
- `docs/CLI_REFERENCE.md`: Updated reference guide.
- `docs/reports/project-initialization-report.md`: Documented project initialization changes.
- `docs/reports/toolchain-and-config-report.md`: This comprehensive report.

---

## 11. Backward Compatibility

- **Legacy Manifests**: Projects using `[package]` without `language` continue to validate cleanly; toolchain inspection is safely omitted or defaults gracefully.
- **Existing Commands**: All existing commands (`crux init`, `crux check`, `crux lock`, `crux resolve`, `crux fetch`, `crux build`, `crux install`, `crux integrate`) maintain full operational compatibility.
- **Lockfiles**: `crux.lock` schema and checksum verification are 100% untouched.

---

## 12. Future M19 Preparation

Milestone 19 (Application Build Engine) will directly consume the foundations built in M18:
1. **`ProjectConfig`**: M19 will invoke `ProjectConfig::load_from_dir(project_dir)` to retrieve application name, language, standard, and `src/` source path.
2. **`ProjectToolchain`**: M19 will invoke `detect_project_toolchain(&config)` to obtain the validated compiler path, family, and standard flags.
3. **CMake Generation**: M19 will translate `ProjectConfig` and `ProjectToolchain` into an internal `CMakeLists.txt` under `.crux/build/generated/`, compiling all discovered files under `src/`.

---

## 13. Suggested Conventional Commits

For the repository maintainer:

```text
feat(project): simplify default project structure to minimal layout (M17 follow-up)
feat(project): introduce ProjectConfig domain model and manifest validation
feat(build): implement compiler and toolchain detection engine with capability probing
feat(cli): integrate toolchain inspection into crux check
feat(cli): introduce crux toolchain command
test(toolchain): add integration test suite for toolchain detection
docs: document minimal project conventions and M18 completion report
```

---

## 14. Suggested Pull Request

### Title:
`feat: project configuration validation & compiler toolchain detection (M18)`

### Description:
```markdown
## Summary
This PR delivers Crux Milestone 18 (Project Configuration & Toolchain Detection) and incorporates the M17 follow-up for minimal default project structures.

### Key Changes
1. **Minimal Project Structure (M17 Follow-up)**:
   - `crux init` generates only essential files: `crux.toml`, `crux.lock`, and `src/main.c` (or `src/main.cpp`).
   - Removes empty starter boilerplate (`include/`, `tests/`, `.crux/`, `src/modules/`).
   - Formally establishes the discovery rule: Crux recursively discovers application sources under `src/` only.

2. **Project Configuration Domain Model (`ProjectConfig`)**:
   - Decoupled `ProjectConfig` representing validated project name, version, language, standard, and source paths.
   - Dual support for modern `[project]` and legacy `[package]` manifest schemas.

3. **Compiler Detection Engine (`src/build/toolchain.rs`)**:
   - Discovers C (`gcc`, `clang`, `$CC`) and C++ (`g++`, `clang++`, `$CXX`) compilers.
   - Extracts compiler family, version, and host target triple.
   - Probes language standard compatibility (`c17`, `c23`, `c++20`, `c++23`) via isolated non-destructive compilation probes.
   - Safely bypasses compiler requirements for `other` projects.

4. **CLI Enhancements**:
   - `crux check`: Now reports compiler and toolchain status.
   - `crux toolchain`: New command for inspecting host toolchain diagnostics.

### Verification
- `cargo fmt --all -- --check`: Passed (0 errors)
- `cargo clippy --all-targets -- -D warnings`: Passed (0 errors)
- `cargo test --all`: All unit, integration, and CLI tests passing
```
