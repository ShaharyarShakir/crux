# Crux Quickstart Guide

Get up and running with **Crux** in under 5 minutes. Crux builds ordinary C and C++ applications without requiring you to write or maintain CMake.

---

## 1. Prerequisites

Ensure you have installed Crux and the required tools as described in [installation.md](installation.md):

- `crux`
- `cmake` (3.20+)
- A C and/or C++ compiler (`gcc`/`clang`, `g++`/`clang++`)
- `git` (only when using registry packages)

Crux detects the compiler automatically, or you can select one explicitly with the `CC` and `CXX` environment variables.

---

## 2. Walkthrough: Build a C++ Application

### Step 1: Create a Project

```bash
crux init hello-crux --language cpp
cd hello-crux
```

This creates a standardized Crux project:

```text
hello-crux/
├── crux.toml
├── crux.lock
└── src/
    └── main.cpp
```

Generated `crux.toml`:

```toml
[project]
name = "hello-crux"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
```

For a C project, use:

```bash
crux init hello-crux-c --language c
```

C projects default to `c17` and generate `src/main.c`.

---

### Step 2: Add or Edit Application Sources

Only source files under `src/` are application inputs. Crux discovers them recursively:

- **C**: `.c`
- **C++**: `.cpp`, `.cc`, `.cxx`

Headers are not compiled independently, and C and C++ sources must not be mixed in one project. For example:

```text
hello-crux/
└── src/
    ├── main.cpp
    └── greeting/
        └── greeting.cpp
```

Crux includes nested source directories automatically and sorts discovered files deterministically.

---

### Step 3: Build

```bash
crux build
```

Crux loads `crux.toml`, detects the compiler, discovers sources, generates internal CMake configuration, and compiles the application.

Example output:

```text
Crux Application Build

Project:       hello-crux
Language:      C++
Standard:      C++20
Configuration: Debug

Toolchain:
  Compiler:    GCC 15.3.0
  CMake:       CMake 4.1.6
  Generator:   Ninja

Source files:  2

Generating build configuration...
Configuring...
Building...

Build successful!

Executable:
  .crux/build/debug/hello-crux
```

Build a Release executable with:

```bash
crux build --release
```

Debug and Release artifacts are isolated under `.crux/build/debug/` and `.crux/build/release/`.

---

### Step 4: Run the Application

```bash
crux run
```

`crux run` builds the application (Debug by default) and executes it from the project root:

```text
Crux Run

Project:       hello-crux
Configuration: Debug

Checking application...
Building...
Build successful.

Running hello-crux...

Hello from hello-crux!
```

Run the Release configuration or pass arguments to the application:

```bash
crux run --release
crux run -- --name Alice --count 5
```

You can also run the built executable directly if needed:

```bash
./.crux/build/debug/hello-crux
./.crux/build/release/hello-crux
```

#### Run a single file without a project

`crux run` also accepts ad-hoc C or C++ sources, so a scratch program does not need a `crux.toml` manifest:

```bash
crux run --bin hello.c
crux run --bin hello.cpp --release
crux run --bin hello.c -- --greeting "Good morning"
```

For multiple translation units, use repeatable or comma-separated `--sources` and add include directories with `-I` / `--include-dir`:

```bash
crux run --sources main.cpp,math.cpp --sources util.cpp -Iinclude
crux run --sources main.c --sources util.c --include-dir ../shared
```

Ad-hoc sources are copied into an isolated `.crux/run/<name>-<hash>/` workspace and compiled by the same CMake engine as project applications. No registry or package resolution is involved.

---

### Step 5: Add and Use Dependencies (`crux add`)

Crux manages dependencies through the official registry automatically without manual CMake configuration. The M22 catalog contains 50 curated packages; 15 are fully verified on Linux x86_64 and marked **Verified** in the [official catalog](official-package-catalog.md). Catalog-only entries are discoverable but are rejected by build/install commands until a matching build backend exists.

```bash
crux add fmt
```

You can also supply explicit SemVer constraints:

```bash
crux add fmt@^11.0
crux add zlib@">=1.2"
```

What `crux add` does:
1. Locates the package in the official or configured registry (the official registry is auto-configured on first run).
2. Deterministically resolves the full dependency graph and preserves existing locked versions.
3. Updates `crux.toml` and records the exact graph in `crux.lock`.
4. Automatically downloads, builds, and installs the dependency for your project.

Now update `src/main.cpp` to use the library:

```cpp
#include <fmt/core.h>

int main() {
    fmt::print("Hello from Crux with fmt!\n");
    return 0;
}
```

Build and run your application:

```bash
crux run
```

Crux detects the dependency, links the required static or shared libraries, sets up include paths and runtime RPATHs, and executes:

```text
Hello from Crux with fmt!
```

Crux also supports building and running the Release configuration seamlessly:

```bash
crux run --release
```

If the Release build of a dependency package is not yet installed, Crux automatically compiles and installs it before building the application.

---

### Step 6: Remove Dependencies (`crux remove`)

When you no longer need a direct dependency:

```bash
crux remove fmt
```

`crux remove` removes the package from `crux.toml`, re-resolves the remaining dependency graph, and updates `crux.lock` while retaining any dependencies still required transitively.

---

### Step 7: Rebuild Incrementally

Edit a source file and run `crux build` or `crux run` again. CMake rebuilds only what changed, while newly added source files are discovered automatically and removed files are no longer compiled.

Useful options:

```bash
# Show compiler/CMake paths and full build output
crux build --verbose
crux run --verbose

# Force a clean rebuild of the selected configuration
crux build --force
```

---

## 3. Generated Build Layout

Crux manages CMake internally. You do **not** need a root-level `CMakeLists.txt`.

```text
hello-crux/
├── crux.toml
├── crux.lock
├── src/
└── .crux/
    └── build/
        ├── generated/
        │   └── CMakeLists.txt   # generated by Crux; do not edit
        ├── debug/
        │   └── hello-crux
        ├── release/
        │   └── hello-crux
        └── logs/
            ├── configure-debug.log
            └── build-debug.log
```

All build artifacts stay inside `.crux/build/`. Source files are never modified. Add `.crux/` to your version-control ignore rules; Crux does not modify `.gitignore` automatically.

---

## 4. Common Diagnostics

- **Language `other`**: Projects with `language = "other"` cannot be compiled with `crux build`. Choose `c` or `cpp`.
- **No sources found**: Add at least one supported source file under `src/`.
- **Mixed sources**: A C project cannot contain C++ sources, and a C++ project cannot contain C sources.
- **Missing compiler**: Install a C/C++ compiler or select one with `CC`/`CXX`.
- **Missing CMake**: Install CMake 3.20 or newer and ensure `cmake` is on `PATH`.
- **Compiler selection changes**: Changing `CC`/`CXX` or the language standard triggers reconfiguration; Crux discards stale CMake caches rather than mixing toolchains.

---

## 5. Next Steps

- Read the full command and environment reference in [cli-reference.md](../reference/cli-reference.md).
- Review the architecture in [architecture.md](../reference/architecture.md).
- Understand Crux's process and integrity model in [security-model.md](../reference/security-model.md).
