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

mod builder;
mod context;
mod edge;
mod executor;
#[cfg(test)]
mod integration_test;
mod persistence;
mod superstep;
mod types;
mod workflow;

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

// PR #3: Edge Types
pub use edge::{
    AggregationFn, ConditionFn, Edge, EdgeCollection, EdgeId, EdgeKind, EdgeSource, EdgeTarget,
    SelectionFn, SwitchFn,
};

// PR #4: WorkflowBuilder
pub use builder::{
    validate_workflow, ExecutorMetadata, ValidationReport, WorkflowBuilder, WorkflowDefinition,
};

// PR #5: Superstep Execution
pub use superstep::{
    Checkpoint, CheckpointBackend, ExecutorSuperstepResult, InMemoryCheckpointBackend,
    SuperstepMetrics, SuperstepState,
};
pub use workflow::{
    DynExecutor, ExecutorRegistry, Workflow, WorkflowConfig, WorkflowResult, WorkflowStreamEvent,
};

// PR #6: Graph Persistence
pub use persistence::{
    CheckpointMetadata, InMemoryPersistentBackend, PersistentCheckpoint, PersistentCheckpointBackend,
};
#[cfg(feature = "redis-persistence")]
pub use persistence::RedisCheckpointBackend;
