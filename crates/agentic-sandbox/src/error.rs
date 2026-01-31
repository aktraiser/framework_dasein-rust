//! Sandbox error types.

use thiserror::Error;

/// Errors that can occur during sandbox operations.
#[derive(Error, Debug)]
pub enum SandboxError {
    /// Failed to connect to sandbox runtime
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Sandbox is not ready
    #[error("Sandbox not ready: {0}")]
    NotReady(String),

    /// Sandbox not available (missing prerequisites)
    #[error("Sandbox not available: {0}")]
    NotAvailable(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Failed to create sandbox container/process
    #[error("Failed to create sandbox: {0}")]
    CreateFailed(String),

    /// Failed to start sandbox
    #[error("Failed to start sandbox: {0}")]
    StartFailed(String),

    /// Start error (alternative)
    #[error("Start error: {0}")]
    StartError(String),

    /// Execution timed out
    #[error("Execution timed out")]
    Timeout,

    /// Execution failed
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Failed to stop sandbox
    #[error("Failed to stop sandbox: {0}")]
    StopFailed(String),
}
