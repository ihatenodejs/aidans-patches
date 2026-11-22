//! Subprocess adapter.
//!
//! The adapter is the only place that constructs a [`Command`]. It never
//! invokes a shell: the program and every argument are passed as separate argv
//! elements. Output is captured with a hard byte bound so a hostile or noisy
//! child cannot exhaust memory; excess bytes are drained and discarded.
//!
//! Callers configure secrets; diagnostics built here redact them before they
//! reach stderr or an error message.

use crate::error::Error;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Default retained bytes per captured stream.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const READ_BUFFER_BYTES: usize = 8 * 1024;
const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinMode {
    Inherit,
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Inherit,
    Null,
    Captured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedOutput {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

impl BoundedOutput {
    #[must_use]
    pub fn text_lossy(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: OsString,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout: Option<Duration>,
    /// Cooperative cancellation. A set flag kills and reaps the child.
    pub cancel: Option<Arc<AtomicBool>>,
    pub stdin: StdinMode,
    pub stdout: OutputMode,
    pub stderr: OutputMode,
    pub max_output_bytes: usize,
    pub secrets: Vec<String>,
}

impl ProcessSpec {
    #[must_use]
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            timeout: None,
            cancel: None,
            stdin: StdinMode::Null,
            stdout: OutputMode::Captured,
            stderr: OutputMode::Captured,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            secrets: Vec::new(),
        }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    #[must_use]
    pub fn stdin(mut self, mode: StdinMode) -> Self {
        self.stdin = mode;
        self
    }

    #[must_use]
    pub fn stdout(mut self, mode: OutputMode) -> Self {
        self.stdout = mode;
        self
    }

    #[must_use]
    pub fn stderr(mut self, mode: OutputMode) -> Self {
        self.stderr = mode;
        self
    }

    #[must_use]
    pub fn max_output_bytes(mut self, limit: usize) -> Self {
        self.max_output_bytes = limit;
        self
    }

    #[must_use]
    pub fn secret(mut self, secret: impl Into<String>) -> Self {
        self.secrets.push(secret.into());
        self
    }

    /// Removes every configured secret from diagnostic text. Longer secrets are
    /// replaced first so an overlapping shorter secret cannot leave a suffix.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        let mut secrets: Vec<&str> = self
            .secrets
            .iter()
            .map(String::as_str)
            .filter(|secret| !secret.is_empty())
            .collect();
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));

        let mut redacted = text.to_string();
        for secret in secrets {
            redacted = redacted.replace(secret, REDACTED);
        }
        redacted
    }

    /// Command line for diagnostics only. Never executed through a shell.
    #[must_use]
    pub fn describe_command(&self) -> String {
        let mut parts = Vec::with_capacity(self.args.len() + 1);
        parts.push(self.program.to_string_lossy().into_owned());
        parts.extend(self.args.iter().cloned());
        self.redact(&parts.join(" "))
    }
}

#[derive(Debug)]
pub struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: BoundedOutput,
    pub stderr: BoundedOutput,
    /// The child was killed because it exceeded the configured timeout.
    pub timed_out: bool,
    /// The child was killed because a shared cancellation flag was set.
    pub cancelled: bool,
}

impl ProcessOutput {
    #[must_use]
    pub fn success(&self) -> bool {
        !self.timed_out && !self.cancelled && self.status.success()
    }

    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.status.code()
    }

    /// Redacted stderr suitable for a diagnostic message.
    #[must_use]
    pub fn diagnostics(&self, spec: &ProcessSpec) -> String {
        spec.redact(&self.stderr.text_lossy())
    }
}

/// Runs one process to completion, a timeout, or an I/O failure.
///
/// A non-zero child exit and a timeout are successful adapter outcomes; callers
/// decide whether they are failures. Spawn and wait failures are [`Error`]s.
pub fn run(spec: &ProcessSpec) -> Result<ProcessOutput, Error> {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    command.stdin(match spec.stdin {
        StdinMode::Inherit => Stdio::inherit(),
        StdinMode::Null => Stdio::null(),
    });
    command.stdout(stdio_for(spec.stdout));
    command.stderr(stdio_for(spec.stderr));

    let mut child = command.spawn().map_err(|error| spawn_error(spec, &error))?;

    let stdout_reader = child
        .stdout
        .take()
        .map(|stream| spawn_reader(stream, spec.max_output_bytes));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stream| spawn_reader(stream, spec.max_output_bytes));

    let started = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::io(
                    format!("failed while waiting for '{}'", spec.describe_command()),
                    error,
                ));
            }
        }

        if spec
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            cancelled = true;
            let _ = child.kill();
            break child.wait().map_err(|error| {
                Error::io(
                    format!("failed to reap cancelled '{}'", spec.describe_command()),
                    error,
                )
            })?;
        }

        if let Some(limit) = spec.timeout {
            if started.elapsed() >= limit {
                timed_out = true;
                let _ = child.kill();
                break child.wait().map_err(|error| {
                    Error::io(
                        format!("failed to reap timed-out '{}'", spec.describe_command()),
                        error,
                    )
                })?;
            }
        }

        thread::sleep(POLL_INTERVAL);
    };

    let stdout = join_reader(stdout_reader, "stdout")?;
    let stderr = join_reader(stderr_reader, "stderr")?;

    Ok(ProcessOutput {
        status,
        stdout,
        stderr,
        timed_out,
        cancelled,
    })
}

fn stdio_for(mode: OutputMode) -> Stdio {
    match mode {
        OutputMode::Inherit => Stdio::inherit(),
        OutputMode::Null => Stdio::null(),
        OutputMode::Captured => Stdio::piped(),
    }
}

fn spawn_error(spec: &ProcessSpec, error: &io::Error) -> Error {
    if error.kind() == io::ErrorKind::NotFound {
        Error::usage(format!(
            "cannot execute '{}': executable not found",
            spec.describe_command()
        ))
    } else {
        Error::io(
            format!("cannot start '{}'", spec.describe_command()),
            io::Error::new(error.kind(), error.to_string()),
        )
    }
}

fn spawn_reader<R>(mut reader: R, limit: usize) -> thread::JoinHandle<io::Result<BoundedOutput>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || read_bounded(&mut reader, limit))
}

fn join_reader(
    handle: Option<thread::JoinHandle<io::Result<BoundedOutput>>>,
    stream: &str,
) -> Result<BoundedOutput, Error> {
    let Some(handle) = handle else {
        return Ok(BoundedOutput {
            bytes: Vec::new(),
            truncated: false,
        });
    };
    match handle.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(Error::io(format!("failed to read child {stream}"), error)),
        Err(_) => Err(Error::infrastructure(format!(
            "child {stream} reader thread panicked"
        ))),
    }
}

fn read_bounded<R: Read>(reader: &mut R, limit: usize) -> io::Result<BoundedOutput> {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buffer = vec![0u8; READ_BUFFER_BYTES];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(kept.len());
        if remaining == 0 {
            truncated = true;
            continue;
        }
        let take = remaining.min(read);
        kept.extend_from_slice(&buffer[..take]);
        if take < read {
            truncated = true;
        }
    }

    Ok(BoundedOutput {
        bytes: kept,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_boundary_is_exact() {
        let mut input: &[u8] = b"0123456789";
        let output = read_bounded(&mut input, 4).expect("bounded read");
        assert_eq!(output.bytes, b"0123");
        assert!(output.truncated);

        let mut exact: &[u8] = b"0123";
        let output = read_bounded(&mut exact, 4).expect("bounded read");
        assert_eq!(output.bytes, b"0123");
        assert!(!output.truncated);
    }

    #[test]
    fn redaction_removes_every_configured_secret() {
        let spec = ProcessSpec::new("tool")
            .arg("--token")
            .arg("super-secret")
            .secret("super-secret");
        assert!(!spec.describe_command().contains("super-secret"));
        assert!(spec.describe_command().contains(REDACTED));
        assert_eq!(spec.redact("super-secret"), REDACTED);
    }

    #[test]
    fn overlapping_secrets_are_redacted_longest_first() {
        let spec = ProcessSpec::new("tool")
            .secret("abc")
            .secret("abcdef")
            .secret("");
        assert_eq!(spec.redact("abcdef"), REDACTED);
        assert_eq!(spec.redact("abc"), REDACTED);
    }
}
