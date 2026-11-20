# Importing Existing C/C++ Projects

Crux can adopt an existing C or C++ project without moving its sources, rewriting its build files, or executing its build scripts. `crux import` inspects a project directory, reports what it found, and writes a conservative `crux.toml` plus Crux-managed discovery metadata.

The generated project can then be inspected and built through the normal Crux workflow:

```bash
crux check
crux build
crux run
```

Import is read-only with respect to existing project content. It never deletes, renames, or edits source files, headers, `CMakeLists.txt`, Makefiles, or an existing `crux.toml`.

---

## 1. Basic Usage

From inside the existing project:

```bash
cd legacy-project
crux import
```

From another directory:

```bash
crux import --path ./legacy-project
crux import --path .
```

Preview without writing any files:

```bash
crux import --dry-run
```

A dry run prints the import report and the proposed `crux.toml`, then exits without creating `crux.toml` or `.crux/import.toml`.

---

## 2. What Import Detects

Crux performs a deterministic, read-only scan of the selected directory.

Recognized files and directories:

- `CMakeLists.txt`
- `Makefile`, `makefile`, and `GNUmakefile`
- `crux.toml` (existing Crux project)
- Conventional source directories: `src/`, `source/`, `app/`
- Conventional include directories: `include/`, `inc/`, `headers/`
- Root-level source files with extensions `.c`, `.cc`, `.cpp`, and `.cxx`
- Root-level headers with extensions `.h`, `.hh`, `.hpp`, and `.hxx`
- Conventional entry points such as `main.c` / `main.cpp`, plus source files containing a top-level `int main` declaration
- Test directories named `test/` or `tests/`

Crux reports detected build system, language, source directories, include directories, likely targets, source/header counts, and test directories. The report separates **Detected facts** from **Assumptions** so the generated configuration can be reviewed critically.

The following directories are never scanned: `.git`, `.crux`, `build`, `out`, `dist`, `node_modules`, `target`, and `vendor`. Hidden files and directories are also ignored. Symbolic links that resolve outside the project root are skipped and reported; they are not followed.

---

## 3. Supported Project Layouts

### Single source directory

```text
legacy-app/
├── src/
│   ├── main.cpp
│   └── math.cpp
└── include/
    └── math.hpp
```

This is the canonical Crux layout. Import records `src/` as the source root and `include/` as an include root.

### Multiple source directories

```text
legacy-app/
├── app/
│   └── main.c
└── source/
    └── helper.c
```

Import records both `app/` and `source/` in Crux-managed metadata. The build engine compiles both roots without moving the files.

### Root-level sources

```text
legacy-app/
├── main.c
├── helper.c
└── util.h
```

Root-level `.c`, `.cc`, `.cpp`, and `.cxx` files are supported. Import records the project root (`.`) as the source root. Generated or irrelevant directories such as `build/` and `dist/` are ignored.

### Mixed layouts

A project may combine root-level sources with conventional source directories. Import records every directory that contains a compilation unit. The current build engine still supports only one language, so a project that mixes C and C++ compilation units is rejected with an actionable diagnostic rather than silently mis-built.

---

## 4. Build System Compatibility

### CMake projects

If `CMakeLists.txt` exists at the project root, import:

- Reports the build system as `CMake`.
- Reads a simple top-level `project(<name> ...)` declaration when present and uses it as the package-name hint.
- Preserves `CMakeLists.txt` byte-for-byte.
- Never creates a competing root-level CMake project.

Crux does **not** consume or execute the existing CMake configuration during import. `crux build` and `crux run` compile the detected sources through Crux's internal CMake project under `.crux/build/generated/`. Existing `add_executable()`, target definitions, custom commands, and CMake options are not translated. Projects whose correct build depends on those definitions may need manual review after import.

### Makefile projects

If `Makefile`, `makefile`, or `GNUmakefile` exists, import:

- Reports the build system as `Make`.
- Preserves the Makefile.
- Never executes `make` or any Makefile recipe during discovery.
- Does not translate Makefiles into CMake.
- Generates a conservative manifest when C/C++ sources are discoverable.

The current Crux build engine does not use Makefiles. After import, `crux build` compiles detected sources with Crux's internal CMake backend. Projects that depend on generated sources, custom compiler flags, or nontrivial Makefile rules must be configured or adapted manually.

### No recognized build system

A plain directory containing C or C++ sources is importable. Import generates a manifest compatible with the application build engine and leaves the original layout intact. Developers are not required to create `src/`.

---

## 5. Generated Files

A successful import creates two files:

```text
legacy-app/
├── crux.toml
└── .crux/
    └── import.toml
```

### `crux.toml`

The manifest uses only the existing Crux schema. A typical result is:

```toml
[project]
name = "legacy-app"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
```

Import derives the name from the CMake `project()` declaration when available, otherwise from the directory name. The name is sanitized to a valid Crux package name. The version defaults to `0.1.0`. The language is inferred from source extensions, and the default language standard is selected by Crux.

Import never guesses dependencies. The `[dependencies]` table is empty unless the user adds packages later with `crux add`.

### `.crux/import.toml` (Crux-managed metadata)

The manifest schema intentionally has no keys for arbitrary legacy source roots or include directories. Crux records those discovered facts in `.crux/import.toml`:

```toml
version = 1
source_roots = ["app", "source"]
include_dirs = ["include"]
```

This metadata is Crux-managed. The build engine reads it to discover sources outside `src/` and to add project-relative include directories to the internal CMake target. The manifest remains the user-facing configuration file; metadata records observations, not user intent.

---

## 6. Options

| Option | Description |
|---|---|
| `--path <DIR>` | Import a directory other than the current directory. Relative paths resolve from the caller's working directory. |
| `--dry-run` | Report detected facts and print the proposed manifest without writing files. |
| `--force` | Accept low-confidence layouts (for example, no detected `main()` or multiple executable entry points) and allow refresh of Crux-managed `.crux/import.toml`. |
| `-h`, `--help` | Show command help. |

`--force` has strict limits:

- It **never** overwrites an existing `crux.toml`. If `crux.toml` exists, import always reports that the project is already initialized and preserves the file.
- It may refresh `.crux/import.toml` when the stored discovery facts no longer match the current project. The import report explicitly lists any refreshed file under `Updated (--force)`.
- It does not make impossible configurations buildable. Projects with no C/C++ compilation units or with mixed C/C++ sources are rejected even with `--force`.

---

## 7. Safety and Diagnostics

Import refuses to proceed when no C/C++ compilation units are found, when C and C++ sources are mixed, or when a usable executable target cannot be identified without `--force`. These errors include the detected build system, source directories, source count, include directories, likely targets, and explanatory facts.

Existing manifests are always preserved:

```text
error: project is already initialized at '/path/to/project/crux.toml'; existing crux.toml was preserved
```

This remains true with `--force`.

Import never executes project build scripts and never performs network access.

---

## 8. Complete Example

Given an existing project:

```text
legacy-app/
├── CMakeLists.txt
├── include/
│   └── greeter.hpp
└── src/
    └── main.cpp
```

Run the import:

```bash
cd legacy-app
crux import --dry-run
crux import
```

Review the generated files:

```bash
cat crux.toml
cat .crux/import.toml
```

Validate and build:

```bash
crux check
crux build
crux run
```

The original `CMakeLists.txt`, `include/`, and `src/` contents remain untouched. Crux writes its generated CMake project only under `.crux/build/generated/` and places build artifacts under `.crux/build/debug/` or `.crux/build/release/`.

---

## 9. Manual Steps After Import

Review the generated manifest and import report when:

- The project uses Makefiles with generated sources or custom flags.
- The existing CMake configuration defines compile definitions, linked libraries, or generated headers required by the sources.
- Multiple `main()` entry points were accepted with `--force`.
- No conventional `main()` was found and `--force` was used.
- The project contains conditional compilation controlled by environment variables or build scripts.
- Dependencies were previously vendored and need to be declared with `crux add`.

For full CMake-specific build semantics, continue using the project's own CMake workflow. Crux import and `crux build` currently provide a source-based compatibility path, not a translation of arbitrary existing build graphs.
