//! Executor trait - The core abstraction for graph nodes.
//!
//! An Executor is a node in the workflow graph that processes input
//! and produces output. Three kinds exist:
//! - **Worker**: Does work (LLM calls, transformations)
//! - **Validator**: Verifies something (compile, test, lint)
//! - **Orchestrator**: Coordinates a sub-graph
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::{Executor, ExecutorKind, WorkflowContext};
//!
//! struct CodeGenerator {
//!     id: ExecutorId,
//! }
//!
//! #[async_trait]
//! impl Executor for CodeGenerator {
//!     type Input = String;
//!     type Message = GeneratedCode;
//!     type Output = ();
//!
//!     fn id(&self) -> &ExecutorId { &self.id }
//!     fn kind(&self) -> ExecutorKind { ExecutorKind::Worker }
//!
//!     async fn handle(
//!         &self,
//!         input: Self::Input,
//!         ctx: &mut WorkflowContext<Self::Message, Self::Output>,
//!     ) -> Result<(), ExecutorError> {
//!         let code = generate_code(&input).await?;
//!         ctx.send_message(code).await?;
//!         Ok(())
//!     }
//! }
//! ```

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

use super::types::{ExecutorError, ExecutorId};

// ============================================================================
// EXECUTOR KIND
// ============================================================================

/// The type of executor - determines its role in the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorKind {
    /// Does work: LLM calls, transformations, merging.
    Worker,
    /// Verifies something: compile, test, lint, review.
    Validator,
    /// Coordinates a sub-graph (recursive).
    Orchestrator,
}

impl std::fmt::Display for ExecutorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorKind::Worker => write!(f, "Worker"),
            ExecutorKind::Validator => write!(f, "Validator"),
            ExecutorKind::Orchestrator => write!(f, "Orchestrator"),
        }
    }
}

// ============================================================================
// EXECUTOR TRAIT
// ============================================================================

/// The core trait for all graph executors.
///
/// An executor processes input and communicates via the WorkflowContext:
/// - `send_message()` → sends to connected executors (internal)
/// - `yield_output()` → produces output visible to caller (external)
/// - `add_event()` → emits observability events
///
/// # Type Parameters
///
/// - `Input`: The type of data this executor receives
/// - `Message`: The type sent to other executors via edges
/// - `Output`: The type visible to the workflow caller
#[async_trait]
pub trait Executor: Send + Sync {
    /// Input type accepted by this executor.
    type Input: Serialize + DeserializeOwned + Send + Sync;

    /// Message type sent to connected executors.
    type Message: Serialize + DeserializeOwned + Send + Sync;

    /// Output type visible to workflow caller.
    type Output: Serialize + DeserializeOwned + Send + Sync;

    /// Unique identifier for this executor.
    fn id(&self) -> &ExecutorId;

    /// The kind of executor (Worker, Validator, Orchestrator).
    fn kind(&self) -> ExecutorKind;

    /// Process input and use context to communicate.
    ///
    /// Use the context to:
    /// - `ctx.send_message(msg)` - Send to connected executors
    /// - `ctx.yield_output(out)` - Produce output for caller
    /// - `ctx.add_event(evt)` - Emit observability event
    /// - `ctx.previous_errors()` - Access previous attempt errors
    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send;

    /// Optional: Human-readable name for logging.
    fn name(&self) -> &str {
        self.id().as_str()
    }

    /// Optional: Description of what this executor does.
    fn description(&self) -> Option<&str> {
        None
    }
}

// ============================================================================
// EXECUTOR CONTEXT TRAIT
// ============================================================================

/// Context provided to executors for communication.
///
/// This trait defines the interface executors use to:
/// - Send messages to other executors
/// - Yield outputs to the workflow caller
/// - Emit events for observability
/// - Access execution history
#[async_trait]
pub trait ExecutorContext<TMessage, TOutput>: Send + Sync
where
    TMessage: Serialize + Send,
    TOutput: Serialize + Send,
{
    /// Send a message to connected executors (via edges).
    async fn send_message(&mut self, message: TMessage) -> Result<(), ExecutorError>;

    /// Yield an output visible to the workflow caller.
    async fn yield_output(&mut self, output: TOutput) -> Result<(), ExecutorError>;

    /// Emit a custom event for observability.
    async fn add_event(
        &mut self,
        event_type: &str,
        data: serde_json::Value,
    ) -> Result<(), ExecutorError>;

    /// Get errors from previous execution attempts.
    fn previous_errors(&self) -> &[ExecutorError];

    /// Log a message (convenience method).
    async fn log(&mut self, level: LogLevel, message: &str) -> Result<(), ExecutorError> {
        self.add_event(
            "log",
            serde_json::json!({
                "level": format!("{:?}", level),
                "message": message,
            }),
        )
        .await
    }
}

/// Log levels for executor logging.
#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

// ============================================================================
// VALIDATION RESULT (for Validator executors)
// ============================================================================

/// Result from a Validator executor.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ValidationResult {
    /// Whether validation passed.
    pub passed: bool,
    /// Error messages if failed.
    pub errors: Vec<String>,
    /// Structured feedback for retry.
    pub feedback: Option<String>,
}

impl ValidationResult {
    pub fn success() -> Self {
        Self {
            passed: true,
            errors: vec![],
            feedback: None,
        }
    }

    pub fn failure(errors: Vec<String>) -> Self {
        Self {
            passed: false,
            errors,
            feedback: None,
        }
    }

    pub fn with_feedback(mut self, feedback: impl Into<String>) -> Self {
        self.feedback = Some(feedback.into());
        self
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_kind_display() {
        assert_eq!(format!("{}", ExecutorKind::Worker), "Worker");
        assert_eq!(format!("{}", ExecutorKind::Validator), "Validator");
        assert_eq!(format!("{}", ExecutorKind::Orchestrator), "Orchestrator");
    }

    #[test]
    fn test_validation_result_success() {
        let result = ValidationResult::success();
        assert!(result.passed);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validation_result_failure() {
        let result = ValidationResult::failure(vec!["Error 1".into(), "Error 2".into()])
            .with_feedback("Fix the errors");

        assert!(!result.passed);
        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.feedback, Some("Fix the errors".into()));
    }
}
