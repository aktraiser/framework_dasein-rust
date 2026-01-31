//! Core types for the graph-based multi-agent architecture.

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

// ============================================================================
// IDENTIFIERS
// ============================================================================

/// Unique identifier for an executor node in the graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecutorId(String);

impl ExecutorId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExecutorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ExecutorId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for ExecutorId {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// Unique identifier for a task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(format!("task-{}", uuid::Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(String);

impl WorkflowId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(format!("wf-{}", uuid::Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// ERRORS
// ============================================================================

/// Graph-level errors.
#[derive(Debug, Error)]
pub enum GraphError {
    #[error("Executor not found: {0}")]
    ExecutorNotFound(ExecutorId),

    #[error("Edge not found: {0}")]
    EdgeNotFound(String),

    #[error("Invalid graph: {0}")]
    InvalidGraph(String),

    #[error("Type mismatch: {0}")]
    TypeMismatch(String),

    #[error("Executor failed: {0}")]
    ExecutorFailed(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Max retries exceeded for executor: {0}")]
    MaxRetriesExceeded(ExecutorId),

    #[error("Workflow cancelled")]
    Cancelled,
}

pub type GraphResult<T> = Result<T, GraphError>;

/// Executor-level error with metadata for routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorError {
    pub executor_id: ExecutorId,
    pub message: String,
    pub retriable: bool,
    pub category: ErrorCategory,
}

impl ExecutorError {
    pub fn new(executor_id: impl Into<ExecutorId>, message: impl Into<String>) -> Self {
        Self {
            executor_id: executor_id.into(),
            message: message.into(),
            retriable: true,
            category: ErrorCategory::Unknown,
        }
    }

    /// Create an internal error (non-retriable).
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            executor_id: ExecutorId::new("internal"),
            message: message.into(),
            retriable: false,
            category: ErrorCategory::Unknown,
        }
    }

    /// Create a validation error.
    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            executor_id: ExecutorId::new("validation"),
            message: message.into(),
            retriable: true,
            category: ErrorCategory::Compilation,
        }
    }

    /// Create a timeout error.
    pub fn timeout() -> Self {
        Self {
            executor_id: ExecutorId::new("timeout"),
            message: "Operation timed out".into(),
            retriable: true,
            category: ErrorCategory::Timeout,
        }
    }

    pub fn non_retriable(mut self) -> Self {
        self.retriable = false;
        self
    }

    pub fn with_category(mut self, category: ErrorCategory) -> Self {
        self.category = category;
        self
    }
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.executor_id, self.message)
    }
}

impl std::error::Error for ExecutorError {}

/// Error category for conditional routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ErrorCategory {
    Compilation,
    TestFailure,
    Linting,
    Timeout,
    LLMError,
    #[default]
    Unknown,
}

// ============================================================================
// EVENTS
// ============================================================================

/// Events emitted during executor processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutorEvent {
    Started {
        executor_id: ExecutorId,
        task_id: TaskId,
    },
    Progress {
        executor_id: ExecutorId,
        percent: u8,
        message: Option<String>,
    },
    Completed {
        executor_id: ExecutorId,
        duration_ms: u64,
    },
    Failed {
        executor_id: ExecutorId,
        error: String,
    },
}

/// Events emitted by edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EdgeEvent {
    Activated {
        edge_id: String,
        from: ExecutorId,
        to: ExecutorId,
    },
    ConditionEvaluated {
        edge_id: String,
        result: bool,
    },
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_id() {
        let id1 = ExecutorId::new("code-gen");
        let id2: ExecutorId = "code-gen".into();
        assert_eq!(id1, id2);
        assert_eq!(id1.as_str(), "code-gen");
    }

    #[test]
    fn test_task_id_generate() {
        let id = TaskId::generate();
        assert!(id.as_str().starts_with("task-"));
    }

    #[test]
    fn test_executor_error() {
        let err = ExecutorError::new("validator", "Compilation failed")
            .with_category(ErrorCategory::Compilation)
            .non_retriable();

        assert!(!err.retriable);
        assert_eq!(err.category, ErrorCategory::Compilation);
    }

    #[test]
    fn test_executor_id_in_hashset() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ExecutorId::new("a"));
        set.insert(ExecutorId::new("b"));
        set.insert(ExecutorId::new("a"));
        assert_eq!(set.len(), 2);
    }
}
