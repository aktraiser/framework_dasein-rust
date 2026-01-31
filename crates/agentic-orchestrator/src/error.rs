//! Orchestrator error types.

use thiserror::Error;

/// Errors that can occur during orchestration.
#[derive(Error, Debug)]
pub enum OrchestratorError {
    /// Agent not found
    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    /// Agent error
    #[error("Agent error: {0}")]
    AgentError(String),

    /// Timeout
    #[error("Operation timed out")]
    Timeout,

    /// Circular dependency in workflow
    #[error("Circular dependency detected in workflow")]
    CircularDependency,

    /// Join error
    #[error("Task join error: {0}")]
    JoinError(String),

    /// Workflow execution failed
    #[error("Workflow failed: {0}")]
    WorkflowFailed(String),
}
