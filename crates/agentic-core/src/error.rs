//! Agent error types.

use thiserror::Error;

/// Errors that can occur during agent operations.
#[derive(Error, Debug)]
pub enum AgentError {
    /// LLM is not available
    #[error("LLM not available")]
    LLMNotAvailable,

    /// Sandbox is not ready
    #[error("Sandbox not ready")]
    SandboxNotReady,

    /// Sandbox is required but not configured
    #[error("Sandbox required for execution but not configured")]
    SandboxRequired,

    /// Model-generated code would run on a sandbox with no isolation
    #[error("Refusing to run model-generated code on a non-isolated sandbox; use an isolated sandbox or opt in with Agent::with_unisolated_execution(true)")]
    SandboxNotIsolated,

    /// LLM error
    #[error("LLM error: {0}")]
    LLMError(String),

    /// Sandbox error
    #[error("Sandbox error: {0}")]
    SandboxError(#[from] dasein_agentic_sandbox::SandboxError),

    /// Task execution failed
    #[error("Task execution failed: {0}")]
    ExecutionFailed(String),

    /// Agent is already running
    #[error("Agent is already running")]
    AlreadyRunning,

    /// Agent is not running
    #[error("Agent is not running")]
    NotRunning,

    /// Timeout
    #[error("Operation timed out")]
    Timeout,

    /// Invalid task
    #[error("Invalid task: {0}")]
    InvalidTask(String),
}
