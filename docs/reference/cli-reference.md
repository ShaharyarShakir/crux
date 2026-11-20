# Crux Command-Line Interface Reference

**Version**: 0.2.0  
**Binary Name**: `crux`  

---

## 1. Global Usage & Exit Codes

```text
Usage:
    crux [options]
    crux <command> [args]
```

### Global Options

| Option | Short | Description |
|---|---|---|
| `--help` | `-h` | Display top-level help or subcommand help when combined with `<command>` |
| `--version` | `-V` | Print Crux version information (`crux 0.2.0`) |

### Standard Exit Codes

| Exit Code | Classification | Meaning |
|---|---|---|
| `0` | Success | Operation completed successfully |
| `1` | Runtime / General Error | Command failed (e.g. build failure, lockfile out-of-sync, network failure, missing file) |
| `2` | Argument Parsing Error | Unknown flag, invalid argument value, or missing required parameter |

---

## 2. Command Reference

### `crux init`
Scaffolds a new Crux project or package with standardized conventions, `crux.toml`, initialized `crux.lock`, and working starter applications for C and C++.

```text
Usage:
    crux init [path] [options]

Arguments:
    [path]               Target directory for the project (defaults to current directory)

Options:
    -n, --name <NAME>    Specify custom package/project name (defaults to directory name)
    -l, --language <LANG> Project language: c, cpp (or c++), other
    -s, --standard <STD> Language standard (e.g. c17 for C, c++20 for C++)
    -h, --help           Print help information
```

**Behavior**:
- Interactive selection: Prompts user to select C, C++, or Other when stdin is a terminal and `--language` is omitted.
- Non-interactive mode: Supports piped input or safely defaults to C++ (`c++20`) in automated environments.
- Minimal default structure ("Start small. Let projects grow naturally."):
  - **C**: Generates `crux.toml`, `crux.lock`, and `src/main.c`.
  - **C++**: Generates `crux.toml`, `crux.lock`, and `src/main.cpp`.
  - **Other**: Generates `crux.toml`, `crux.lock`, and an empty `src/` directory.
- Optional directories (`src/modules/`, `include/`, `tests/`, `.crux/`) may be created by users as their project grows, or by Crux commands on demand.
- Source Discovery Rule: Crux recursively discovers application source files under `src/` only.
- Generates compilable starter application for C and C++; leaves directories clean for Other.
- Initializes canonical `crux.lock` version 1.
- Validates language-standard combinations (e.g. rejects C++ standards for C, or standards on Other).
- Atomic safety: Refuses to overwrite existing manifests, lockfiles, or user source files.
- Option A CMake Boundary: Ordinary Crux-managed projects do not require or generate a root-level `CMakeLists.txt`.

---

### `crux import`
Safely adopts an existing C/C++ project, reports detected structure, and writes a conservative Crux manifest without modifying existing project files.

```text
Usage:
    crux import [options]

Options:
    -p, --path <DIR>  Project directory to import (defaults to current directory)
    --dry-run         Show detected facts and proposed crux.toml without writing files
    --force           Accept low-confidence layouts and refresh Crux-managed import metadata;
                      an existing crux.toml is never overwritten
    -h, --help        Print help information
```

**Behavior**:
- Performs read-only, deterministic discovery; never executes project build scripts or accesses the network.
- Recognizes `CMakeLists.txt`, `Makefile`, `makefile`, `GNUmakefile`, conventional source/include directories, root-level C/C++ sources, likely entry points, and test directories.
- Ignores `.git`, `.crux`, `build`, `out`, `dist`, `node_modules`, `target`, `vendor`, hidden paths, and symlinks escaping the project root.
- Preserves existing sources, headers, CMake files, Makefiles, and any existing `crux.toml`.
- Creates `crux.toml` using only the existing manifest schema and records observed source/include roots in Crux-managed `.crux/import.toml`.
- Reuses the existing application build engine: `crux build` and `crux run` compile detected sources into an internal CMake project under `.crux/build/generated/`.
- Reports detected facts separately from assumptions and limitations. Low-confidence layouts fail with an actionable diagnostic unless `--force` is supplied.
- `--force` never overwrites `crux.toml`; it only accepts low-confidence discoveries and may explicitly refresh `.crux/import.toml`.

See [Importing Existing C/C++ Projects](../guides/import-existing-projects.md) for supported layouts, CMake compatibility, Makefile limitations, and a complete example.

---

### `crux check`
Validates syntactic and semantic correctness of the project manifest (`crux.toml`) and inspects toolchain availability.

```text
Usage:
    crux check [options]

Options:
    -h, --help       Print help information
```

**Checks Performed**:
- Valid TOML syntax.
- Required `[project]` or legacy `[package]` metadata (`name`, `version`).
- Valid SemVer 2.0.0 package version.
- Language and standard validation (supported C/C++ standards).
- Toolchain inspection: Verifies compiler availability for C and C++ projects and tests standard compatibility.
- Dependency name and constraint syntax.

---

### `crux toolchain`
Detects and inspects installed C and C++ compilers, reporting compiler family, version, target triple, executable path, and supported language standards.

```text
Usage:
    crux toolchain [options]

Options:
    -l, --language <LANG> Filter toolchain inspection by language (c or cpp)
    -h, --help           Print help information
```

**Behavior**:
- Without `--language`: Automatically detects both C and C++ toolchains on the host platform. If run inside a project directory, evaluates compatibility against the project's requested standard.
- With `--language c`: Inspects the C compiler (`gcc`, `clang`, or `$CC`).
- With `--language cpp`: Inspects the C++ compiler (`g++`, `clang++`, or `$CXX`).
- With `--language other`: Reports that compiler detection is not applicable for Other projects.
- Standard capability probing: Performs isolated, non-destructive probe compilations in a temporary directory to verify compiler support for standards (`c17`, `c23`, `c++20`, `c++23`).
- Compiler override: Respects `CC` and `CXX` environment variables. If an override path is invalid or missing, reports clear, actionable errors.

---

### `crux resolve`
Executes dependency resolution against active registries and previews the resolved graph without modifying disk state.

```text
Usage:
    crux resolve [options]

Options:
    -h, --help       Print help information
```

**Behavior**:
- Queries the active Git registry only; fresh projects bootstrap the official registry automatically. There is no in-memory production fallback.
- Evaluates constraints with strict SemVer precedence.
- Enforces single-version One Definition Rule (ODR) collision prevention.
- Prints resolution summary with resolved versions and package sources.

---

### `crux add`
Adds a direct dependency, resolves the full graph (including transitives), updates `crux.toml` and `crux.lock`, and installs the dependency for the Debug profile.

```text
Usage:
    crux add <package>[@<constraint>]

Arguments:
    <package>        Package name to add (for example, fmt)
    @<constraint>    Optional SemVer constraint (for example, @^11.0)

Options:
    --git <URL>       Use a Git repository source
    --tag <TAG>       Resolve a Git tag (with --git)
    --branch <BRANCH> Track a Git branch (with --git); resolve and lock its commit
    --rev <REV>       Resolve an arbitrary Git revision (with --git)
    --commit <SHA>    Pin a full 40-hex Git commit (with --git)
    --url <URL>       Use a remote archive source (requires --sha256)
    --sha256 <DIGEST> SHA-256 of the remote archive (with --url)
    --path <PATH>     Use a local package directory
    -h, --help        Print help information
```

**Behavior**:
- Without a constraint, Crux uses `"*"` (any version), which resolves to the highest available version.
- Direct sources are written as inline tables in `[dependencies]`. Git refs resolve to immutable commits in `crux.lock`; archive digests are mandatory; local paths must contain a valid `package.toml`.
- A package name cannot be declared both by the registry and by a direct source. Conflicting direct sources, invalid refs, and source cycles fail before files are modified. See [Direct Package Sources](../guides/direct-sources.md).
- The manifest's `[dependencies]` table is updated in place; comments, ordering, and unrelated formatting are preserved.
- Existing lockfile preferences are reused so compatible pinned versions are not churned.
- The updated lockfile is constructed in memory before any write. `crux.toml` is written first; if the lockfile write fails, the previous manifest is restored.
- Duplicate dependencies and invalid specs fail before any file is modified.
- Installation uses the Debug profile so the default `crux build` / `crux run` flow works immediately.

---

### `crux remove`
Removes a direct dependency, re-resolves the remaining graph, and updates `crux.toml` and `crux.lock`.

```text
Usage:
    crux remove <package>

Arguments:
    <package>        Name of the direct dependency to remove

Options:
    -h, --help       Print help information
```

**Behavior**:
- Removes only the named direct dependency; shared transitive dependencies that are still required remain locked.
- Resolution and lockfile construction happen before writes, and the manifest is restored if the lockfile cannot be written.
- Cached sources and installed files are intentionally left in place. Crux never deletes files it cannot confidently attribute to Crux-managed installations.
- Removing a dependency that is not declared fails with the list of currently declared dependencies.

---

### `crux lock`
Manages deterministic lockfile (`crux.lock`) generation, synchronization, and integrity checks.

```text
Usage:
    crux lock [options]

Options:
    --update         Upgrade all dependencies to latest compatible versions (re-resolves)
    --check          Verify lockfile validity and manifest synchronization without modifying files
    -h, --help       Print help information
```

**Behavior**:
- `crux lock`: Synchronizes `crux.lock` with `crux.toml`, preserving existing locked versions if still compatible.
- `crux lock --update`: Discards locked version preferences and resolves highest compatible versions.
- `crux lock --check`: Exits with code `1` if `crux.lock` is missing, invalid, or out-of-sync with `crux.toml`.

---

### `crux registry`
Manages Git package registries.

```text
Usage:
    crux registry <subcommand> [args]

Subcommands:
    list             List all configured registries and package/version counts
    add <name> <url> Add a new Git package registry (clones into managed registry store)
    update <name>    Update an existing Git registry to the latest remote commit

Options:
    -h, --help       Print help information
```

**Notes**:
- Fresh projects automatically use the official Crux registry (`https://github.com/ShaharyarShakir/crux.git`, registry root in its `registry/` subdirectory) without running `crux registry add`.
- `<url>` can be a remote Git URL (`https://...` or `git@...`) or a local filesystem directory.
- Registry definitions are stored under `~/.crux/registries/<name>/` and registered in `~/.crux/config.toml` (or the project's `.crux/config.toml` when a project is active).
- The official checkout is reused when present, so subsequent runs work offline. `crux registry update default` refreshes it from upstream.
- The M22 official catalog contains 50 packages and 53 versions. See `docs/guides/official-package-catalog.md` for exact versions, digests, and support status.
- `crux registry list` displays a `Subdir:` line when a registry root is below the checkout root.
- Existing custom registry configuration is never overwritten by automatic bootstrap.

---

### `crux fetch`
Downloads source archives, validates cryptographic SHA-256 digests, and safely extracts them into content-addressed cache storage.

```text
Usage:
    crux fetch [package] [options]

Arguments:
    [package]        Optional package name or name@version to fetch (defaults to all locked dependencies)

Options:
    --force          Force re-download and re-extraction even if cached
    -h, --help       Print help information
```

**Security Guards**:
- TLS required for HTTP downloads.
- SHA-256 verified before extraction; partial downloads deleted on mismatch.
- Zip Slip and symlink escape defenses enforced during extraction.
- Decompression bomb limits (500 MB max uncompressed size, 50,000 files, 100:1 ratio).

---

### `crux build`
Builds the local Crux application (for `[project]` manifests) or compiles package sources using the internal CMake engine.

```text
Usage:
    crux build [package] [options]

Arguments:
    [package]        Optional package name or name@version to build
                     (omit to build the local application)

Options:
    --debug          Build with debug configuration (default for applications)
    --release        Build with release configuration
    --profile <type> Build profile: 'debug' or 'release' (defaults to release for packages, debug for applications)
    -v, --verbose    Show verbose compiler and CMake output
    --force          Force rebuild even if cache is valid
    -h, --help       Print help information
```

**Dispatch rule**:
- `crux build <package>` builds that package from the configured registry.
- `crux build` with a `[project]` manifest builds the local application.
- `crux build` with a legacy `[package]` manifest (or only a `crux.lock`) retains v0.1.0 behavior and builds locked dependencies.

**Application build behavior**:
- Loads and validates `crux.toml`, then reuses project configuration and toolchain detection.
- Recursively discovers C sources (`.c`) or C++ sources (`.cpp`, `.cc`, `.cxx`) under `src/` only.
- Rejects mixed C/C++ sources, unsupported source extensions, and language `other` with actionable diagnostics.
- Ensures declared dependencies (including transitives) are locked and installed for the selected profile before configuring CMake. Missing or incomplete installations are installed automatically, with actionable diagnostics if installation fails.
- Derives include directories, library directories, and library files from validated `installed.toml` metadata and emits them into the generated CMake project. Static libraries are preferred when both static and shared variants are installed; dependents are linked before their dependencies.
- Emits an RPATH for shared-library dependencies so the executable resolves them from the Crux install tree.
- Generates internal CMake configuration under `.crux/build/generated/CMakeLists.txt`.
- Configures and compiles out-of-source into `.crux/build/debug/` or `.crux/build/release/`.
- Writes configure/build logs to `.crux/build/logs/`.
- Requires no root-level `CMakeLists.txt`; never modifies user sources or root CMake configuration.
- Defaults to Debug; `--release` selects Release. Debug and Release artifacts and dependency fingerprints are isolated.
- `--verbose` prints detected compiler/CMake paths, discovered sources, linked dependencies, and captured CMake output.
- `--force` discards the selected profile build tree and performs a clean rebuild.

**Package build behavior**:
- Rejects registry entries whose `[build].system` is not `cmake` with an explicit `catalog-only` diagnostic before downloading or configuring the source.
- Automatically fetches and extracts dependencies if missing.
- Computes deterministic build fingerprints based on package identity, profile, source checksum, and toolchain info.
- Preserves full compiler diagnostics in `.crux/builds/<pkg>/<version>/<fingerprint>/logs/`.

---

### `crux run`
Builds and runs a C/C++ application using the M19 application build engine, either from a `[project]` manifest or from ad-hoc sources.

```text
Usage:
    crux run [options] [-- <args>...]
    crux run --bin <source> [options] [-- <args>...]
    crux run --sources <source>... [options] [-- <args>...]

Options:
    --bin <source>   Run one C/C++ source file without a crux.toml manifest
    --sources <list> Run one or more sources (repeat the flag, comma-separated, or space-separated)
    -I, --include-dir <dir>
                     Add an include directory for ad-hoc runs (repeatable)
    --debug          Build and run the debug configuration (default)
    --release        Build and run the release configuration
    -v, --verbose    Show verbose compiler and CMake output
    -h, --help       Print help information

Arguments:
    <args>...        Arguments forwarded to the application (after `--`)
```

**Behavior**:
- Reuses the M19 application build engine (`crux build` for `[project]` manifests). `crux run` does not implement a separate build system.
- Ad-hoc mode is selected by `--bin` or `--sources` and needs no `crux.toml` or registry. `.c` selects C; `.cpp`, `.cc`, and `.cxx` select C++. Header files (`.h`, `.hpp`, `.hxx`, `.hh`, `.inl`, `.ipp`) may be listed in `--sources` so they are copied into the scratch source tree. `--sources` accepts repeated flags, comma-separated values, and space-separated values; a space-separated list ends at the next Crux option or `--`.
- Ad-hoc include directories accept `-I <dir>`, `-I<dir>`, `--include-dir <dir>`, and `--include-dir=<dir>` and may be repeated. Relative paths resolve from the current working directory.
- Ad-hoc sources and artifacts are isolated under `.crux/run/<name>-<hash>/`. The scratch workspace is discarded before each invocation, so a failed build cannot execute a stale binary. Debug and Release artifacts remain in separate `.crux/build/<profile>/` directories inside that workspace.
- Ensures declared dependencies are locked and installed for the selected profile before compiling, using the same dependency-ensure step as `crux build`.
- Defaults to Debug; `--debug` selects Debug and `--release` selects Release. The executable from the selected configuration is run (never a mismatched profile).
- Invokes incremental CMake builds as needed; compilation failures stop execution and Crux does not run a stale/previous executable.
- Arguments after `--` are forwarded unchanged to the application (order, spaces, and leading dashes preserved). Crux options and application arguments are separated by `--`.
- The application is launched with **working directory = project root** (relative file paths resolve in the project root, not `.crux/build/`).
- The application inherits the parent environment by default; stdin/stdout/stderr are inherited so interactive programs and real-time output work normally. Shared dependency library directories are prepended to the platform loader path (`LD_LIBRARY_PATH`, `DYLD_LIBRARY_PATH`, or `PATH` on Windows).
- On success Crux exits with status `0`. On application nonzero exit Crux propagates that exit status. On Unix signal termination Crux propagates platform-appropriate status where feasible. Crux execution/launch failures are reported as Crux errors.
- Project mode supports C and C++ `[project]` applications only. Projects with `language = "other"` receive a clear diagnostic. Ad-hoc mode supports C and C++ source sets; mixed C/C++ compilation units are rejected.

**Relationship to `crux build`**:
- `crux build` compiles the application and reports the executable path.
- `crux run` performs the same build step (Debug by default) and then executes the resulting binary.


### `crux install`
Builds and installs packages into project-local storage (`.crux/installed/`) using staged publication.

```text
Usage:
    crux install [package] [options]

Arguments:
    [package]        Optional package name or name@version to install (defaults to all locked dependencies)

Options:
    --profile <name> Build profile: 'debug' or 'release' (defaults to 'release')
    --force          Force reinstall even if already installed with matching fingerprint
    --locked         Require exact synchronization with crux.lock; fails if lockfile missing or modified
    -h, --help       Print help information
```

**Behavior**:
- Rejects catalog-only packages before acquisition when their declared build system is not CMake.
- Runs `cmake --install` into a private staging directory.
- Inspects and inventories all installed files into `installed.toml`.
- Atomically renames staging directory to destination on successful verification.

---

### `crux integrate`
Generates consumer build-system configuration files (e.g. CMake integration script).

```text
Usage:
    crux integrate <subcommand> [options]

Subcommands:
    cmake            Generate relocatable CMake integration script

Options (for cmake):
    --profile <type> Build profile: 'debug' or 'release' (defaults to 'release')
    --output <path>  Custom output path (defaults to '.crux/cmake/CruxDependencies.cmake')
    -h, --help       Print help information
```

**Generated Script Features**:
- Defines one `_CRUX_<PKG>_PREFIX` variable per validated installation.
- Prepends those prefixes to `CMAKE_PREFIX_PATH` and appends discovered module directories to `CMAKE_MODULE_PATH`.
- Relocatable path calculation relative to `${CMAKE_CURRENT_LIST_DIR}`.
- Does not fabricate imported targets or emit `find_package()` calls; consumers use upstream-provided config/module files or CMake built-in find modules. No `<Package>_ROOT` variable is generated.

---

## 3. Environment Variables

| Variable | Description | Default |
|---|---|---|
| `CRUX_HOME` | Root directory for Crux user data and configuration | `~/.crux` |
| `CRUX_CONFIG_PATH` | Path to global Crux configuration file | `$CRUX_HOME/config.toml` |
| `CRUX_REGISTRIES_DIR` | Directory containing cloned Git registries | `$CRUX_HOME/registries` |
| `CRUX_CACHE_DIR` | Directory for downloaded archives and extracted sources | `$CRUX_HOME/cache` |
| `CRUX_REGISTRY_PATH` | Direct override path to an active Git package registry | Unset |
| `CRUX_OFFICIAL_REGISTRY_URL` | Git URL or local checkout used to bootstrap the official registry (useful for mirrors/offline setups) | `https://github.com/ShaharyarShakir/crux.git` |
| `CRUX_OFFLINE` | When set to `1`, `true`, `yes`, or `on`, prevents network bootstrap. Crux uses an existing configured/cached registry or fails with an actionable diagnostic. | Unset |
| `CC` | Explicit C compiler executable for checks and application builds | Detected (`gcc`, then `clang`) |
| `CXX` | Explicit C++ compiler executable for checks and application builds | Detected (`g++`, then `clang++`) |

---

## 4. Configuration & Metadata Files

### `crux.toml` (Project Manifest)
Located at the root of a Crux-managed user project:
```toml
[project]
name = "my_project"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
fmt = "^11.1"
zlib = ">=1.2, <1.4"
```

Legacy `[package]` manifests remain supported for package projects. A `[project]` manifest declares a local application that `crux build` compiles from `src/`.

### `.crux/build/` (Generated Application Build Tree)
Created on demand by `crux build` for application projects:
```text
.crux/build/
├── generated/
│   └── CMakeLists.txt      # internal CMake project generated by Crux
├── debug/                  # Debug build tree and executable
├── release/                # Release build tree and executable
└── logs/
    ├── configure-debug.log
    ├── configure-release.log
    ├── build-debug.log
    └── build-release.log
```

These files are disposable internal artifacts. Projects should ignore `.crux/` in version control; Crux does not modify a user's `.gitignore` automatically.

### `crux.lock` (Lockfile)
Auto-generated canonical dependency record:
```toml
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"
```

### `config.toml` (Crux Global Configuration)
Located at `~/.crux/config.toml`:
```toml
[registries.default]
url = "https://github.com/ShaharyarShakir/crux-registry.git"
path = "/home/user/.crux/registries/default"
primary = true
```

### `installed.toml` (Installation Metadata)
Located at `.crux/installed/<pkg>/<version>/<fingerprint>/installed.toml`:
```toml
format_version = 1
package_name = "fmt"
package_version = "11.1.1"
fingerprint = "sha256:16980a71eba7eea86813317ad46fa0065484ce172c1e5fdeb234aaa0f721259c"
profile = "release"
total_size_bytes = 821076
success = true
installed_at = "2026-11-15T18:08:00Z"

[[installed_files]]
relative_path = "include/fmt/core.h"
file_type = "header"
size_bytes = 187
```
