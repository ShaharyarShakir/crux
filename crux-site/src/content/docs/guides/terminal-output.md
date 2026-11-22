---
title: Terminal Output & Diagnostics
description: Status lines, download progress, structured build diagnostics, and the color and verbosity options.
---

Crux prints Cargo-inspired status lines and structured diagnostics. Command results go to **stdout**; status lines, progress, warnings, and errors go to **stderr**, so piped results stay clean. Output is plain ASCII.

## Status lines

```text
   Compiling fmt v11.1.1
  Downloaded spdlog v1.13.0 (412.0 KiB, checksum verified)
     Fetched util v1.2.3 (git commit 0123456789ab)
   Compiling my-app v0.1.0
    Finished `debug` profile in 8.42s
```

On an interactive terminal, downloads show one in-place progress line. The bar and percentage appear only when the server declares the size; otherwise only the byte count is shown. Piped output and CI logs never contain progress redraws.

## Verbosity

| Mode | Behavior |
|---|---|
| default | Status lines, warnings, errors |
| `-v` | Sources (credential-redacted), sizes, checksums, cache hits, `Fresh` packages, full tool output on failure |
| `-vv` | Also streams CMake and compiler output live |
| `-q`, `--quiet` | No routine progress; errors and warnings are always shown |

These are global options: `crux -v fetch`, `crux build -vv`, `crux --quiet install`. Arguments after `--` are passed to your application unchanged.

## Color

`--color auto|always|never` (default `auto`: color only when stderr is a terminal and `TERM` is not `dumb`).

Precedence, highest first:

1. `--color always` / `--color never`
2. `CLICOLOR_FORCE` (non-empty, not `0`)
3. `NO_COLOR` (non-empty)
4. terminal detection

Color never changes the text or the exit status.

## Build failures

```text
error: package compilation failed

  package:     fmt v11.1.1
  stage:       compilation and linking (cmake --build)
  exit status: 2
  build dir:   …/builds/fmt/11.1.1/<fingerprint>/build
  log:         …/logs/build.log

  The build tool exited with status 2.

  command:
    cmake --build …/build --config Debug

  note: the original CMake and compiler output follows, unmodified
  help: re-run with -vv to stream CMake and compiler output live

--- original output (…/logs/build.log) ---
src/format.cc:12:3: error: 'foo' was not declared in this scope
   12 |   foo();
      |   ^~~
--- end of original output ---
```

Crux reports only facts it knows: the package, stage, command, and exit status. It never reinterprets compiler messages. The compiler's own output, including source locations and caret lines, is printed verbatim. Long output is shortened in the middle unless you pass `-v`, and the complete log always stays on disk.

See the repository guide [Terminal Output, Diagnostics & Verbosity](https://github.com/ShaharyarShakir/crux/blob/main/docs/guides/terminal-output.md) for details.
