//! Safe subprocess execution with captured output and optional timeouts.
//!
//! All external build tools (CMake, compilers, generators) are launched through
//! explicit argument vectors. No shell interpretation is performed.

use std::io::{self, Read};
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

/// Executes a command with captured output, terminating it if it exceeds `timeout`.
///
/// A zero timeout disables the limit. Standard input is closed so build tools can
/// never block waiting for interactive input.
pub fn run_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<ProcessOutput> {
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

    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));

    let started = Instant::now();
    let mut timed_out = false;

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }

        if !timeout.is_zero() && started.elapsed() >= timeout {
            timed_out = true;
            // Terminate the process and reap it so no zombie is left behind.
            let _ = child.kill();
            break child.wait()?;
        }

        thread::sleep(Duration::from_millis(20));
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    Ok(ProcessOutput {
        status,
        stdout,
        stderr,
        timed_out,
    })
}

fn read_all(mut reader: impl Read) -> Vec<u8> {
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    buffer
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
}
