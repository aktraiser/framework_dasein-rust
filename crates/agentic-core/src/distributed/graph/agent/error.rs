//! Error types for the Agent Layer.

use thiserror::Error;

use super::types::AgentId;

/// Agent-level errors.
#[derive(Debug, Error)]
pub enum AgentError {
    /// LLM call failed.
    #[error("LLM error: {0}")]
    LLMError(String),

    /// Workflow execution failed.
    #[error("Workflow error: {0}")]
    WorkflowError(String),

    /// Thread error (persistence, state).
    #[error("Thread error: {0}")]
    ThreadError(String),

    /// Invalid input provided.
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Tool execution failed.
    #[error("Tool error: {tool_name}: {message}")]
    ToolError { tool_name: String, message: String },

    /// Agent not found.
    #[error("Agent not found: {0}")]
    AgentNotFound(AgentId),

    /// Operation timed out.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Stream was cancelled.
    #[error("Stream cancelled")]
    Cancelled,

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Memory operation error.
    #[error("Memory error: {0}")]
    MemoryError(String),
}

impl AgentError {
    /// Create an LLM error.
    pub fn llm(msg: impl Into<String>) -> Self {
        Self::LLMError(msg.into())
    }

    /// Create a workflow error.
    pub fn workflow(msg: impl Into<String>) -> Self {
        Self::WorkflowError(msg.into())
    }

    /// Create a thread error.
    pub fn thread(msg: impl Into<String>) -> Self {
        Self::ThreadError(msg.into())
    }

    /// Create an invalid input error.
    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::InvalidInput(msg.into())
    }

    /// Create a tool error.
    pub fn tool(name: impl Into<String>, msg: impl Into<String>) -> Self {
        Self::ToolError {
            tool_name: name.into(),
            message: msg.into(),
        }
    }

    /// Create a timeout error.
    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::Timeout(msg.into())
    }

    /// Create an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Create a memory error.
    pub fn memory(msg: impl Into<String>) -> Self {
        Self::MemoryError(msg.into())
    }

    /// Check if this error is retriable.
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            AgentError::LLMError(_) | AgentError::Timeout(_) | AgentError::WorkflowError(_)
        )
    }
}

impl From<agentic_llm::LLMError> for AgentError {
    fn from(err: agentic_llm::LLMError) -> Self {
        Self::LLMError(err.to_string())
    }
}

impl From<serde_json::Error> for AgentError {
    fn from(err: serde_json::Error) -> Self {
        Self::SerializationError(err.to_string())
    }
}

impl From<super::memory::MemoryError> for AgentError {
    fn from(err: super::memory::MemoryError) -> Self {
        Self::MemoryError(err.to_string())
    }
}

/// Result type for agent operations.
pub type AgentResult<T> = Result<T, AgentError>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = AgentError::llm("Connection failed");
        assert_eq!(err.to_string(), "LLM error: Connection failed");
    }

    #[test]
    fn test_tool_error() {
        let err = AgentError::tool("calculator", "Division by zero");
        assert!(err.to_string().contains("calculator"));
        assert!(err.to_string().contains("Division by zero"));
    }

    #[test]
    fn test_is_retriable() {
        assert!(AgentError::llm("timeout").is_retriable());
        assert!(AgentError::timeout("5s").is_retriable());
        assert!(!AgentError::invalid_input("bad data").is_retriable());
        assert!(!AgentError::Cancelled.is_retriable());
    }
}
