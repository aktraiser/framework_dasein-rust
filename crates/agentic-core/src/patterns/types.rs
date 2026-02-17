//! Shared types for orchestration patterns.

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use thiserror::Error;

use crate::distributed::graph::ExecutorId;

// ============================================================================
// ERRORS
// ============================================================================

/// Errors that can occur in orchestration patterns.
#[derive(Debug, Error)]
pub enum PatternError {
    /// No participants were added to the pattern.
    #[error("No participants added to pattern")]
    NoParticipants,

    /// Invalid pattern configuration.
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    /// Aggregation failed.
    #[error("Aggregation failed: {0}")]
    AggregationFailed(String),

    /// Too many failures in concurrent execution.
    #[error("Too many failures: {failed} of {total}")]
    TooManyFailures { failed: usize, total: usize },

    /// Termination condition not met within max iterations.
    #[error("Max iterations ({0}) reached without termination")]
    MaxIterationsReached(usize),

    /// Workflow build error.
    #[error("Workflow build error: {0}")]
    WorkflowBuildError(String),

    /// Execution error.
    #[error("Execution error: {0}")]
    ExecutionError(String),
}

/// Result type for pattern operations.
pub type PatternResult<T> = Result<T, PatternError>;

// ============================================================================
// PARTICIPANT RESULT
// ============================================================================

/// Result from a single participant in concurrent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantResult {
    /// Executor/participant ID.
    pub executor_id: ExecutorId,

    /// Whether execution succeeded.
    pub success: bool,

    /// Output data (if successful).
    pub output: Option<serde_json::Value>,

    /// Error message (if failed).
    pub error: Option<String>,

    /// Execution duration in milliseconds.
    pub duration_ms: u64,
}

impl ParticipantResult {
    /// Create a successful result.
    pub fn success(
        executor_id: impl Into<ExecutorId>,
        output: serde_json::Value,
        duration_ms: u64,
    ) -> Self {
        Self {
            executor_id: executor_id.into(),
            success: true,
            output: Some(output),
            error: None,
            duration_ms,
        }
    }

    /// Create a failed result.
    pub fn failure(
        executor_id: impl Into<ExecutorId>,
        error: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            executor_id: executor_id.into(),
            success: false,
            output: None,
            error: Some(error.into()),
            duration_ms,
        }
    }

    /// Check if this result is successful.
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Check if this result is a failure.
    pub fn is_failure(&self) -> bool {
        !self.success
    }
}

// ============================================================================
// AGGREGATED RESULT
// ============================================================================

/// Result of aggregating multiple participant results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AggregatedResult {
    /// All results collected without modification.
    All(Vec<ParticipantResult>),

    /// Results combined into a single value (e.g., LLM synthesis).
    Combined(serde_json::Value),

    /// Partial results (some succeeded, some failed).
    Partial {
        /// Successful results.
        successes: Vec<ParticipantResult>,
        /// Number of failures.
        failure_count: usize,
    },

    /// Custom aggregation output.
    Custom {
        /// Aggregator name/type.
        aggregator: String,
        /// Aggregated output.
        output: serde_json::Value,
    },
}

impl AggregatedResult {
    /// Create from a list of all results.
    pub fn all(results: Vec<ParticipantResult>) -> Self {
        Self::All(results)
    }

    /// Create a combined result from a single value.
    pub fn combined(value: serde_json::Value) -> Self {
        Self::Combined(value)
    }

    /// Create a partial result (with some failures).
    pub fn partial(results: Vec<ParticipantResult>) -> Self {
        let (successes, failures): (Vec<_>, Vec<_>) =
            results.into_iter().partition(ParticipantResult::is_success);
        Self::Partial {
            successes,
            failure_count: failures.len(),
        }
    }

    /// Create a custom aggregation result.
    pub fn custom(aggregator: impl Into<String>, output: serde_json::Value) -> Self {
        Self::Custom {
            aggregator: aggregator.into(),
            output,
        }
    }

    /// Get all successful outputs as a vector.
    pub fn successful_outputs(&self) -> Vec<&serde_json::Value> {
        match self {
            Self::All(results) => results.iter().filter_map(|r| r.output.as_ref()).collect(),
            Self::Combined(value) => vec![value],
            Self::Partial { successes, .. } => {
                successes.iter().filter_map(|r| r.output.as_ref()).collect()
            }
            Self::Custom { output, .. } => vec![output],
        }
    }

    /// Get the number of successful results.
    pub fn success_count(&self) -> usize {
        match self {
            Self::All(results) => results.iter().filter(|r| r.is_success()).count(),
            Self::Combined(_) => 1,
            Self::Partial { successes, .. } => successes.len(),
            Self::Custom { .. } => 1,
        }
    }

    /// Get the number of failed results.
    pub fn failure_count(&self) -> usize {
        match self {
            Self::All(results) => results.iter().filter(|r| r.is_failure()).count(),
            Self::Combined(_) => 0,
            Self::Partial { failure_count, .. } => *failure_count,
            Self::Custom { .. } => 0,
        }
    }
}

// ============================================================================
// AGGREGATOR FUNCTION TYPE
// ============================================================================

/// Type alias for custom aggregator functions.
///
/// Takes a vector of participant results and returns an aggregated result.
///
/// # Example
///
/// ```rust,ignore
/// let aggregator: AggregatorFn = Box::new(|results| {
///     Box::pin(async move {
///         // Combine all successful outputs
///         let outputs: Vec<_> = results
///             .iter()
///             .filter_map(|r| r.output.clone())
///             .collect();
///         Ok(AggregatedResult::Combined(serde_json::json!(outputs)))
///     })
/// });
/// ```
pub type AggregatorFn = Box<
    dyn Fn(
            Vec<ParticipantResult>,
        ) -> Pin<Box<dyn Future<Output = PatternResult<AggregatedResult>> + Send>>
        + Send
        + Sync,
>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_participant_result_success() {
        let result = ParticipantResult::success("worker1", serde_json::json!({"answer": 42}), 100);

        assert!(result.is_success());
        assert!(!result.is_failure());
        assert_eq!(result.executor_id.as_str(), "worker1");
        assert!(result.output.is_some());
        assert!(result.error.is_none());
    }

    #[test]
    fn test_participant_result_failure() {
        let result = ParticipantResult::failure("worker2", "Connection timeout", 500);

        assert!(result.is_failure());
        assert!(!result.is_success());
        assert!(result.output.is_none());
        assert_eq!(result.error, Some("Connection timeout".into()));
    }

    #[test]
    fn test_aggregated_result_all() {
        let results = vec![
            ParticipantResult::success("a", serde_json::json!(1), 10),
            ParticipantResult::success("b", serde_json::json!(2), 20),
            ParticipantResult::failure("c", "error", 30),
        ];

        let agg = AggregatedResult::all(results);
        assert_eq!(agg.success_count(), 2);
        assert_eq!(agg.failure_count(), 1);
    }

    #[test]
    fn test_aggregated_result_partial() {
        let results = vec![
            ParticipantResult::success("a", serde_json::json!(1), 10),
            ParticipantResult::failure("b", "error", 20),
        ];

        let agg = AggregatedResult::partial(results);
        assert_eq!(agg.success_count(), 1);
        assert_eq!(agg.failure_count(), 1);
    }

    #[test]
    fn test_pattern_error_display() {
        let err = PatternError::NoParticipants;
        assert_eq!(err.to_string(), "No participants added to pattern");

        let err = PatternError::TooManyFailures {
            failed: 3,
            total: 5,
        };
        assert_eq!(err.to_string(), "Too many failures: 3 of 5");
    }
}
