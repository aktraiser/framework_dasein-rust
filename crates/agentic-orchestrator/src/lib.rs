//! # agentic-orchestrator
//!
//! Multi-agent orchestration and workflow execution.
//!
//! Provides:
//! - Agent registration and discovery
//! - Workflow execution with dependencies
//! - Parallel task execution

mod error;
mod orchestrator;
mod workflow;

pub use error::OrchestratorError;
pub use orchestrator::{AgentRef, Orchestrator};
pub use workflow::{Workflow, WorkflowResult, WorkflowStep};
