//! Distributed agent architecture - Supervisor/Executor/Validator pattern.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │  Supervisor │  Orchestrates executors and validators
//! └──────┬──────┘
//!        │
//!    ┌───┴───┐
//!    ▼       ▼
//! ┌──────┐ ┌──────────┐
//! │Executor│ │Validator │  Two validation strategies available
//! └──────┘ └──────────┘
//! ```
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use agentic_core::distributed::Supervisor;
//!
//! // Create a supervisor with 4 executors
//! let sup = Supervisor::new("my-sup")
//!     .executors(4)
//!     .llm_gemini("gemini-2.0-flash")
//!     .build_async()
//!     .await;
//! ```
//!
//! # Two Validation Strategies
//!
//! ## 1. Validator (Rule-based, fast)
//!
//! Use for quick heuristic checks on any output type:
//!
//! ```rust
//! use agentic_core::distributed::{Validator, ValidationRule};
//!
//! let validator = Validator::new("val-001", "sup-001")
//!     .rule(ValidationRule::OutputNotEmpty)
//!     .rule(ValidationRule::NoErrors)
//!     .rule(ValidationRule::NoSecrets)
//!     .build();
//!
//! let result = validator.validate("generated output", 0);
//! if !result.passed {
//!     println!("Feedback: {:?}", result.feedback);
//! }
//! ```
//!
//! ## 2. SandboxValidator (Ground truth, for code)
//!
//! Use for real compilation and test execution:
//!
//! ```rust,no_run
//! use agentic_core::distributed::SandboxValidator;
//! use agentic_sandbox::ProcessSandbox;
//!
//! let sandbox = ProcessSandbox::new();
//! let validator = SandboxValidator::new(sandbox);
//!
//! let result = validator.validate_rust_code(code).await?;
//! if !result.passed {
//!     // Real compiler errors, not LLM guesses
//!     println!("Errors: {:?}", result.compiler_errors);
//! }
//! ```
//!
//! # When to Use Which
//!
//! | Task Type | Validator | SandboxValidator |
//! |-----------|-----------|------------------|
//! | Code generation | ❌ | ✅ Real compile/test |
//! | Text/docs | ✅ Rules | ❌ |
//! | JSON/config | ✅ ValidJson | ❌ |
//! | Scripts | ✅ Quick check | ✅ Execute |
//!
//! # Grounded Validation Loop
//!
//! For code generation, use the grounded loop pattern:
//!
//! ```text
//! Executor (LLM) ──► SandboxValidator ──► Real errors ──┐
//!      ▲                                                │
//!      └────────────── Feedback ◄───────────────────────┘
//! ```
//!
//! See `examples/grounded_loop.rs` for a complete implementation.

mod allocation;
pub mod bus;
mod code_assembler;
mod config;
mod error_enricher_validator;
mod executor;
pub mod graph;
pub mod incremental_pipeline;
mod liaison_architect;
mod mcp_doc_validator;
mod pool;
pub mod repair_engine;
mod sandbox_pipeline_validator;
mod sandbox_validator;
mod supervisor;
mod task_decomposer;
mod validator;
mod validator_pipeline;

pub use allocation::{AllocationGrant, AllocationManager, AllocationRequest};
pub use code_assembler::{AssemblyError, CodeAssembler};
pub use config::*;
pub use error_enricher_validator::ErrorEnricherValidator;
pub use executor::{ExecutionError, ExecutionResult, Executor, ExecutorBuilder, ExecutorState};
pub use liaison_architect::{
    CoherenceIssue, CoherenceReport, IssueCategory, LiaisonArchitect, LiaisonError,
};
pub use mcp_doc_validator::{MCPDocConfig, MCPDocValidator};
pub use pool::{ExecutorPool, PoolConfig};
pub use sandbox_pipeline_validator::SandboxPipelineValidator;
pub use sandbox_validator::{Language, SandboxValidationResult, SandboxValidator};
pub use supervisor::{Supervisor, SupervisorBuilder, SupervisorHandle};
pub use task_decomposer::{SubTask, SubTaskResult, TaskDecomposer};
pub use validator::{
    ValidationAction, ValidationResult, ValidationRule, Validator, ValidatorBuilder,
};
pub use validator_pipeline::{
    DocSnippet, PipelineResult, PipelineValidator, ValidatorInput, ValidatorOutput,
    ValidatorPipeline,
};
