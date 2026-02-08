//! Concurrent Pattern - Fan-out/fan-in parallel execution.
//!
//! A concurrent workflow broadcasts input to multiple executors in parallel,
//! then aggregates their results.
//!
//! # Features
//!
//! - Parallel execution of independent tasks
//! - Built-in aggregator executor
//! - Custom aggregation functions
//! - Partial failure handling
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::patterns::ConcurrentBuilder;
//!
//! // Build an analysis pipeline: input → [researcher, marketer, legal] → aggregator
//! let definition = ConcurrentBuilder::new("analysis")
//!     .add_participant("researcher")
//!     .add_participant("marketer")
//!     .add_participant("legal")
//!     .build()?;
//!
//! // Run with a registry of executors
//! let workflow = Workflow::new(definition, registry, config);
//! let result = workflow.run("Analyze this product launch").await?;
//! ```

use crate::distributed::graph::{ExecutorId, WorkflowBuilder, WorkflowDefinition};

use super::types::{PatternError, PatternResult};

// ============================================================================
// CONCURRENT BUILDER
// ============================================================================

/// Builder for concurrent (fan-out/fan-in) workflows.
///
/// Creates a workflow where input is broadcast to all participants,
/// they execute in parallel, and results are aggregated.
///
/// ```text
///                          Input
///                            │
///              ┌─────────────┼─────────────┐
///              ▼             ▼             ▼
///         ┌────────┐   ┌────────┐   ┌────────┐
///         │ Task A │   │ Task B │   │ Task C │   (parallel)
///         └───┬────┘   └───┬────┘   └───┬────┘
///             │            │            │
///             └────────────┼────────────┘
///                          ▼
///                   ┌────────────┐
///                   │ Aggregator │
///                   └────────────┘
/// ```
///
/// # Type Parameter
///
/// - `T`: Message type flowing through the workflow (default: `serde_json::Value`)
pub struct ConcurrentBuilder<T: Send + Sync + 'static = serde_json::Value> {
    /// Workflow ID.
    id: String,
    /// Human-readable name.
    name: Option<String>,
    /// Participant executor IDs (execute in parallel).
    participants: Vec<ExecutorId>,
    /// Custom aggregator ID (optional).
    aggregator_id: Option<ExecutorId>,
    /// Phantom for message type.
    _marker: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + 'static> ConcurrentBuilder<T> {
    /// Create a new concurrent workflow builder.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this workflow.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            participants: Vec::new(),
            aggregator_id: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// Set a human-readable name for the workflow.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a participant to the concurrent execution.
    ///
    /// All participants receive the same input and execute in parallel.
    pub fn add_participant(mut self, executor_id: impl Into<ExecutorId>) -> Self {
        self.participants.push(executor_id.into());
        self
    }

    /// Add multiple participants at once.
    pub fn add_participants(mut self, executor_ids: impl IntoIterator<Item = impl Into<ExecutorId>>) -> Self {
        for id in executor_ids {
            self.participants.push(id.into());
        }
        self
    }

    /// Set all participants at once (replaces existing).
    pub fn participants(mut self, executor_ids: impl IntoIterator<Item = impl Into<ExecutorId>>) -> Self {
        self.participants = executor_ids.into_iter().map(std::convert::Into::into).collect();
        self
    }

    /// Set a custom aggregator executor ID.
    ///
    /// If not set, a default aggregator named `{workflow_id}-aggregator` is used.
    /// The aggregator must be registered in the ExecutorRegistry when running.
    pub fn with_aggregator(mut self, aggregator_id: impl Into<ExecutorId>) -> Self {
        self.aggregator_id = Some(aggregator_id.into());
        self
    }

    /// Get the number of participants.
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    /// Get the aggregator ID that will be used.
    pub fn aggregator_id(&self) -> ExecutorId {
        self.aggregator_id
            .clone()
            .unwrap_or_else(|| ExecutorId::new(format!("{}-aggregator", self.id)))
    }

    /// Build the workflow definition.
    ///
    /// Creates a graph with:
    /// - A "splitter" node that broadcasts to all participants
    /// - All participants connected from the splitter via fan-out
    /// - All participants connected to the aggregator via fan-in
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No participants were added
    pub fn build(self) -> PatternResult<WorkflowDefinition<T>> {
        // Validate: need at least one participant
        if self.participants.is_empty() {
            return Err(PatternError::NoParticipants);
        }

        let splitter_id = ExecutorId::new(format!("{}-splitter", self.id));
        let aggregator_id = self.aggregator_id();

        // Build workflow
        let mut builder = WorkflowBuilder::<T>::new(&self.id);

        // Set optional name
        if let Some(name) = self.name {
            builder = builder.name(name);
        }

        // Start with splitter
        builder = builder.set_start(splitter_id.clone());

        // Add splitter and aggregator
        builder = builder.add_executor_id(splitter_id.clone());
        builder = builder.add_executor_id(aggregator_id.clone());

        // Add all participants
        for participant in &self.participants {
            builder = builder.add_executor_id(participant.clone());
        }

        // Fan-out: splitter → all participants
        builder = builder.add_fan_out_edge(splitter_id, self.participants.clone());

        // Fan-in: all participants → aggregator
        builder = builder.add_fan_in_edge(self.participants, aggregator_id);

        // Build and convert error
        builder
            .build()
            .map_err(|e| PatternError::WorkflowBuildError(e.to_string()))
    }
}

// ============================================================================
// CONVENIENCE FUNCTIONS
// ============================================================================

/// Create a concurrent workflow from a list of executor IDs.
///
/// Uses a default aggregator named `{id}-aggregator`.
///
/// # Example
///
/// ```rust,ignore
/// let definition = concurrent("analysis", vec!["researcher", "marketer", "legal"])?;
/// ```
pub fn concurrent<T: Send + Sync + 'static>(
    id: impl Into<String>,
    executor_ids: impl IntoIterator<Item = impl Into<ExecutorId>>,
) -> PatternResult<WorkflowDefinition<T>> {
    ConcurrentBuilder::new(id)
        .participants(executor_ids)
        .build()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::graph::EdgeKind;

    #[test]
    fn test_concurrent_builder_new() {
        let builder = ConcurrentBuilder::<serde_json::Value>::new("test");
        assert_eq!(builder.participant_count(), 0);
    }

    #[test]
    fn test_concurrent_builder_add_participant() {
        let builder = ConcurrentBuilder::<serde_json::Value>::new("test")
            .add_participant("worker1")
            .add_participant("worker2");

        assert_eq!(builder.participant_count(), 2);
    }

    #[test]
    fn test_concurrent_builder_build_empty() {
        let result = ConcurrentBuilder::<serde_json::Value>::new("test").build();

        assert!(result.is_err());
        match result.unwrap_err() {
            PatternError::NoParticipants => {}
            _ => panic!("Expected NoParticipants error"),
        }
    }

    #[test]
    fn test_concurrent_builder_build_single() {
        let definition = ConcurrentBuilder::<serde_json::Value>::new("test")
            .add_participant("worker")
            .build()
            .unwrap();

        // Should have: splitter, worker, aggregator
        assert_eq!(definition.executors.len(), 3);
        assert!(definition.has_executor(&ExecutorId::new("test-splitter")));
        assert!(definition.has_executor(&ExecutorId::new("worker")));
        assert!(definition.has_executor(&ExecutorId::new("test-aggregator")));
    }

    #[test]
    fn test_concurrent_builder_build_multiple() {
        let definition = ConcurrentBuilder::<serde_json::Value>::new("analysis")
            .name("Product Analysis")
            .add_participant("researcher")
            .add_participant("marketer")
            .add_participant("legal")
            .build()
            .unwrap();

        // Check workflow
        assert_eq!(definition.id.as_str(), "analysis");
        assert_eq!(definition.name, Some("Product Analysis".into()));

        // Check executors: splitter + 3 workers + aggregator = 5
        assert_eq!(definition.executors.len(), 5);

        // Check start is splitter
        assert_eq!(definition.start.as_str(), "analysis-splitter");

        // Check edges
        let edges = definition.edges.all();
        assert_eq!(edges.len(), 2); // 1 fan-out + 1 fan-in

        // Find fan-out and fan-in edges
        let fan_out = edges.iter().find(|e| e.kind == EdgeKind::FanOut);
        let fan_in = edges.iter().find(|e| e.kind == EdgeKind::FanIn);

        assert!(fan_out.is_some(), "Should have fan-out edge");
        assert!(fan_in.is_some(), "Should have fan-in edge");

        // Check fan-out targets
        let fan_out = fan_out.unwrap();
        let targets = fan_out.target.targets();
        assert_eq!(targets.len(), 3);

        // Check aggregator is terminal
        assert!(definition.is_terminal(&ExecutorId::new("analysis-aggregator")));
    }

    #[test]
    fn test_concurrent_builder_custom_aggregator() {
        let definition = ConcurrentBuilder::<serde_json::Value>::new("test")
            .add_participant("a")
            .add_participant("b")
            .with_aggregator("my-custom-aggregator")
            .build()
            .unwrap();

        assert!(definition.has_executor(&ExecutorId::new("my-custom-aggregator")));
        assert!(!definition.has_executor(&ExecutorId::new("test-aggregator")));
    }

    #[test]
    fn test_concurrent_convenience_function() {
        let definition = concurrent::<serde_json::Value>(
            "quick",
            vec!["x", "y", "z"],
        )
        .unwrap();

        assert_eq!(definition.executors.len(), 5); // splitter + 3 + aggregator
    }
}
