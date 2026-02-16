//! Sequential Pattern - Linear pipeline execution.
//!
//! A sequential workflow chains executors: A → B → C → D
//! Each executor receives messages from the previous one.
//!
//! # Features
//!
//! - Simple linear flow
//! - Each step sees accumulated context
//! - Optional early exit conditions
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::patterns::SequentialBuilder;
//!
//! // Build a content pipeline: writer → reviewer → polisher
//! let definition = SequentialBuilder::new("content-pipeline")
//!     .add_participant("writer")
//!     .add_participant("reviewer")
//!     .add_participant("polisher")
//!     .build()?;
//!
//! // Run with a registry of executors
//! let workflow = Workflow::new(definition, registry, config);
//! let result = workflow.run("Write about AI").await?;
//! ```

use crate::distributed::graph::{ExecutorId, WorkflowBuilder, WorkflowDefinition};

use super::types::{PatternError, PatternResult};

// ============================================================================
// SEQUENTIAL BUILDER
// ============================================================================

/// Builder for sequential (pipeline) workflows.
///
/// Creates a linear chain where each executor passes its output to the next.
///
/// ```text
/// ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐
/// │ Step 1  │───▶│ Step 2  │───▶│ Step 3  │───▶│ Step 4  │
/// └─────────┘    └─────────┘    └─────────┘    └─────────┘
/// ```
///
/// # Type Parameter
///
/// - `T`: Message type flowing through the pipeline (default: `serde_json::Value`)
pub struct SequentialBuilder<T: Send + Sync + 'static = serde_json::Value> {
    /// Workflow ID.
    id: String,
    /// Human-readable name.
    name: Option<String>,
    /// Ordered list of participant executor IDs.
    participants: Vec<ExecutorId>,
    /// Phantom for message type.
    _marker: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + 'static> SequentialBuilder<T> {
    /// Create a new sequential workflow builder.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this workflow.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            participants: Vec::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Set a human-readable name for the workflow.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a participant to the end of the sequence.
    ///
    /// Participants execute in the order they are added.
    pub fn add_participant(mut self, executor_id: impl Into<ExecutorId>) -> Self {
        self.participants.push(executor_id.into());
        self
    }

    /// Add multiple participants at once.
    ///
    /// They will be appended to the sequence in order.
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

    /// Get the number of participants.
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    /// Build the workflow definition.
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

        // Build workflow using WorkflowBuilder
        let mut builder = WorkflowBuilder::<T>::new(&self.id);

        // Set optional name
        if let Some(name) = self.name {
            builder = builder.name(name);
        }

        // First participant is the start
        builder = builder.set_start(self.participants[0].clone());

        // Add all participants
        for participant in &self.participants {
            builder = builder.add_executor_id(participant.clone());
        }

        // Chain participants: A → B → C → D
        for window in self.participants.windows(2) {
            let from = &window[0];
            let to = &window[1];
            builder = builder.add_direct_edge(from.clone(), to.clone());
        }

        // Build and convert error
        builder
            .build()
            .map_err(|e| PatternError::WorkflowBuildError(e.to_string()))
    }
}

// ============================================================================
// CONVENIENCE FUNCTIONS
// ============================================================================

/// Create a sequential workflow from a list of executor IDs.
///
/// # Example
///
/// ```rust,ignore
/// let definition = sequential("pipeline", vec!["step1", "step2", "step3"])?;
/// ```
pub fn sequential<T: Send + Sync + 'static>(
    id: impl Into<String>,
    executor_ids: impl IntoIterator<Item = impl Into<ExecutorId>>,
) -> PatternResult<WorkflowDefinition<T>> {
    SequentialBuilder::new(id)
        .participants(executor_ids)
        .build()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequential_builder_new() {
        let builder = SequentialBuilder::<serde_json::Value>::new("test");
        assert_eq!(builder.participant_count(), 0);
    }

    #[test]
    fn test_sequential_builder_add_participant() {
        let builder = SequentialBuilder::<serde_json::Value>::new("test")
            .add_participant("step1")
            .add_participant("step2");

        assert_eq!(builder.participant_count(), 2);
    }

    #[test]
    fn test_sequential_builder_add_participants() {
        let builder = SequentialBuilder::<serde_json::Value>::new("test")
            .add_participants(vec!["step1", "step2", "step3"]);

        assert_eq!(builder.participant_count(), 3);
    }

    #[test]
    fn test_sequential_builder_build_empty() {
        let result = SequentialBuilder::<serde_json::Value>::new("test").build();

        assert!(result.is_err());
        match result.unwrap_err() {
            PatternError::NoParticipants => {}
            _ => panic!("Expected NoParticipants error"),
        }
    }

    #[test]
    fn test_sequential_builder_build_single() {
        let definition = SequentialBuilder::<serde_json::Value>::new("test")
            .add_participant("only")
            .build()
            .unwrap();

        assert_eq!(definition.start.as_str(), "only");
        assert_eq!(definition.executors.len(), 1);
        assert!(definition.edges.all().is_empty()); // No edges for single node
    }

    #[test]
    fn test_sequential_builder_build_chain() {
        let definition = SequentialBuilder::<serde_json::Value>::new("pipeline")
            .name("Content Pipeline")
            .add_participant("writer")
            .add_participant("reviewer")
            .add_participant("polisher")
            .build()
            .unwrap();

        // Check workflow ID and name
        assert_eq!(definition.id.as_str(), "pipeline");
        assert_eq!(definition.name, Some("Content Pipeline".into()));

        // Check executors
        assert_eq!(definition.executors.len(), 3);
        assert!(definition.has_executor(&ExecutorId::new("writer")));
        assert!(definition.has_executor(&ExecutorId::new("reviewer")));
        assert!(definition.has_executor(&ExecutorId::new("polisher")));

        // Check start
        assert_eq!(definition.start.as_str(), "writer");

        // Check edges: writer → reviewer, reviewer → polisher
        let edges = definition.edges.all();
        assert_eq!(edges.len(), 2);

        // Verify chain
        let successors_writer = definition.successors(&ExecutorId::new("writer"));
        assert_eq!(successors_writer.len(), 1);
        assert_eq!(successors_writer[0].as_str(), "reviewer");

        let successors_reviewer = definition.successors(&ExecutorId::new("reviewer"));
        assert_eq!(successors_reviewer.len(), 1);
        assert_eq!(successors_reviewer[0].as_str(), "polisher");

        // Polisher is terminal
        assert!(definition.is_terminal(&ExecutorId::new("polisher")));
    }

    #[test]
    fn test_sequential_convenience_function() {
        let definition = sequential::<serde_json::Value>(
            "quick-pipeline",
            vec!["a", "b", "c"],
        )
        .unwrap();

        assert_eq!(definition.executors.len(), 3);
        assert_eq!(definition.start.as_str(), "a");
    }
}
