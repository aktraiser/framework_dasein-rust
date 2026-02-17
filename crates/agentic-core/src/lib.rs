//! # agentic-core
//!
//! Core components for the Agentic Framework.
//!
//! This crate provides:
//! - [`Agent`] - The autonomous entity that processes tasks
//! - [`Supervisor`] - Orchestrates executors for distributed systems
//! - [`Executor`] - Worker that processes tasks with LLM
//! - [`Validator`] - Rule-based output validation (fast, heuristic)
//! - [`SandboxValidator`] - Ground truth validation via real execution (for code)
//! - Protocol types for inter-agent communication
//!
//! # Two Validation Strategies
//!
//! | Strategy | Use Case | How it Works |
//! |----------|----------|--------------|
//! | [`Validator`] | Text, JSON, quick checks | Rule-based heuristics |
//! | [`SandboxValidator`] | Code generation | Real `cargo test` execution |
//!
//! ## Quick Start (Distributed Mode)
//!
//! ```rust,no_run
//! use dasein_agentic_core::distributed::{Supervisor, Executor, Validator};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a supervisor with 4 executors
//!     let sup = Supervisor::new("sup-code")
//!         .domain("code")
//!         .executors(4)
//!         .llm_gemini("gemini-2.0-flash")
//!         .allow_lending(true)
//!         .build_async()
//!         .await;
//!
//!     println!("Pool size: {}", sup.pool_size().await);
//!     println!("Idle: {}", sup.idle_count().await);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Even Simpler
//!
//! ```rust,no_run
//! use dasein_agentic_core::distributed::Supervisor;
//!
//! // One-liner
//! let config = Supervisor::quick("my-sup", 4).build();
//! ```

pub mod agent;
pub mod distributed;
pub mod error;
pub mod patterns;
pub mod prelude;
pub mod protocol;
pub mod types;

pub use agent::Agent;
pub use error::AgentError;
pub use types::{
    AgentConfig, AgentMetrics, AgentState, AgentStatus, Artifact, ArtifactType, ExecutionMode,
    ResultMetrics, ResultPayload, ResultStatus, TaskPayload,
};

// Re-export distributed types for convenience
pub use distributed::{
    AllocationGrant, AllocationManager, AllocationRequest, Capability, Executor, ExecutorBuilder,
    ExecutorPool, ExecutorState, LLMConfig, Language, PoolConfig, SandboxConfig,
    SandboxValidationResult, SandboxValidator, Supervisor, SupervisorBuilder, SupervisorHandle,
    ValidationAction, ValidationResult, ValidationRule, Validator, ValidatorBuilder,
};
