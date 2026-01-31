//! Graph-based Multi-Agent Architecture
//!
//! This module implements a dynamic graph where:
//! - **Executors** are nodes that do work (generate code, validate, assemble)
//! - **Edges** are connections that transport data between executors
//! - **Workflows** orchestrate execution via Supersteps (Pregel/BSP model)
//!
//! # Executor Types
//!
//! | Type | Role | Example |
//! |------|------|---------|
//! | Worker | Does work | Code generation, merging |
//! | Validator | Verifies | Compile, test, lint |
//! | Orchestrator | Coordinates sub-graph | Complex sub-tasks |
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::{Executor, ExecutorKind, ExecutorId};
//!
//! struct MyWorker {
//!     id: ExecutorId,
//! }
//!
//! impl Executor for MyWorker {
//!     type Input = String;
//!     type Message = String;
//!     type Output = ();
//!
//!     fn id(&self) -> &ExecutorId { &self.id }
//!     fn kind(&self) -> ExecutorKind { ExecutorKind::Worker }
//!
//!     async fn handle(&self, input: Self::Input, ctx: &mut impl ExecutorContext<...>) {
//!         ctx.send_message(input.to_uppercase()).await?;
//!         Ok(())
//!     }
//! }
//! ```

mod context;
mod executor;
mod types;

// PR #1: Executor Trait
pub use executor::{Executor, ExecutorContext, ExecutorKind, LogLevel, ValidationResult};
pub use types::{
    EdgeEvent, ErrorCategory, ExecutorError, ExecutorEvent, ExecutorId, GraphError, GraphResult,
    TaskId, WorkflowId,
};

// PR #2: WorkflowContext
pub use context::{
    InMemoryStateBackend, MessageSink, OutputSink, SharedStateBackend, WorkflowContext,
    WorkflowContextBuilder, WorkflowEvent,
};

// Future PRs will add:
// - PR #3: Edge types (Direct, Conditional, Switch, FanOut, FanIn)
// - PR #4: WorkflowBuilder
// - PR #5: Workflow with Superstep execution
