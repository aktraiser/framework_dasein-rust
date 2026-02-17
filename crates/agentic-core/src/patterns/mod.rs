//! Orchestration Patterns - High-level workflow construction patterns.
//!
//! This module provides builder patterns for common multi-agent orchestrations,
//! inspired by [Microsoft Agent Framework](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/orchestrations/).
//!
//! # Available Patterns
//!
//! | Pattern | Description | Use Case |
//! |---------|-------------|----------|
//! | [`SequentialBuilder`] | Pipeline: A → B → C | Dependent steps |
//! | [`ConcurrentBuilder`] | Fan-out/fan-in | Independent parallel tasks |
//! | [`GroupChatBuilder`] | Star topology with orchestrator | Collaborative iteration |
//! | [`handoff`] | Dynamic mesh routing | Specialist escalation |
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use dasein_agentic_core::patterns::{SequentialBuilder, ConcurrentBuilder};
//!
//! // Sequential: writer → reviewer → polisher
//! let workflow = SequentialBuilder::new("content-pipeline")
//!     .add_participant("writer")
//!     .add_participant("reviewer")
//!     .add_participant("polisher")
//!     .build()?;
//!
//! // Concurrent: researcher, marketer, legal → aggregator
//! let workflow = ConcurrentBuilder::new("analysis")
//!     .add_participant("researcher")
//!     .add_participant("marketer")
//!     .add_participant("legal")
//!     .build()?;
//! ```
//!
//! # Pattern Selection Guide
//!
//! | Scenario | Recommended Pattern |
//! |----------|---------------------|
//! | Steps depend on previous result | [`SequentialBuilder`] |
//! | Independent tasks, combine results | [`ConcurrentBuilder`] |
//! | Multi-turn discussion/debate | [`GroupChatBuilder`] |
//! | Expert routing based on content | [`handoff`] |

mod concurrent;
mod group_chat;
mod handoff;
mod sequential;
mod types;

// Re-export all patterns
pub use concurrent::{concurrent, ConcurrentBuilder};
pub use group_chat::{GroupChatBuilder, GroupChatState, SelectionFunc, TerminationFunc};
pub use handoff::{handoff, HandoffBuilder, HandoffCapable, HandoffDecision};
pub use sequential::{sequential, SequentialBuilder};
pub use types::{AggregatedResult, AggregatorFn, ParticipantResult, PatternError, PatternResult};

// Re-export selectors and termination conditions from group_chat
pub use group_chat::{
    any_termination, keyword_termination, max_messages_termination, max_rounds_termination,
    no_repeat_selector, round_robin_selector, smart_selector,
};
