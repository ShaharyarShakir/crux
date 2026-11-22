//! Safe subprocess execution with captured output and optional timeouts.
//!
//! All external build tools (CMake, compilers, generators) are launched through
//! explicit argument vectors. No shell interpretation is performed.

use std::io::{self, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Result of executing a subprocess with captured standard streams.
#[derive(Debug)]
pub struct ProcessOutput {
    /// Process exit status (after any forced termination).
    pub status: ExitStatus,
    /// Captured standard output bytes.
    pub stdout: Vec<u8>,
    /// Captured standard error bytes.
    pub stderr: Vec<u8>,
    /// Whether the process exceeded its timeout and was terminated.
    pub timed_out: bool,
    /// Whether `stdout` holds only the most recent bytes (see [`run_logged`]).
    pub stdout_truncated: bool,
    /// Whether `stderr` holds only the most recent bytes (see [`run_logged`]).
    pub stderr_truncated: bool,
}

impl ProcessOutput {
    /// Returns the exit code, if the process exited normally.
    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    /// Returns true when the process completed successfully within the timeout.
    pub fn success(&self) -> bool {
        !self.timed_out && self.status.success()
    }
}

/// How long captured-output readers may keep running after the child has
/// exited (or been killed). A process that inherited the pipes, such as a
/// daemon spawned by a build tool, must not be able to block the caller.
pub const READER_GRACE_PERIOD: Duration = Duration::from_secs(2);

/// Executes a command with captured output, terminating it if it exceeds `timeout`.
///
/// A zero timeout disables the limit. Standard input is closed so build tools can
/// never block waiting for interactive input.
///
/// On timeout the child **and every descendant process** (for example Git's
/// `git-remote-https` or `ssh` helpers, or compiler processes spawned by a
/// build tool) are terminated with `SIGKILL`, and the child is reaped.
/// Descendants are discovered through `/proc` on Linux; on other platforms
/// only the direct child is killed.
///
/// After the child exits, output readers get at most [`READER_GRACE_PERIOD`]
/// to finish, so a surviving process that still holds the pipes cannot
/// extend the call indefinitely. Output written after that point is dropped.
pub fn run_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<ProcessOutput> {
    run_capturing(command, timeout, &CaptureOptions::default())
}

/// Receives subprocess output chunks as they arrive (for live `-vv` output).
pub type OutputTee = std::sync::Arc<dyn Fn(&[u8]) + Send + Sync>;

/// A cloneable, `Debug`-printable holder for an [`OutputTee`], so it can be
/// carried by request structs that derive `Debug`.
#[derive(Clone)]
pub struct OutputObserver(pub OutputTee);

impl std::fmt::Debug for OutputObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OutputObserver(..)")
    }
}

/// Formats a program and its arguments for display (logs and diagnostics).
/// Arguments containing whitespace or shell metacharacters are single-quoted.
/// The result is for humans only; commands are never run through a shell.
pub fn format_command_line<I, S>(program: S, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let quote = |arg: &std::ffi::OsStr| {
        let text = arg.to_string_lossy();
        if !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./=:+,@%".contains(c))
        {
            text.into_owned()
        } else {
            format!("'{}'", text.replace('\'', "'\\''"))
        }
    };
    std::iter::once(quote(program.as_ref()))
        .chain(args.into_iter().map(|a| quote(a.as_ref())))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Formats a prepared [`Command`] for display.
pub fn describe_command(command: &Command) -> String {
    format_command_line(
        command.get_program(),
        command.get_args().collect::<Vec<_>>(),
    )
}

/// Default number of bytes of each stream kept in memory by [`run_logged`].
pub const DEFAULT_TAIL_LIMIT: usize = 256 * 1024;

/// Options for [`run_logged`].
pub struct LoggedRun<'a> {
    /// Log file receiving the complete output, created or truncated.
    pub log_path: &'a std::path::Path,
    /// Text written at the top of the log (for example the command line).
    pub header: String,
    /// Bytes of each stream kept in memory for summaries and diagnostics.
    pub tail_limit: usize,
    /// Optional live observer of every output chunk.
    pub tee: Option<OutputTee>,
}

/// Like [`run_with_timeout`], but streams the complete output into a log file
/// as it is produced and keeps only the last `tail_limit` bytes of each
/// stream in memory.
///
/// Stdout and stderr are written to the log in arrival order (as a terminal
/// would show them), so compiler diagnostics, source locations, and caret
/// lines are preserved verbatim however large the output is. The log ends
/// with the exit status (and a timeout marker when the process was killed).
pub fn run_logged(
    command: &mut Command,
    timeout: Duration,
    options: LoggedRun<'_>,
) -> io::Result<ProcessOutput> {
    if let Some(parent) = options.log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(options.log_path)?;
    file.write_all(options.header.as_bytes())?;
    let log = std::sync::Arc::new(std::sync::Mutex::new(file));
    let capture = CaptureOptions {
        log: Some(std::sync::Arc::clone(&log)),
        tail_limit: Some(options.tail_limit.max(1)),
        tee: options.tee,
    };
    let output = run_capturing(command, timeout, &capture)?;
    if let Ok(mut file) = log.lock() {
        if output.timed_out {
            file.write_all(b"\n=== Process timed out and was terminated ===\n")?;
        }
        let status = match output.status.code() {
            Some(code) => format!("\n=== exit status: {code} ===\n"),
            None => "\n=== terminated by a signal ===\n".to_string(),
        };
        file.write_all(status.as_bytes())?;
        file.flush()?;
    }
    Ok(output)
}

#[derive(Default, Clone)]
struct CaptureOptions {
    log: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
    tail_limit: Option<usize>,
    tee: Option<OutputTee>,
}

fn run_capturing(
    command: &mut Command,
    timeout: Duration,
    options: &CaptureOptions,
) -> io::Result<ProcessOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "failed to capture stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "failed to capture stderr"))?;

    let stdout_reader = SharedReader::spawn(stdout, options.clone());
    let stderr_reader = SharedReader::spawn(stderr, options.clone());

    let started = Instant::now();
    let mut timed_out = false;

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }

        if !timeout.is_zero() && started.elapsed() >= timeout {
            timed_out = true;
            terminate_process_tree(&mut child);
            // Reap the child so no zombie is left behind.
            break child.wait()?;
        }

        thread::sleep(Duration::from_millis(20));
    };

    let deadline = Instant::now() + READER_GRACE_PERIOD;
    let (stdout, stdout_truncated) = stdout_reader.finish(deadline);
    let (stderr, stderr_truncated) = stderr_reader.finish(deadline);

    Ok(ProcessOutput {
        status,
        stdout,
        stderr,
        timed_out,
        stdout_truncated,
        stderr_truncated,
    })
}

/// Drains a pipe on a background thread into a shared buffer so the caller
/// can stop waiting at a deadline without losing what was already read.
struct SharedReader {
    buffer: std::sync::Arc<std::sync::Mutex<(Vec<u8>, bool)>>,
    done: std::sync::mpsc::Receiver<()>,
}

impl SharedReader {
    fn spawn(mut reader: impl Read + Send + 'static, options: CaptureOptions) -> Self {
        let buffer = std::sync::Arc::new(std::sync::Mutex::new((Vec::new(), false)));
        let (tx, done) = std::sync::mpsc::channel();
        let sink = std::sync::Arc::clone(&buffer);
        thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let bytes = &chunk[..n];
                        if let Some(log) = &options.log
                            && let Ok(mut file) = log.lock()
                        {
                            let _ = file.write_all(bytes);
                        }
                        if let Some(tee) = &options.tee {
                            tee(bytes);
                        }
                        if let Ok(mut guard) = sink.lock() {
                            let (buf, truncated) = &mut *guard;
                            buf.extend_from_slice(bytes);
                            // Keep at most `limit` bytes; trim in batches so
                            // the cost stays amortized O(1) per byte.
                            if let Some(limit) = options.tail_limit
                                && buf.len() > limit.saturating_mul(2)
                            {
                                let excess = buf.len() - limit;
                                buf.drain(..excess);
                                *truncated = true;
                            }
                        }
                    }
                }
            }
            let _ = tx.send(());
        });
        Self { buffer, done }
    }

    /// Waits for end-of-stream until `deadline`, then returns the bytes read
    /// (only the tail when a limit applies) and whether earlier bytes were
    /// dropped.
    fn finish(self, deadline: Instant) -> (Vec<u8>, bool) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = self.done.recv_timeout(remaining);
        self.buffer
            .lock()
            .map(|guard| (guard.0.clone(), guard.1))
            .unwrap_or_default()
    }
}

/// Kills `child` and all of its descendants, descendants first.
///
/// Descendants are collected while the child is still alive (afterwards they
/// would be re-parented and no longer discoverable). Collection is repeated a
/// few times so processes forked during termination are also caught.
fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(target_os = "linux")]
    {
        let root = child.id();
        for _ in 0..3 {
            let descendants = linux_descendants(root);
            if descendants.is_empty() {
                break;
            }
            for pid in descendants.iter().rev() {
                send_sigkill(*pid);
            }
        }
    }
    let _ = child.kill();
}

/// Returns all transitive descendants of `root`, parents before children.
#[cfg(target_os = "linux")]
fn linux_descendants(root: u32) -> Vec<u32> {
    let mut children: std::collections::BTreeMap<u32, Vec<u32>> = std::collections::BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            // Format: `pid (comm) state ppid ...`; `comm` may contain spaces
            // or parentheses, so parse from the last ')'.
            let Some(after_comm) = stat.rfind(')').map(|i| &stat[i + 1..]) else {
                continue;
            };
            let mut fields = after_comm.split_whitespace();
            let _state = fields.next();
            if let Some(ppid) = fields.next().and_then(|p| p.parse::<u32>().ok()) {
                children.entry(ppid).or_default().push(pid);
            }
        }
    }
    let mut ordered = Vec::new();
    let mut queue = std::collections::VecDeque::from([root]);
    while let Some(pid) = queue.pop_front() {
        if let Some(kids) = children.get(&pid) {
            for kid in kids {
                if *kid != root && !ordered.contains(kid) {
                    ordered.push(*kid);
                    queue.push_back(*kid);
                }
            }
        }
    }
    ordered
}

#[cfg(target_os = "linux")]
fn send_sigkill(pid: u32) {
    const SIGKILL: i32 = 9;
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    if let Ok(pid) = i32::try_from(pid)
        && pid > 1
    {
        // SAFETY: `kill(2)` has no memory-safety preconditions; `pid` is a
        // positive, specific process id (never 0, -1, or a process group),
        // so only that single process is signalled.
        unsafe {
            kill(pid, SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_with_timeout_captures_stdout() {
        let mut command = Command::new("printf");
        command.arg("hello");

        let output = run_with_timeout(&mut command, Duration::from_secs(30)).unwrap();
        assert!(output.success());
        assert_eq!(output.stdout, b"hello");
        assert!(!output.timed_out);
    }

    #[test]
    fn test_run_with_timeout_missing_executable() {
        let mut command = Command::new("crux_definitely_missing_binary_xyz");
        let result = run_with_timeout(&mut command, Duration::from_secs(1));
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_run_with_timeout_terminates_long_process() {
        let mut command = Command::new("sleep");
        command.arg("10");

        let output = run_with_timeout(&mut command, Duration::from_millis(100)).unwrap();
        assert!(output.timed_out);
        assert!(!output.success());
    }

    #[cfg(target_os = "linux")]
    fn process_alive(pid: &str) -> bool {
        // A reaped process disappears; a zombie (`Z`) is no longer running.
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|stat| {
                stat.rfind(')')
                    .and_then(|i| stat[i + 1..].split_whitespace().next())
                    .is_some_and(|state| state != "Z")
            })
            .unwrap_or(false)
    }

    /// Regression: on timeout only the direct child was killed, and joining
    /// the output readers then blocked until every grandchild holding the
    /// pipes (e.g. `git-remote-https`) exited on its own.
    #[cfg(target_os = "linux")]
    #[test]
    fn test_timeout_kills_descendants_and_returns_promptly() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("sleep 30 & echo grandchild=$!; wait");
        let started = Instant::now();
        let output = run_with_timeout(&mut command, Duration::from_millis(300)).unwrap();
        let elapsed = started.elapsed();
        assert!(output.timed_out);
        assert!(
            elapsed < Duration::from_secs(5),
            "timeout must bound the call, took {elapsed:?}"
        );
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let pid = text
            .trim()
            .strip_prefix("grandchild=")
            .expect("grandchild pid captured before the timeout")
            .to_string();
        let gone_by = Instant::now() + Duration::from_secs(3);
        while process_alive(&pid) && Instant::now() < gone_by {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !process_alive(&pid),
            "grandchild {pid} survived the timeout"
        );
    }

    /// A process that outlives a *successful* child while holding the pipes
    /// (for example a build-tool daemon) must not block the caller, and is
    /// intentionally not killed.
    #[cfg(unix)]
    #[test]
    fn test_lingering_pipe_holder_does_not_block_successful_exit() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf done; (sleep 30 &)");
        let started = Instant::now();
        let output = run_with_timeout(&mut command, Duration::from_secs(60)).unwrap();
        assert!(output.success());
        assert_eq!(output.stdout, b"done");
        assert!(
            started.elapsed() < READER_GRACE_PERIOD + Duration::from_secs(3),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn test_large_output_is_fully_captured() {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("head -c 3000000 /dev/zero; echo err >&2");
        let output = run_with_timeout(&mut command, Duration::from_secs(30)).unwrap();
        assert!(output.success());
        assert_eq!(output.stdout.len(), 3_000_000);
        assert_eq!(output.stderr, b"err\n");
    }

    #[test]
    fn run_logged_streams_everything_to_the_log_and_bounds_memory() {
        let dir = std::env::temp_dir().join(format!(
            "crux_run_logged_{}",
            crate::core::filesystem::unique_temp_suffix()
        ));
        let log = dir.join("logs/build.log");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let counter = std::sync::Arc::clone(&seen);
        let mut command = Command::new("sh");
        // 2 MB of stdout, then a compiler-style diagnostic on stderr.
        command.arg("-c").arg(
            "head -c 2000000 /dev/zero | tr '\\0' x; \
             printf 'src/a.c:3:5: error: expected ;\\n    int x\\n        ^\\n' >&2; exit 2",
        );
        let output = run_logged(
            &mut command,
            Duration::from_secs(30),
            LoggedRun {
                log_path: &log,
                header: "$ sh -c ...\n\n".to_string(),
                tail_limit: 4096,
                tee: Some(std::sync::Arc::new(move |bytes: &[u8]| {
                    *counter.lock().unwrap() += bytes.len();
                })),
            },
        )
        .unwrap();

        assert_eq!(output.code(), Some(2));
        assert!(
            output.stdout.len() <= 2 * 4096,
            "memory is bounded: {}",
            output.stdout.len()
        );
        assert!(output.stdout_truncated);
        assert!(!output.stderr_truncated);
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "src/a.c:3:5: error: expected ;\n    int x\n        ^\n"
        );
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.starts_with("$ sh -c ...\n\n"));
        assert!(
            text.contains(&"x".repeat(2_000_000)),
            "full stdout preserved in the log"
        );
        assert!(text.contains("src/a.c:3:5: error: expected ;\n    int x\n        ^\n"));
        assert!(text.ends_with("=== exit status: 2 ===\n"));
        assert_eq!(*seen.lock().unwrap(), 2_000_000 + output.stderr.len());
        let _ = std::fs::remove_dir_all(dir);
    }
}
