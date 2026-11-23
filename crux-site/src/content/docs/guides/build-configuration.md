---
title: Build Configuration
description: Compiler flags, linker flags, and AddressSanitizer / UndefinedBehaviorSanitizer for your application, with cache-correct fingerprints.
---

The optional `[build]` table in `crux.toml` customizes how Crux compiles **your application**. It supports extra C and C++ compiler flags, extra linker flags, and AddressSanitizer / UndefinedBehaviorSanitizer instrumentation.

---

## 1. Complete example

```toml
[project]
name = "my-app"
version = "0.1.0"
language = "cpp"
standard = "c++20"

[dependencies]
fmt = "^11.1"

[build]
c_flags = ["-Wall", "-Wextra", "-Wpedantic"]
cxx_flags = ["-Wall", "-Wextra", "-Wpedantic"]
linker_flags = ["-Wl,--as-needed"]

[build.sanitizers]
address = true
undefined = true
```

A runnable copy lives in `examples/build-settings` in the repository.

| Key | Type | Meaning |
|---|---|---|
| `c_flags` | array of strings | Extra compiler arguments for C projects (`language = "c"`). |
| `cxx_flags` | array of strings | Extra compiler arguments for C++ projects (`language = "cpp"`). |
| `linker_flags` | array of strings | Extra linker arguments (passed through the compiler driver). |
| `sanitizers.address` | bool | Build and link with `-fsanitize=address` (plus `-fno-omit-frame-pointer`). |
| `sanitizers.undefined` | bool | Build and link with `-fsanitize=undefined`. |

All keys are optional. Without `[build]`, builds behave exactly as before.

---

## 2. Scope: application only, never dependencies

`[build]` applies **only to the application target** that `crux build` and `crux run` compile from your `src/`.

Dependencies are built separately from their own recipes and are fingerprinted independently. Project flags are **not** passed to them, because a flag such as `-Wpedantic -Werror` that is right for your code can break a third-party package, and changing your flags must not invalidate or rebuild installed dependencies.

Consequences you should know:

- With `address = true`, only your code is instrumented. Memory errors *inside* a dependency's own code are not reported, although linking with dependencies built without ASan works.
- `[build]` is accepted only in `[project]` manifests. A legacy `[package]` manifest with `[build]` is rejected rather than silently ignored.
- `c_flags` is used only by C projects and `cxx_flags` only by C++ projects. The other list is ignored.

---

## 3. How flags are applied

- Each array entry is **exactly one argument**. `["-I", "include"]` and `["-Iinclude"]` are fine; `["-I include"]` is a single argument containing a space and is rejected if it has leading or trailing whitespace. An entry with an internal space (for example `-DGREETING=hello world`) stays one argument.
- Entries must be non-empty and contain no control characters.
- Crux never builds a shell command. Flags are written into the generated CMake project as individually quoted `target_compile_options` / `target_link_options` arguments, with `\`, `"`, `$`, `;` and `@` escaped, so `$(...)`, backticks and `${VAR}` are passed to the compiler literally.
- Compiler defaults are preserved. User flags come **after** Crux's own (`-std=...`, the profile's `-g`/`-O` flags), so a later flag such as `-O1` overrides an earlier default, following normal compiler semantics.
- Sanitizer flags come before your flags, so you can refine them (for example `-fsanitize-recover=address`).

**CMake limitation:** CMake removes duplicate options from `target_compile_options`. A repeated pair such as `-include a.h -include b.h` can be collapsed. Prefer single-argument forms (`-include=...` where the compiler supports it) or a header that includes both.

---

## 4. Sanitizers

```toml
[build.sanitizers]
address = true
undefined = true
```

Both are **off by default** and are never enabled automatically, including in release builds.

Crux applies the same `-fsanitize=...` flags to compilation **and** linking and, before building, verifies that the detected compiler can compile and link a trivial program with them. If it cannot, the build stops:

```text
error: requested sanitizer is not supported

  package:    my-app v0.1.0
  stage:      sanitizer check
  compiler:   GCC 13.2.0
  sanitizers: AddressSanitizer

  the compiler failed to compile and link a trivial program with these flags.

  command:
    /usr/bin/gcc -fsanitize=address -fno-omit-frame-pointer /tmp/.../probe.c -o /tmp/.../probe_bin

  note: the compiler reported:
  note: <compiler output, unmodified>
  note: Crux does not disable requested sanitizers silently
  help: install the compiler's sanitizer runtime libraries, use a different compiler, or remove the entry from [build.sanitizers] in crux.toml
```

### Requirements and limitations

- **Compilers:** GCC, Clang and Apple Clang (any compiler whose family Crux detects as one of these). MSVC is rejected with a clear diagnostic. Support depends on the installed sanitizer runtime libraries (`libasan`, `libubsan`, or Clang's compiler-rt).
- **Probe scope:** the probe compiles and links but does not run the result, so it cannot detect an environment that blocks running ASan binaries (restricted containers, some `ptrace`/ASLR settings).
- **Runtime:** ASan needs extra memory and slows execution; run instrumented binaries with `ASAN_OPTIONS`/`UBSAN_OPTIONS` as documented by your compiler. UBSan prints diagnostics but, by default, does not abort.
- **Combinations:** only `address` plus `undefined` are offered. Other sanitizers (thread, memory, leak-only) are not supported by Crux. Crux does not check combinations beyond what the probe proves.
- **Debug vs. release:** sanitizers work with either profile; for readable reports use the debug profile.
- **Tested:** GCC on Linux. Clang and macOS are expected to work but are not covered by the automated suite.

---

## 5. Cache correctness and fingerprints

Crux records a fingerprint in `.crux/build/<profile>/crux-build.fingerprint`. It covers:

- compiler family, exact version, target triple and path;
- language, standard and profile;
- CMake version and generator;
- `c_flags`, `cxx_flags` and `linker_flags` (**order-sensitive**: later flags override earlier ones);
- sanitizer settings;
- the fingerprints of linked dependency installations.

When the fingerprint differs from the recorded one, the profile build tree is discarded and rebuilt, so objects compiled under a different configuration are never reused. A build tree without a record (created by an older Crux) is rebuilt once. Changing nothing reuses the tree and keeps incremental builds.

The `crux.lock` format is unchanged. Dependency installation fingerprints are unchanged by `[build]`, because `[build]` does not apply to dependencies.

---

## 6. Seeing what is applied

`crux build` prints a `Build settings (application only)` block when anything is configured. With `-v` it lists the actual flags; without it, only counts. With `-vv` the exact CMake and compiler commands are streamed live.
