//! Process-based sandbox for local development.
//!
//! **Warning**: This sandbox provides NO isolation and should only be used
//! for development with trusted code.

use async_trait::async_trait;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, instrument, warn};

use crate::{
    error::SandboxError,
    traits::{ExecutionResult, Sandbox},
};

/// Process-based sandbox for development.
///
/// Executes code directly in a subprocess. Provides NO security isolation.
#[derive(Clone)]
pub struct ProcessSandbox {
    timeout_ms: u64,
    shell: String,
}

impl ProcessSandbox {
    /// Create a new process sandbox.
    #[must_use]
    pub fn new() -> Self {
        Self {
            timeout_ms: 30000,
            shell: if cfg!(windows) {
                "cmd".to_string()
            } else {
                "sh".to_string()
            },
        }
    }

    /// Set execution timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Set shell to use.
    #[must_use]
    pub fn with_shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = shell.into();
        self
    }
}

impl Default for ProcessSandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Sandbox for ProcessSandbox {
    #[instrument(skip(self, code), fields(sandbox = "process"))]
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        warn!("ProcessSandbox provides NO security isolation!");
        debug!(
            "Executing code: {}...",
            code.chars().take(100).collect::<String>()
        );

        let start = std::time::Instant::now();

        let shell_arg = if cfg!(windows) { "/C" } else { "-c" };

        let mut command = Command::new(&self.shell);
        command
            .arg(shell_arg)
            .arg(code)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let child = command.spawn().map_err(SandboxError::IoError)?;
        // The guard also runs if the caller cancels this future. Kill the whole
        // group, including children that inherited stdout and background jobs.
        #[cfg(unix)]
        let _group = ProcessGroup(child.id().and_then(|id| i32::try_from(id).ok()));
        let output = tokio::time::timeout(
            std::time::Duration::from_millis(self.timeout_ms),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| SandboxError::Timeout)?
        .map_err(SandboxError::IoError)?;

        #[allow(clippy::cast_possible_truncation)]
        let execution_time_ms = start.elapsed().as_millis() as u64;

        Ok(ExecutionResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            execution_time_ms,
            artifacts: vec![],
        })
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        Ok(true) // Process sandbox is always ready
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        Ok(()) // Nothing to clean up
    }
}

#[cfg(unix)]
struct ProcessGroup(Option<i32>);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_simple_command() {
        let sandbox = ProcessSandbox::new();
        let result = sandbox.execute("echo hello").await.unwrap();

        assert!(result.is_success());
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_exit_code() {
        let sandbox = ProcessSandbox::new();
        let result = sandbox.execute("exit 42").await.unwrap();

        assert_eq!(result.exit_code, 42);
    }
}
