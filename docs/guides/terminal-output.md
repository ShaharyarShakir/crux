# Terminal Output, Diagnostics & Verbosity

Crux reports progress and problems in a consistent, Cargo-inspired style. This guide explains what Crux prints, where it prints it, and how to control color and detail.

---

## 1. Where output goes

| Stream | Contents |
|---|---|
| **stdout** | Command results: resolution and lockfile summaries, fetched source paths, build summaries, and your application's own output for `crux run`. |
| **stderr** | Status lines (`Compiling`, `Downloaded`, `Finished`, …), progress, warnings, errors, and the original output of failed tools. |

Because status and diagnostics never go to stdout, `crux resolve > deps.txt` or `crux fetch | tee fetch.log` capture only results. When stdout is a closed pipe (for example `crux resolve | head -1`), Crux stops writing quietly; the closed pipe does not change the exit status.

Output is plain ASCII, so it needs no Unicode-capable terminal or font.

---

## 2. Status lines

Routine progress is written as right-aligned status lines:

```text
   Compiling fmt v11.1.1
   Compiling zlib v1.3.1
  Downloaded spdlog v1.13.0 (412.0 KiB, checksum verified)
     Fetched util v1.2.3 (git commit 0123456789ab)
  Installing fmt v11.1.1
   Compiling my-app v0.1.0
    Finished `debug` profile in 8.42s
     Running /path/to/.crux/build/debug/my-app
```

| Status | Meaning |
|---|---|
| `Compiling` | The package is about to be configured and built (it was not up to date). |
| `Fresh` (`-v`) | A previous successful build with the same fingerprint was reused. |
| `Downloaded` | An archive was downloaded and its SHA-256 matched the expected checksum. |
| `Fetched` | A Git source was cloned and pinned to the shown commit. |
| `Cached` (`-v`) | A verified archive, extracted source tree, or Git snapshot was reused from the cache. |
| `Extracted`, `Verified`, `Source`, `Snapshot`, `Built` (`-v`) | Extra detail about acquisition and builds. |
| `Installing` | A built package is being installed into the project. |
| `Finished` | The whole build finished; the profile and wall-clock time are shown. |

### Download progress

On an interactive terminal, a download shows a single in-place progress line, which is cleared when the download completes:

```text
 Downloading fmt v11.1.1 [==================>                     ]  45%  1.2 MiB/2.7 MiB
```

- The bar and percentage are shown **only** when the server declares the size (`Content-Length`). Otherwise only the received byte count is shown; Crux never invents a percentage.
- When stderr is not a terminal (piped output, CI logs), no progress line is drawn at all: each download produces one final `Downloaded` line, and the output contains no carriage returns or cursor control sequences.
- Progress display never affects the download itself: the inactivity timeout, size limit, SHA-256 verification, archive extraction safety, and cache integrity checks are unchanged.

---

## 3. Verbosity

Global verbosity options work with every command, either before or after the command name (but before a `--` separator):

| Mode | Behavior |
|---|---|
| default | Status lines, warnings, and errors. |
| `-v`, `--verbose` | Adds source details (redacted URLs, sizes, checksums, cache hits), `Fresh` packages, build directories, and the complete original output of a failed tool. For `crux build`/`crux run`, also the long-standing verbose compiler and CMake detail. |
| `-vv` | Also streams CMake and compiler output live while it runs. |
| `-q`, `--quiet` | No routine status lines or progress. **Errors and warnings are always shown**, including the full failure diagnostic and the original tool output. |

`-q` cannot be combined with `-v`. Command results on stdout are not affected by verbosity.

URLs in verbose output are always passed through credential redaction (`https://***@host/…`), so tokens are never printed.

```bash
crux -v fetch                 # show sources, sizes, checksums, and cache hits
crux build -vv                # stream CMake and compiler output live
crux --quiet install          # CI-friendly: only warnings and errors
crux run -- -v                # `-v` after `--` is passed to the application
```

---

## 4. Color

| Option / variable | Effect |
|---|---|
| `--color auto` (default) | Color only when stderr is an interactive terminal and `TERM` is not `dumb`. |
| `--color always` | Always emit ANSI color (for example, for CI systems that render it). |
| `--color never` | Never emit color. |
| `CLICOLOR_FORCE=<non-empty, not 0>` | In `auto` mode, enable color even when not a terminal. |
| `NO_COLOR=<non-empty>` | In `auto` mode, disable color. |

**Precedence (highest first):**

1. `--color always` / `--color never`
2. `CLICOLOR_FORCE`
3. `NO_COLOR`
4. terminal detection

An explicit command-line choice always wins, so `crux --color always build` colors output even with `NO_COLOR` set.

Color only adds escape sequences; the text is identical with and without color, and styling never changes a command's exit status. Color applies to diagnostics and status lines on stderr. Command results on stdout and the original output of external tools are never recolored.

---

## 5. Diagnostics

Errors and warnings use one structured layout:

```text
error: package compilation failed

  package:     fmt v11.1.1
  stage:       compilation and linking (cmake --build)
  exit status: 2
  build dir:   /home/me/.cache/crux/builds/fmt/11.1.1/3f9c…/build
  log:         /home/me/.cache/crux/builds/fmt/11.1.1/3f9c…/logs/build.log

  The build tool exited with status 2.

  command:
    /usr/bin/cmake --build /home/me/.cache/crux/builds/fmt/11.1.1/3f9c…/build --config Debug

  note: the original CMake and compiler output follows, unmodified
  help: inspect the build directory or the complete log at …/logs/build.log
  help: re-run with -vv to stream CMake and compiler output live

--- original output (…/logs/build.log) ---
$ /usr/bin/cmake --build … --config Debug

[1/2] Building CXX object CMakeFiles/fmt.dir/src/format.cc.o
FAILED: CMakeFiles/fmt.dir/src/format.cc.o
src/format.cc:12:3: error: 'foo' was not declared in this scope
   12 |   foo();
      |   ^~~
ninja: build stopped: subcommand failed.

=== exit status: 1 ===
--- end of original output ---
```

- **Facts only.** Crux reports what it knows: the package, the stage (`CMake configuration`, or `compilation and linking (cmake --build)`), the exit status, the exact command, and where to look. It does not parse or reinterpret compiler messages, and never guesses causes.
- **Original output is preserved.** The compiler's own diagnostics (file, line, column, caret lines) are printed byte for byte between the `--- original output ---` markers.
  - Very long output is shortened in the middle by default (the first 60 and last 140 lines are kept, so the first error stays visible). `-v` prints all of it.
  - The complete log, in arrival order, is always kept on disk.
- **Bounded memory.** Build output is streamed to the log file as it is produced; Crux keeps only a bounded tail in memory, however much a build prints.

The categories are `error`, `warning`, `note`, `info`, and `success`. `note:` lines add context, and `help:` lines suggest a next step.

---

## 6. Exit status

Terminal presentation never changes exit codes: a successful command exits `0`, a failed one exits non-zero (for `crux run`, the application's own exit code is propagated), regardless of color, verbosity, or quiet mode.
