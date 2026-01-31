//! Sandbox traits and types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::SandboxError;

/// An artifact produced during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxArtifact {
    /// Path to the artifact
    pub path: String,
    /// Content of the artifact
    pub content: String,
}

/// Result of code execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Exit code (0 = success)
    pub exit_code: i32,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
    /// Generated artifacts (files)
    pub artifacts: Vec<SandboxArtifact>,
}

impl ExecutionResult {
    /// Check if execution was successful.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Trait for sandbox implementations.
///
/// A sandbox provides isolated code execution with resource limits.
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Execute code in the sandbox.
    ///
    /// # Arguments
    ///
    /// * `code` - The code/command to execute
    ///
    /// # Errors
    ///
    /// Returns an error if execution fails.
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError>;

    /// Check if the sandbox is ready.
    ///
    /// # Errors
    ///
    /// Returns an error if the sandbox is not accessible.
    async fn is_ready(&self) -> Result<bool, SandboxError>;

    /// Stop and cleanup the sandbox.
    ///
    /// # Errors
    ///
    /// Returns an error if cleanup fails.
    async fn stop(&self) -> Result<(), SandboxError>;
}

/// Implementation of Sandbox for Box<dyn Sandbox>.
/// This allows using trait objects with generic sandbox validators.
#[async_trait]
impl Sandbox for Box<dyn Sandbox> {
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        (**self).execute(code).await
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        (**self).is_ready().await
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        (**self).stop().await
    }
}
