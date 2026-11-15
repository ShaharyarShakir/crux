# Crux Quickstart Guide

Get up and running with **Crux** and CMake in under 5 minutes.

---

## 1. Prerequisites

Ensure you have installed Crux and required tools as described in [docs/INSTALLATION.md](file:///home/shaharyar/01__git_repos/crux/docs/INSTALLATION.md):
- `crux` (v0.1.0)
- `cmake` (3.20+)
- C++ compiler (`g++` or `clang++` supporting C++17)
- `git`

---

## 2. Walkthrough: Building a C++ App with Crux & fmt

### Step 1: Create a Project Workspace
Create a new directory for your project and initialize it with Crux:

```bash
mkdir hello_crux && cd hello_crux
crux init --name hello_crux
```

This creates a default `crux.toml` manifest:
```toml
[package]
name = "hello_crux"
version = "0.1.0"
description = "A C/C++ package managed by Crux"
```

---

### Step 2: Configure Package Registry
If not already configured globally, add the Crux package registry:

```bash
crux registry add default https://github.com/ShaharyarShakir/crux-registry.git
```
*(For local development or testing, you can also specify a local directory path).*

---

### Step 3: Declare Dependencies
Edit `crux.toml` to declare your package dependencies. For example, add `fmt`:

```toml
[package]
name = "hello_crux"
version = "0.1.0"

[dependencies]
fmt = "^11.1"
```

Verify your manifest syntax and constraints:
```bash
crux check
```

---

### Step 4: Resolve and Lock Dependencies
Preview resolution and generate a reproducible `crux.lock`:

```bash
# Preview the resolved graph
crux resolve

# Generate or update crux.lock
crux lock
```

`crux.lock` records exact versions and SHA-256 digests:
```toml
lockfile_version = 1

[[package]]
name = "fmt"
version = "11.1.1"
source = "registry+default"
checksum = "sha256:a25124e41c15c290b214c4dec588385153c91b47198dbacda6babce27edc4b45"
```

---

### Step 5: Acquire, Build, and Install
Fetch, compile, and install the package locally into `.crux/installed/`:

```bash
# Fetch and verify archive integrity
crux fetch

# Compile dependencies via CMake out-of-source build engine
crux build

# Install staged artifacts into project-local storage
crux install
```

*(Tip: `crux install` automatically fetches and builds any dependencies that have not yet been compiled).*

---

### Step 6: Generate CMake Integration
Generate the relocatable CMake integration file:

```bash
crux integrate cmake
```

This generates `.crux/cmake/CruxDependencies.cmake`, which configures CMake `CMAKE_PREFIX_PATH` and package discovery roots.

---

### Step 7: Create Your C++ Consumer Application

Create `CMakeLists.txt`:
```cmake
cmake_minimum_required(VERSION 3.20)
project(hello_crux CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Include the Crux generated integration file
include("${CMAKE_CURRENT_SOURCE_DIR}/.crux/cmake/CruxDependencies.cmake")

# Find the installed package using standard CMake config mode
find_package(fmt CONFIG REQUIRED)

add_executable(hello_crux main.cpp)
target_link_libraries(hello_crux PRIVATE fmt::fmt)
```

Create `main.cpp`:
```cpp
#include <fmt/core.h>
#include <fmt/color.h>

int main() {
    fmt::print(fg(fmt::color::cyan) | fmt::emphasis::bold, "Hello from Crux!\n");
    fmt::print("fmt version {}.{}.{} is working seamlessly with CMake.\n",
               FMT_VERSION / 10000, (FMT_VERSION % 10000) / 100, FMT_VERSION % 100);
    return 0;
}
```

---

### Step 8: Build and Run

```bash
# Configure build directory
cmake -B build -S .

# Compile your application
cmake --build build

# Run the executable!
./build/hello_crux
```

Expected Output:
```text
Hello from Crux!
fmt version 11.1.1 is working seamlessly with CMake.
```

---

## 3. Next Steps

- Consult [docs/CLI_REFERENCE.md](file:///home/shaharyar/01__git_repos/crux/docs/CLI_REFERENCE.md) for detailed descriptions of all flags and subcommands.
- Review [docs/SECURITY.md](file:///home/shaharyar/01__git_repos/crux/docs/SECURITY.md) to understand Crux's process privilege model and defense-in-depth safety layers.
