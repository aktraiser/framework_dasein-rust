//! Handoff Pattern - Dynamic mesh routing between specialists.
//!
//! In a handoff workflow, agents can dynamically route to each other based on
//! the content of messages. This is useful for specialist escalation.
//!
//! # Features
//!
//! - Dynamic routing between agents
//! - Content-based handoff decisions
//! - Self-terminating (agent decides when done)
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::patterns::{handoff, HandoffCapable};
//!
//! // Create specialists that can hand off to each other
//! let definition = handoff("support", vec!["general", "code_expert", "math_expert"])?;
//! ```

use serde::Serialize;

use crate::distributed::graph::{ExecutorId, WorkflowBuilder, WorkflowDefinition};

use super::types::{PatternError, PatternResult};

// ============================================================================
// HANDOFF CAPABLE TRAIT
// ============================================================================

/// Trait for executors that can participate in handoff patterns.
///
/// Implementors decide whether to handle a message themselves or hand off
/// to another specialist.
///
/// # Example
///
/// ```rust,ignore
/// struct CodeSpecialist {
///     id: ExecutorId,
/// }
///
/// impl HandoffCapable for CodeSpecialist {
///     fn executor_id(&self) -> &ExecutorId {
///         &self.id
///     }
///
///     fn can_handle(&self, content: &str) -> bool {
///         content.to_lowercase().contains("code") ||
///         content.to_lowercase().contains("programming")
///     }
///
///     fn suggest_handoff(&self, content: &str) -> Option<ExecutorId> {
///         if content.to_lowercase().contains("math") {
///             Some(ExecutorId::new("math_specialist"))
///         } else {
///             None
///         }
///     }
/// }
/// ```
pub trait HandoffCapable: Send + Sync {
    /// Get this executor's ID.
    fn executor_id(&self) -> &ExecutorId;

    /// Check if this executor can handle the given content.
    ///
    /// Returns `true` if this specialist is appropriate for the content.
    fn can_handle(&self, content: &str) -> bool;

    /// Suggest which executor should handle this content instead.
    ///
    /// Returns `Some(id)` if another specialist should handle it,
    /// or `None` if this executor should handle it or terminate.
    fn suggest_handoff(&self, content: &str) -> Option<ExecutorId>;

    /// Get a description of this executor's specialty.
    fn description(&self) -> Option<&str> {
        None
    }
}

// ============================================================================
// HANDOFF DECISION
// ============================================================================

/// Decision made by a handoff-capable executor.
#[derive(Debug, Clone)]
pub enum HandoffDecision {
    /// Handle the request (no handoff).
    Handle,
    /// Hand off to another executor.
    HandoffTo(ExecutorId),
    /// Terminate the workflow (done).
    Terminate,
}

impl HandoffDecision {
    /// Check if this is a handoff.
    pub fn is_handoff(&self) -> bool {
        matches!(self, HandoffDecision::HandoffTo(_))
    }

    /// Check if this is a termination.
    pub fn is_terminate(&self) -> bool {
        matches!(self, HandoffDecision::Terminate)
    }

    /// Get the target executor ID if this is a handoff.
    pub fn handoff_target(&self) -> Option<&ExecutorId> {
        match self {
            HandoffDecision::HandoffTo(id) => Some(id),
            _ => None,
        }
    }
}

// ============================================================================
// HANDOFF BUILDER
// ============================================================================

/// Builder for handoff (dynamic mesh) workflows.
///
/// Creates a workflow where any participant can hand off to any other,
/// based on content-aware routing.
///
/// ```text
///        ┌──────────────────────────────────────┐
///        │                                      │
///        ▼                                      │
///   ┌────────┐    handoff    ┌────────┐         │
///   │General │──────────────▶│ Code   │─────────┤
///   │ Agent  │◀──────────────│Specialist│        │
///   └────────┘    handoff    └────────┘         │
///        │                        │             │
///        │     ┌────────┐         │             │
///        └────▶│  Math  │◀────────┘             │
///              │Specialist│                      │
///              └────┬─────┘                      │
///                   │                            │
///                   └────────────────────────────┘
/// ```
///
/// # Type Parameter
///
/// - `T`: Message type flowing through the workflow (default: `serde_json::Value`)
pub struct HandoffBuilder<T: Send + Sync + 'static = serde_json::Value> {
    /// Workflow ID.
    id: String,
    /// Human-readable name.
    name: Option<String>,
    /// Participant executor IDs.
    participants: Vec<ExecutorId>,
    /// Starting executor (first to receive input).
    start: Option<ExecutorId>,
    /// Phantom for message type.
    _marker: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + Serialize + 'static> HandoffBuilder<T> {
    /// Create a new handoff workflow builder.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            participants: Vec::new(),
            start: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// Set a human-readable name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a participant to the handoff mesh.
    pub fn add_participant(mut self, executor_id: impl Into<ExecutorId>) -> Self {
        self.participants.push(executor_id.into());
        self
    }

    /// Set all participants at once.
    pub fn participants(mut self, executor_ids: impl IntoIterator<Item = impl Into<ExecutorId>>) -> Self {
        self.participants = executor_ids.into_iter().map(std::convert::Into::into).collect();
        self
    }

    /// Set the starting executor (first to receive input).
    ///
    /// If not set, the first participant is used.
    pub fn start(mut self, executor_id: impl Into<ExecutorId>) -> Self {
        self.start = Some(executor_id.into());
        self
    }

    /// Get the number of participants.
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    /// Build the workflow definition.
    ///
    /// Creates a mesh where every participant can route to every other
    /// participant via conditional edges.
    ///
    /// # Errors
    ///
    /// Returns an error if no participants were added.
    pub fn build(self) -> PatternResult<WorkflowDefinition<T>> {
        if self.participants.is_empty() {
            return Err(PatternError::NoParticipants);
        }

        // Determine start executor
        let start = self
            .start
            .unwrap_or_else(|| self.participants[0].clone());

        if !self.participants.contains(&start) {
            return Err(PatternError::InvalidConfiguration(format!(
                "Start executor '{}' not in participants",
                start
            )));
        }

        // Build workflow
        let mut builder = WorkflowBuilder::<T>::new(&self.id);

        if let Some(name) = self.name {
            builder = builder.name(name);
        }

        // Set start
        builder = builder.set_start(start);

        // Add all participants
        for participant in &self.participants {
            builder = builder.add_executor_id(participant.clone());
        }

        // Create mesh: every participant can route to every other
        // Using conditional edges that check for handoff targets
        for from in &self.participants {
            for to in &self.participants {
                if from != to {
                    // Add conditional edge: from → to if handoff decision targets 'to'
                    // The condition function checks if the message contains handoff metadata
                    let to_id = to.clone();
                    builder = builder.add_conditional_edge(
                        from.clone(),
                        to.clone(),
                        move |msg: &T| {
                            // Check if message has handoff target matching 'to'
                            // This assumes T is serde_json::Value or similar
                            if let Some(obj) = serde_json::to_value(msg)
                                .ok()
                                .and_then(|v| v.as_object().cloned())
                            {
                                if let Some(target) = obj.get("handoff_target") {
                                    return target.as_str() == Some(to_id.as_str());
                                }
                            }
                            false
                        },
                        format!("handoff_to_{}", to.as_str()),
                    );
                }
            }
        }

        builder
            .build()
            .map_err(|e| PatternError::WorkflowBuildError(e.to_string()))
    }
}

// ============================================================================
// CONVENIENCE FUNCTION
// ============================================================================

/// Create a handoff workflow from a list of executor IDs.
///
/// The first executor in the list is the starting point.
///
/// # Example
///
/// ```rust,ignore
/// let definition = handoff("support", vec!["general", "code_expert", "math_expert"])?;
/// ```
pub fn handoff<T: Send + Sync + Serialize + 'static>(
    id: impl Into<String>,
    executor_ids: impl IntoIterator<Item = impl Into<ExecutorId>>,
) -> PatternResult<WorkflowDefinition<T>> {
    HandoffBuilder::new(id)
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
    fn test_handoff_decision() {
        let handle = HandoffDecision::Handle;
        assert!(!handle.is_handoff());
        assert!(!handle.is_terminate());

        let handoff = HandoffDecision::HandoffTo(ExecutorId::new("expert"));
        assert!(handoff.is_handoff());
        assert_eq!(handoff.handoff_target().unwrap().as_str(), "expert");

        let terminate = HandoffDecision::Terminate;
        assert!(terminate.is_terminate());
    }

    #[test]
    fn test_handoff_builder_new() {
        let builder = HandoffBuilder::<serde_json::Value>::new("test");
        assert_eq!(builder.participant_count(), 0);
    }

    #[test]
    fn test_handoff_builder_build_empty() {
        let result = HandoffBuilder::<serde_json::Value>::new("test").build();
        assert!(matches!(result, Err(PatternError::NoParticipants)));
    }

    #[test]
    fn test_handoff_builder_build_single() {
        let definition = HandoffBuilder::<serde_json::Value>::new("test")
            .add_participant("single")
            .build()
            .unwrap();

        assert_eq!(definition.executors.len(), 1);
        assert_eq!(definition.start.as_str(), "single");
        // No edges (only one participant)
        assert!(definition.edges.all().is_empty());
    }

    #[test]
    fn test_handoff_builder_build_mesh() {
        let definition = HandoffBuilder::<serde_json::Value>::new("support")
            .name("Customer Support")
            .add_participant("general")
            .add_participant("code_expert")
            .add_participant("math_expert")
            .build()
            .unwrap();

        // Check executors
        assert_eq!(definition.executors.len(), 3);
        assert!(definition.has_executor(&ExecutorId::new("general")));
        assert!(definition.has_executor(&ExecutorId::new("code_expert")));
        assert!(definition.has_executor(&ExecutorId::new("math_expert")));

        // Check start
        assert_eq!(definition.start.as_str(), "general");

        // Check edges: each can route to 2 others = 3 * 2 = 6 conditional edges
        let edges = definition.edges.all();
        assert_eq!(edges.len(), 6);
    }

    #[test]
    fn test_handoff_builder_custom_start() {
        let definition = HandoffBuilder::<serde_json::Value>::new("test")
            .participants(vec!["a", "b", "c"])
            .start("b")
            .build()
            .unwrap();

        assert_eq!(definition.start.as_str(), "b");
    }

    #[test]
    fn test_handoff_builder_invalid_start() {
        let result = HandoffBuilder::<serde_json::Value>::new("test")
            .participants(vec!["a", "b"])
            .start("unknown")
            .build();

        assert!(matches!(result, Err(PatternError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_handoff_convenience_function() {
        let definition = handoff::<serde_json::Value>(
            "quick",
            vec!["x", "y", "z"],
        )
        .unwrap();

        assert_eq!(definition.executors.len(), 3);
        assert_eq!(definition.start.as_str(), "x");
    }
}
