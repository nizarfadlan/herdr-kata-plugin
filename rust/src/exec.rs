//! Runs a CLI with captured output and a timeout, so a wedged `ctr` or
//! `herdr` can never hang an action.
//!
//! Note: output is read after the child exits. A child that blocks writing
//! more than the pipe buffer while never exiting is killed by the timeout;
//! use this only for short-lived probes, never for the interactive
//! `ctr run` in the bridge (which inherits the pane's stdio instead).

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How much of each stream is kept.
pub const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

/// The result of a command that ran to completion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOutput {
    /// The exit code, `None` when a signal ended the process.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CliOutput {
    /// stdout followed by stderr, as the failure messages show them.
    pub fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    pub fn success(&self) -> bool {
        self.status == Some(0)
    }
}

/// Why a command did not complete.
#[derive(Debug)]
pub enum ExecError {
    Spawn(std::io::Error),
    TimedOut,
}

/// Options of [`run_cli`].
pub struct RunOptions {
    pub timeout: Duration,
}

impl RunOptions {
    pub fn new(timeout: Duration) -> Self {
        RunOptions { timeout }
    }
}

fn read_capped(stream: Option<std::process::ChildStdout>, out: &mut String) {
    if let Some(stream) = stream {
        let _ = std::io::BufReader::new(stream)
            .take(MAX_OUTPUT_BYTES)
            .read_to_string(out);
    }
}

fn read_capped_err(stream: Option<std::process::ChildStderr>, out: &mut String) {
    if let Some(stream) = stream {
        let _ = std::io::BufReader::new(stream)
            .take(MAX_OUTPUT_BYTES)
            .read_to_string(out);
    }
}

/// Runs `program` with `args`, capturing both streams; kills the child when
/// `opts.timeout` elapses.
pub fn run_cli(program: &str, args: &[String], opts: &RunOptions) -> Result<CliOutput, ExecError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ExecError::Spawn)?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                read_capped(child.stdout.take(), &mut stdout);
                read_capped_err(child.stderr.take(), &mut stderr);
                return Ok(CliOutput {
                    status: status.code(),
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() > opts.timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ExecError::TimedOut);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(ExecError::Spawn(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_status() {
        let output = run_cli(
            "sh",
            &["-c".to_string(), "echo hello".to_string()],
            &RunOptions::new(Duration::from_secs(5)),
        )
        .expect("sh runs");
        assert!(output.success());
        assert_eq!(output.stdout.trim(), "hello");
    }

    #[test]
    fn timeout_kills_a_hung_child() {
        let error = run_cli(
            "sh",
            &["-c".to_string(), "sleep 30".to_string()],
            &RunOptions::new(Duration::from_millis(200)),
        )
        .expect_err("must time out");
        assert!(matches!(error, ExecError::TimedOut));
    }

    #[test]
    fn missing_binary_is_a_spawn_error() {
        let error = run_cli(
            "herdr-kata-definitely-not-a-binary",
            &[],
            &RunOptions::new(Duration::from_secs(5)),
        )
        .expect_err("must fail to spawn");
        assert!(matches!(error, ExecError::Spawn(_)));
    }
}
