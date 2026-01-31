//! Graph-based Multi-Agent Architecture
//!
//! This module implements a dynamic graph where:
//! - **Executors** are nodes that do work (generate code, validate, assemble)
//! - **Edges** are connections that transport data between executors
//! - **Orchestrator** manages the graph execution
//! - **NATS** provides memory, communication, and history for all components
//!
//! # Architecture
//!
//! ```text
//!                          NATS BUS
//!                    ┌─────────────────┐
//!                    │ Events │ State  │
//!                    └────────┬────────┘
//!                             │
//!        ┌────────────────────┼────────────────────┐
//!        ▼                    ▼                    ▼
//!   Orchestrator         Executors            Validators
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use agentic_core::distributed::graph::{Orchestrator, TaskGraph, GraphExecutor, Edge};
//!
//! let orchestrator = Orchestrator::new(nats_client).await?;
//! let result = orchestrator.run(task).await?;
//! ```

mod executor;
mod edge;
mod graph;
mod orchestrator;
mod types;

pub use executor::{GraphExecutor, ExecutorType, ExecutorState, ExecutorConfig, ExecutorOutput, ExecutorContext};
pub use edge::{Edge, EdgeId, EdgeDataType, EdgeCondition, EdgeData, EdgeMetadata};
pub use graph::{TaskGraph, GraphBuilder};
pub use orchestrator::{Orchestrator, OrchestratorConfig, OrchestratorState, DecisionEngine};
pub use types::{ExecutorId, TaskId, GraphError, GraphResult, ExecutorEvent, EdgeEvent};
