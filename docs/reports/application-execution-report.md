# M20 — Application Execution (`crux run`) Completion Report

## 1. Implementation summary
M20 introduces `crux run` to build and execute the current Crux C/C++ application project in a single command. The implementation directly reuses the existing M19 application build engine (planning, generation, configuration, compilation), validates and locates the resulting executable within the project-local `.crux/build/` directory, launches it from the project root with inherited standard streams (`stdin`, `stdout`, `stderr`), inherited environment, and forwarded command-line arguments, and propagates the application's exit status to the calling shell. Existing package build, legacy dependency build, and M19 application build behaviors remain completely intact.

## 2. Final execution architecture
- **CLI parsing & dispatch (`src/cli/args.rs`, `src/cli/mod.rs`)**:
  - Implements `Action::Run(RunArgs)` and `Action::RunHelp` (`RUN_HELP_MESSAGE`).
  - Supports `--debug`, `--release`, `-v` / `--verbose`, and trailing application arguments after `--`.
  - Dispatches to `crux::cli::commands::run::run_run`.
  - Top-level `HELP_MESSAGE` updated to include `run`.
- **Pre-execution validation & run command (`src/cli/commands/run.rs`)**:
  - Orchestrates the full execution lifecycle:
    1. Validates project discovery and manifest existence (`crux.toml`).
    2. Validates project configuration (`[project]` manifest required).
    3. Validates supported application language (C and C++ supported; `other` rejected).
    4. Prepares M19 application build model (`AppBuildEngine::prepare`).
    5. Executes build through M19 (`AppBuildEngine::execute`).
    6. Validates executable path, regular file status, Unix execute permissions, and confinement within `.crux/build/`.
    7. Flushes stdout/stderr buffers.
    8. Launches process and returns/propagates exit status.
- **Build integration (`src/build/app.rs`, M19)**:
  - Reused unchanged as the build engine (`AppBuildModel`, `AppBuildLayout`, `AppBuildResult`).
  - `crux run` does not duplicate build planning, toolchain detection, CMake generation, or compilation logic.
- **Process & exit propagation (`src/main.rs`, `src/cli/mod.rs`)**:
  - `CliError::AppExit(i32)` allows `main` to terminate with the child application's exact exit status code without emitting misleading Crux diagnostic messages for normal nonzero application exits.

## 3. CLI syntax and argument forwarding
Supported command syntax:
- `crux run` (builds and runs Debug configuration by default)
- `crux run --debug` (explicit Debug profile)
- `crux run --release` (explicit Release profile)
- `crux run --verbose` (verbose CMake and compiler output)
- `crux run [options] -- <app args...>` (argument forwarding)

Behavior:
- Crux options are parsed before `--`; the `--` delimiter terminates Crux option parsing.
- Arguments after `--` are forwarded unchanged in order as discrete arguments to the application.
- Spaces, special characters, and leading dashes in application arguments (e.g. `--name "Alice Smith" --count 5 -v --flag`) are preserved exactly.
- Empty forwarded argument lists (`crux run --`) are supported.
- Option conflicts between `--debug` and `--release` are detected and rejected with a clear diagnostic.
- Positional/unrecognized Crux options before `--` are rejected as unexpected arguments.
- No shell command string is constructed, avoiding any shell injection or argument corruption.

## 4. M19 build-engine integration
- `crux run` constructs `AppBuildRequest::new(project_dir).with_profile(profile).with_force(false).with_verbose(args.verbose)` and invokes `AppBuildEngine::prepare` and `AppBuildEngine::execute`.
- Default configuration is Debug; `--release` selects the Release profile and layout.
- M18/M19 toolchain detection (`ProjectToolchain`), compiler selection (`CC`/`CXX`), manifest validation, source discovery under `src/`, internal CMake generation under `.crux/build/generated/`, and out-of-source builds under `.crux/build/debug/` or `.crux/build/release/` are preserved.
- Incremental compilation is managed natively by CMake/Ninja/Make; no redundant freshness or timestamp cache is introduced.
- Build is invoked exactly once per run command.
- If compilation fails, `crux run` reports the failure immediately and halts; it never attempts to run an outdated or stale binary.

## 5. Executable resolution and validation
- The executable path is obtained directly from `AppBuildResult::executable_path`.
- Before executing the binary, `validate_executable`:
  - Confirms the executable path exists.
  - Confirms it is a regular file.
  - Resolves both project root and executable via `canonicalize()`, verifying that the executable is confined strictly inside `.crux/build/`.
  - On Unix, inspects file metadata to verify executable permission bits (`mode & 0o111 != 0`).
  - Rejects missing, invalid, or unconfined binaries with clear, actionable diagnostics without silent fallbacks.
- Handles platform-specific executable naming (e.g. `.exe` on Windows) through M19 target metadata and path-safe abstractions.

## 6. Process execution behavior
- Implemented with Rust standard library `std::process::Command` without shell wrappers (`sh -c` / `bash -c` are never used).
- Target binary: `Command::new(executable_path)`.
- Arguments: `command.args(app_args)`.
- Standard streams:
  - `command.stdin(Stdio::inherit())`
  - `command.stdout(Stdio::inherit())`
  - `command.stderr(Stdio::inherit())`
  - Interactive terminal input/output and unbuffered streaming operate natively without buffering output in memory.
- Process execution is waited with `.status()`; process spawn failures are converted to structured Crux execution errors with underlying OS diagnostic details.

## 7. Environment and working-directory behavior
- **Working directory**: Explicitly set to the Crux project root (`command.current_dir(project_root)`).
  - Relative file paths read or written by the application resolve predictably within the user's project workspace rather than inside `.crux/build/`.
  - The calling user's shell working directory is not mutated.
- **Environment**: Inherited from the parent process by default.
  - No environment clearing or replacement occurs.
  - Standard system variables (`PATH`, `HOME`, `TERM`, etc.) and user variables are preserved.

## 8. Exit status and signal handling
- Normal exit with code 0: Crux exits with status 0.
- Normal exit with nonzero code: Crux propagates the child process's exact exit code via `CliError::AppExit(code)` / `std::process::exit(code)`.
- Signal termination on Unix: When `status.code()` is None and `status.signal()` is present, mapped to `128 + signal` (following POSIX shell standard conventions).
- Launch or validation failures: Reported as Crux errors, exiting with standard CLI failure code (1).
- No misleading success messages are printed when an application terminates with a nonzero status.

## 9. Files created and modified
Created:
- `src/cli/commands/run.rs` (application execution logic, executable validation, unit tests)
- `tests/run.rs` (comprehensive end-to-end integration tests for `crux run`)
- `docs/reports/application-execution-report.md` (this report)

Modified:
- `src/cli/args.rs` (added `run` command to `HELP_MESSAGE`, `RunArgs`, `Action::Run`, `Action::RunHelp`, `parse_run_args`, option conflict detection)
- `src/cli/commands/mod.rs` (exported `run::run_run`)
- `src/cli/mod.rs` (added `Action::Run` execution, `CliError::AppExit(i32)`, unit tests for CLI parsing and option conflict detection)
- `src/main.rs` (propagates `CliError::AppExit(code)` directly to `std::process::exit(code)`)
- `docs/reference/architecture.md` (documented `crux run` in system overview, CLI subsystem, and dedicated Application Execution Engine section)
- `docs/reference/cli-reference.md` (documented `crux run` options, behavior, argument forwarding, and relationship to `crux build`)
- `docs/guides/quickstart.md` (updated getting started workflow to use `crux run`)
- `README.md` (added `crux run` command documentation and workflow examples)
- `crux-site/src/content/docs/guides/getting-started.md` (updated quickstart guide for `crux run`)
- `crux-site/src/content/docs/reference/cli.md` (added reference documentation for `crux run`)

## 10. Tests added
### Unit tests (`src/cli/commands/run.rs`)
- `test_validate_executable_nonexistent`: Rejects missing binary paths.
- `test_validate_executable_directory_not_file`: Rejects directory paths masquerading as executables.
- `test_validate_executable_outside_build_root`: Rejects binaries located outside `.crux/build/`.
- `test_validate_executable_valid`: Accepts valid executables inside `.crux/build/`.
- `test_validate_executable_non_executable_permissions`: Rejects files without Unix execute bits.

### CLI parsing unit tests (`src/cli/mod.rs`)
- `test_parse_run_basic_and_profiles`: Tests `crux run`, `crux run --debug`, and `crux run --release --verbose`.
- `test_parse_run_args_forwarding`: Tests forwarding multiple arguments, spaces, and leading dashes, plus empty argument lists (`--`).
- `test_parse_run_option_conflicts`: Confirms `--debug` and `--release` conflict detection.
- `test_parse_run_help_and_errors`: Tests help flags and rejection of unexpected options / positional arguments.

### Integration tests (`tests/run.rs`)
- `run_cpp_application_debug_and_release`: Validates C++ application build and execution in Debug and Release profiles, verifying isolated artifacts.
- `run_c_application_with_args_and_spaces`: Validates C application argument forwarding with spaces and leading dashes (`--name "Alice Smith" --count 5 -v --flag`).
- `run_propagates_nonzero_exit_status`: Validates exact exit code propagation (returns 7).
- `run_executes_from_project_root_and_preserves_stdin`: Validates child execution in the project root (creating relative files in project root, not `.crux/build/`) and stdin stream inheritance.
- `run_inherits_environment`: Validates environment variable inheritance across the child process boundary.
- `run_stdout_and_stderr_forwarding`: Validates distinct stdout and stderr forwarding to parent streams.
- `run_incremental_rebuild_behavior`: Validates that repeated `crux run` calls succeed incrementally without rebuild errors.
- `run_paths_with_spaces_and_nested_sources`: Validates execution of projects inside paths with spaces and nested source files (`src/modules/helper.cpp`).
- `run_rejects_other_language_project`: Validates rejection of `language = "other"` with an actionable diagnostic.
- `run_missing_manifest_fails_with_clear_error`: Validates that running in an uninitialized directory reports a clear missing manifest diagnostic.
- `build_failure_prevents_execution`: Validates that compilation errors stop execution immediately and do not execute an existing stale binary.
- `existing_package_build_commands_still_work`: Confirms `crux build` functionality remains intact.

## 11. Actual quality gate results
All quality gates executed within the Nix flake shell (`nix develop`):

| Quality Gate | Command | Result |
|---|---|---|
| Formatting | `nix develop --command cargo fmt --all -- --check` | **PASS** (zero diffs) |
| Linter | `nix develop --command cargo clippy --all-targets --all-features -- -D warnings` | **PASS** (zero warnings) |
| Test Suite | `nix develop --command cargo test --all` | **PASS** (**442 passed**, 0 failed, 0 ignored: 183 unit + 259 integration across 15 suites, 0 doc-tests) |
| Compilation Check | `nix develop --command cargo check` | **PASS** (zero errors) |
| Release Build | `nix develop --command cargo build --release` | **PASS** (produced `target/release/crux`) |

## 12. Manual end-to-end results
Executed using release binary `target/release/crux` in a clean temporary directory (`/tmp/crux_m20_manual_test`):

1. **Project initialization**:
   - `crux init demo-c --language c` -> Initialized C project.
   - `crux init demo-cpp --language cpp` -> Initialized C++ project.
2. **Build and execution for C**:
   - `crux build` -> Compiled Debug binary to `.crux/build/debug/demo-c`.
   - `crux run` -> Built and executed Debug binary, outputting `"Hello from demo-c!"`.
   - `crux run --release` -> Built and executed Release binary, outputting `"Hello from demo-c!"`.
3. **Build and execution for C++**:
   - `crux build` -> Compiled Debug binary to `.crux/build/debug/demo-cpp`.
   - `crux run` -> Built and executed Debug binary, outputting `"Hello from demo-cpp!"`.
   - `crux run --release` -> Built and executed Release binary, outputting `"Hello from demo-cpp!"`.
4. **Argument forwarding**:
   - Command: `crux run -- --name "Alice Smith" --count 5`
   - Child application printed:
     ```text
     argv[1] = --name
     argv[2] = Alice Smith
     argv[3] = --count
     argv[4] = 5
     ```
   - Confirmed discrete arguments, preserved spaces, and exact ordering.
5. **Exit status propagation**:
   - Child application configured to `return 42;`.
   - Executed `crux run`; shell `$?` evaluated to `42`.
   - Confirmed exact code propagation without misleading Crux error text.
6. **Working directory confirmation**:
   - Child application opened and wrote `"test_output.txt"` via relative path.
   - File verified at `demo-c/test_output.txt` (project root), not inside `.crux/build/`.
7. **Clean project boundary**:
   - Verified that no root-level `CMakeLists.txt` exists.
   - Verified that all build artifacts remain strictly inside `.crux/build/` (`debug/`, `release/`, `generated/`, `logs/`).
8. **Build failure prevents execution**:
   - Introduced syntax error into `src/main.c`.
   - Executed `crux run`; build failed with compiler error output and exit status 1.
   - Confirmed the binary was not executed (`"Running demo-c..."` was never emitted).

## 13. Backward compatibility
- All existing CLI commands (`init`, `check`, `lock`, `resolve`, `registry`, `fetch`, `build`, `install`, `integrate`, `toolchain`) remain completely functional.
- Existing package builds (`crux build <package>`), legacy dependency builds, and M19 application builds remain unchanged.
- Output layout conforms to established Crux directory conventions (`.crux/build/{debug,release,generated,logs}`).
- No external networking or new third-party crate dependencies were introduced.

## 14. Known limitations
- `crux run` executes single-target executable applications defined by a `[project]` manifest. Projects with `language = "other"` or legacy package manifests without an executable application cannot be executed.
- Advanced process orchestration features such as daemonization, file watching/auto-reload, manifest-defined custom environment overrides, and cross-compilation execution are outside the scope of M20.
- Signal mapping on non-Unix platforms is constrained by standard library `ExitStatus` availability (on Unix, POSIX `128 + signal` mapping is applied).

## 15. Confirmation of Git workflow compliance
In accordance with `AGENTS.md` and repository guidelines:
- No Git commits, pushes, branches, worktrees, pull requests, merges, rebases, cherry-picks, or resets were performed.
- All user modifications and untracked files provided in the workspace were preserved.
- Implementation and verification were conducted strictly within the working tree.
