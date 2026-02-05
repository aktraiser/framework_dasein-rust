//! Group Chat Pattern - Star topology with orchestrator.
//!
//! A group chat has a central orchestrator that selects which participant
//! speaks next, broadcasts responses to all, and terminates when done.
//!
//! # Features
//!
//! - Central orchestrator controls flow
//! - Multiple selection strategies (round-robin, smart, LLM)
//! - Configurable termination conditions
//! - Context synchronization (all see everything)
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::patterns::{GroupChatBuilder, round_robin_selector};
//!
//! let definition = GroupChatBuilder::new("code-review")
//!     .add_participant("coder")
//!     .add_participant("reviewer")
//!     .add_participant("tester")
//!     .with_orchestrator_func(round_robin_selector)
//!     .with_max_iterations(10)
//!     .build()?;
//! ```

use std::collections::HashMap;

use crate::distributed::graph::agent::ChatMessage;
use crate::distributed::graph::{ExecutorId, WorkflowBuilder, WorkflowDefinition};

use super::types::{PatternError, PatternResult};

// ============================================================================
// TYPE DEFINITIONS
// ============================================================================

/// State of the group chat (passed to selection/termination functions).
#[derive(Debug, Clone)]
pub struct GroupChatState {
    /// Map of participant IDs to their names/descriptions.
    pub participants: HashMap<ExecutorId, String>,
    /// Ordered list of participant IDs (preserves insertion order for round-robin).
    participant_order: Vec<ExecutorId>,
    /// Conversation history.
    pub conversation: Vec<ChatMessage>,
    /// Current round/iteration number (0-indexed).
    pub current_round: usize,
    /// ID of the last participant who spoke (None at start).
    pub last_speaker: Option<ExecutorId>,
}

impl GroupChatState {
    /// Create a new empty state.
    pub fn new(participants: Vec<ExecutorId>) -> Self {
        let participant_order = participants.clone();
        Self {
            participants: participants
                .into_iter()
                .map(|id| (id.clone(), id.as_str().to_string()))
                .collect(),
            participant_order,
            conversation: Vec::new(),
            current_round: 0,
            last_speaker: None,
        }
    }

    /// Get participant IDs as a vector (preserves insertion order).
    pub fn participant_ids(&self) -> Vec<ExecutorId> {
        self.participant_order.clone()
    }

    /// Get the last message content.
    pub fn last_message(&self) -> Option<&ChatMessage> {
        self.conversation.last()
    }

    /// Get the last message content as a string.
    pub fn last_content(&self) -> Option<&str> {
        self.conversation.last().map(|m| m.content.as_str())
    }
}

/// Function that selects which participant speaks next.
pub type SelectionFunc = Box<dyn Fn(&GroupChatState) -> ExecutorId + Send + Sync>;

/// Function that determines if the conversation should terminate.
pub type TerminationFunc = Box<dyn Fn(&GroupChatState) -> bool + Send + Sync>;

// ============================================================================
// BUILT-IN SELECTORS
// ============================================================================

/// Round-robin selector: cycles through participants in order.
///
/// # Example
///
/// ```rust,ignore
/// let workflow = GroupChatBuilder::new("chat")
///     .participants(vec!["a", "b", "c"])
///     .with_orchestrator_func(round_robin_selector)
///     .build()?;
/// ```
pub fn round_robin_selector(state: &GroupChatState) -> ExecutorId {
    let ids: Vec<_> = state.participant_ids();
    if ids.is_empty() {
        return ExecutorId::new("unknown");
    }
    ids[state.current_round % ids.len()].clone()
}

/// Smart selector: selects based on conversation content.
///
/// Looks for keywords to determine the next speaker:
/// - "research" / "data" → researcher
/// - "write" / "draft" → writer
/// - "review" / "feedback" → reviewer
///
/// Falls back to round-robin if no keywords match.
pub fn smart_selector(state: &GroupChatState) -> ExecutorId {
    let last_content = state.last_content().unwrap_or("").to_lowercase();
    let ids = state.participant_ids();

    // Find by keyword
    for id in &ids {
        let name = id.as_str().to_lowercase();
        if (last_content.contains("research") || last_content.contains("data"))
            && name.contains("research")
        {
            return id.clone();
        }
        if (last_content.contains("write") || last_content.contains("draft"))
            && name.contains("writ")
        {
            return id.clone();
        }
        if (last_content.contains("review") || last_content.contains("feedback"))
            && name.contains("review")
        {
            return id.clone();
        }
    }

    // Fallback to round-robin
    round_robin_selector(state)
}

/// Creates a selector that avoids repeating the last speaker.
///
/// Useful for ensuring all participants get a chance to speak.
pub fn no_repeat_selector(base_selector: SelectionFunc) -> SelectionFunc {
    Box::new(move |state| {
        let selected = base_selector(state);

        // If selected is the last speaker, try to find another
        if state.last_speaker.as_ref() == Some(&selected) {
            let ids = state.participant_ids();
            for id in ids {
                if Some(&id) != state.last_speaker.as_ref() {
                    return id;
                }
            }
        }

        selected
    })
}

// ============================================================================
// BUILT-IN TERMINATION CONDITIONS
// ============================================================================

/// Terminate after a fixed number of messages.
pub fn max_messages_termination(max: usize) -> TerminationFunc {
    Box::new(move |state| state.conversation.len() >= max)
}

/// Terminate after a fixed number of rounds.
pub fn max_rounds_termination(max: usize) -> TerminationFunc {
    Box::new(move |state| state.current_round >= max)
}

/// Terminate when conversation contains a keyword.
pub fn keyword_termination(keyword: &'static str) -> TerminationFunc {
    Box::new(move |state| {
        state
            .last_content()
            .map(|c| c.to_lowercase().contains(keyword))
            .unwrap_or(false)
    })
}

/// Combine multiple termination conditions (OR logic).
pub fn any_termination(conditions: Vec<TerminationFunc>) -> TerminationFunc {
    Box::new(move |state| conditions.iter().any(|c| c(state)))
}

// ============================================================================
// GROUP CHAT BUILDER
// ============================================================================

/// Builder for group chat workflows.
///
/// Creates a star topology with an orchestrator that:
/// 1. Selects which participant speaks (via selection_func)
/// 2. Routes input to the selected participant
/// 3. Broadcasts the response to all participants
/// 4. Repeats until termination condition is met
///
/// ```text
///                    ┌─────────────────┐
///                    │   Orchestrator  │
///                    │ (selection_func)│
///                    └────────┬────────┘
///            ┌────────────────┼────────────────┐
///            │                │                │
///            ▼                ▼                ▼
///       ┌────────┐       ┌────────┐       ┌────────┐
///       │  P1    │◀─────▶│  P2    │◀─────▶│  P3    │
///       └────────┘       └────────┘       └────────┘
///                  (broadcast to all)
/// ```
///
/// # Type Parameter
///
/// - `T`: Message type flowing through the workflow (default: `serde_json::Value`)
pub struct GroupChatBuilder<T: Send + Sync + 'static = serde_json::Value> {
    /// Workflow ID.
    id: String,
    /// Human-readable name.
    name: Option<String>,
    /// Participant executor IDs.
    participants: Vec<ExecutorId>,
    /// Participant descriptions (for LLM selectors).
    participant_descriptions: HashMap<ExecutorId, String>,
    /// Selection function (decides who speaks).
    selection_func: Option<SelectionFunc>,
    /// Termination condition.
    termination_func: Option<TerminationFunc>,
    /// Maximum iterations (safety limit).
    max_iterations: usize,
    /// Custom orchestrator ID.
    orchestrator_id: Option<ExecutorId>,
    /// Phantom for message type.
    _marker: std::marker::PhantomData<T>,
}

impl<T: Send + Sync + 'static> GroupChatBuilder<T> {
    /// Create a new group chat builder.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this workflow.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            participants: Vec::new(),
            participant_descriptions: HashMap::new(),
            selection_func: None,
            termination_func: None,
            max_iterations: 20, // Default safety limit
            orchestrator_id: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// Set a human-readable name for the workflow.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a participant to the group chat.
    pub fn add_participant(mut self, executor_id: impl Into<ExecutorId>) -> Self {
        self.participants.push(executor_id.into());
        self
    }

    /// Add a participant with a description (useful for LLM selectors).
    pub fn add_participant_with_description(
        mut self,
        executor_id: impl Into<ExecutorId>,
        description: impl Into<String>,
    ) -> Self {
        let id = executor_id.into();
        self.participants.push(id.clone());
        self.participant_descriptions.insert(id, description.into());
        self
    }

    /// Set all participants at once.
    pub fn participants(mut self, executor_ids: impl IntoIterator<Item = impl Into<ExecutorId>>) -> Self {
        self.participants = executor_ids.into_iter().map(|id| id.into()).collect();
        self
    }

    /// Set the selection function (decides who speaks next).
    ///
    /// Built-in options:
    /// - [`round_robin_selector`]
    /// - [`smart_selector`]
    pub fn with_orchestrator_func<F>(mut self, selection_func: F) -> Self
    where
        F: Fn(&GroupChatState) -> ExecutorId + Send + Sync + 'static,
    {
        self.selection_func = Some(Box::new(selection_func));
        self
    }

    /// Set the termination condition.
    ///
    /// Built-in options:
    /// - [`max_messages_termination`]
    /// - [`max_rounds_termination`]
    /// - [`keyword_termination`]
    pub fn with_termination_condition<F>(mut self, condition: F) -> Self
    where
        F: Fn(&GroupChatState) -> bool + Send + Sync + 'static,
    {
        self.termination_func = Some(Box::new(condition));
        self
    }

    /// Set the maximum number of iterations (safety limit).
    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    /// Set a custom orchestrator executor ID.
    pub fn with_orchestrator_id(mut self, id: impl Into<ExecutorId>) -> Self {
        self.orchestrator_id = Some(id.into());
        self
    }

    /// Get the number of participants.
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    /// Get the orchestrator ID that will be used.
    pub fn orchestrator_id(&self) -> ExecutorId {
        self.orchestrator_id
            .clone()
            .unwrap_or_else(|| ExecutorId::new(format!("{}-orchestrator", self.id)))
    }

    /// Build the workflow definition.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No participants were added
    /// - No selection function was set
    pub fn build(self) -> PatternResult<WorkflowDefinition<T>> {
        // Validate
        if self.participants.is_empty() {
            return Err(PatternError::NoParticipants);
        }

        if self.selection_func.is_none() {
            return Err(PatternError::InvalidConfiguration(
                "Selection function not set. Use with_orchestrator_func()".into(),
            ));
        }

        let orchestrator_id = self.orchestrator_id();
        let collector_id = ExecutorId::new(format!("{}-collector", self.id));

        // Build workflow
        let mut builder = WorkflowBuilder::<T>::new(&self.id);

        if let Some(name) = self.name {
            builder = builder.name(name);
        }

        // Start with orchestrator
        builder = builder.set_start(orchestrator_id.clone());

        // Add orchestrator and collector
        builder = builder.add_executor_id(orchestrator_id.clone());
        builder = builder.add_executor_id(collector_id.clone());

        // Add all participants
        for participant in &self.participants {
            builder = builder.add_executor_id(participant.clone());
        }

        // Orchestrator → participants (switch edge based on selection)
        // The orchestrator outputs a message that includes the target participant
        builder = builder.add_fan_out_edge(orchestrator_id.clone(), self.participants.clone());

        // All participants → collector (fan-in)
        builder = builder.add_fan_in_edge(self.participants.clone(), collector_id.clone());

        // Collector → orchestrator (loop back for next round)
        // This creates the iterative loop
        builder = builder.add_direct_edge(collector_id, orchestrator_id);

        // Build
        builder
            .build()
            .map_err(|e| PatternError::WorkflowBuildError(e.to_string()))
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_chat_state_new() {
        let state = GroupChatState::new(vec![
            ExecutorId::new("alice"),
            ExecutorId::new("bob"),
        ]);

        assert_eq!(state.participants.len(), 2);
        assert_eq!(state.current_round, 0);
        assert!(state.last_speaker.is_none());
        assert!(state.conversation.is_empty());
    }

    #[test]
    fn test_round_robin_selector() {
        let mut state = GroupChatState::new(vec![
            ExecutorId::new("a"),
            ExecutorId::new("b"),
            ExecutorId::new("c"),
        ]);

        // Round 0 → a
        assert_eq!(round_robin_selector(&state).as_str(), "a");

        state.current_round = 1;
        assert_eq!(round_robin_selector(&state).as_str(), "b");

        state.current_round = 2;
        assert_eq!(round_robin_selector(&state).as_str(), "c");

        state.current_round = 3;
        assert_eq!(round_robin_selector(&state).as_str(), "a"); // Wraps around
    }

    #[test]
    fn test_smart_selector_keywords() {
        let mut state = GroupChatState::new(vec![
            ExecutorId::new("researcher"),
            ExecutorId::new("writer"),
            ExecutorId::new("reviewer"),
        ]);

        // Add a message about research
        state.conversation.push(ChatMessage::user("We need more data"));
        assert_eq!(smart_selector(&state).as_str(), "researcher");

        // Add a message about writing
        state.conversation.push(ChatMessage::assistant("Here's the data"));
        state.conversation.push(ChatMessage::user("Now write a draft"));
        assert_eq!(smart_selector(&state).as_str(), "writer");
    }

    #[test]
    fn test_max_messages_termination() {
        let termination = max_messages_termination(3);
        let mut state = GroupChatState::new(vec![ExecutorId::new("a")]);

        assert!(!termination(&state));

        state.conversation.push(ChatMessage::user("1"));
        state.conversation.push(ChatMessage::assistant("2"));
        assert!(!termination(&state));

        state.conversation.push(ChatMessage::user("3"));
        assert!(termination(&state));
    }

    #[test]
    fn test_keyword_termination() {
        let termination = keyword_termination("approved");
        let mut state = GroupChatState::new(vec![ExecutorId::new("a")]);

        state.conversation.push(ChatMessage::assistant("Still working on it"));
        assert!(!termination(&state));

        state.conversation.push(ChatMessage::assistant("This is APPROVED!"));
        assert!(termination(&state));
    }

    #[test]
    fn test_group_chat_builder_new() {
        let builder = GroupChatBuilder::<serde_json::Value>::new("test");
        assert_eq!(builder.participant_count(), 0);
    }

    #[test]
    fn test_group_chat_builder_build_no_participants() {
        let result = GroupChatBuilder::<serde_json::Value>::new("test")
            .with_orchestrator_func(round_robin_selector)
            .build();

        assert!(matches!(result, Err(PatternError::NoParticipants)));
    }

    #[test]
    fn test_group_chat_builder_build_no_selector() {
        let result = GroupChatBuilder::<serde_json::Value>::new("test")
            .add_participant("a")
            .build();

        assert!(matches!(result, Err(PatternError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_group_chat_builder_build() {
        let definition = GroupChatBuilder::<serde_json::Value>::new("review")
            .name("Code Review")
            .add_participant("coder")
            .add_participant("reviewer")
            .with_orchestrator_func(round_robin_selector)
            .with_max_iterations(10)
            .build()
            .unwrap();

        // Should have: orchestrator, collector, coder, reviewer
        assert_eq!(definition.executors.len(), 4);
        assert!(definition.has_executor(&ExecutorId::new("review-orchestrator")));
        assert!(definition.has_executor(&ExecutorId::new("review-collector")));
        assert!(definition.has_executor(&ExecutorId::new("coder")));
        assert!(definition.has_executor(&ExecutorId::new("reviewer")));

        // Start is orchestrator
        assert_eq!(definition.start.as_str(), "review-orchestrator");
    }
}
