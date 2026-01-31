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

mod supervisor;
mod executor;
mod validator;
mod sandbox_validator;
mod sandbox_pipeline_validator;
mod validator_pipeline;
mod mcp_doc_validator;
mod error_enricher_validator;
mod task_decomposer;
mod code_assembler;
mod liaison_architect;
mod config;
mod pool;
mod allocation;
pub mod bus;
pub mod graph;
pub mod incremental_pipeline;
pub mod repair_engine;

pub use supervisor::{Supervisor, SupervisorBuilder, SupervisorHandle};
pub use executor::{Executor, ExecutorBuilder, ExecutorState, ExecutionResult, ExecutionError};
pub use validator::{Validator, ValidatorBuilder, ValidationRule, ValidationResult, ValidationAction};
pub use sandbox_validator::{SandboxValidator, SandboxValidationResult, Language};
pub use sandbox_pipeline_validator::SandboxPipelineValidator;
pub use validator_pipeline::{
    ValidatorPipeline, PipelineValidator, ValidatorInput, ValidatorOutput,
    PipelineResult, DocSnippet
};
pub use mcp_doc_validator::{MCPDocValidator, MCPDocConfig};
pub use error_enricher_validator::ErrorEnricherValidator;
pub use task_decomposer::{TaskDecomposer, SubTask, SubTaskResult};
pub use code_assembler::{CodeAssembler, AssemblyError};
pub use liaison_architect::{LiaisonArchitect, CoherenceReport, CoherenceIssue, IssueCategory, LiaisonError};
pub use config::*;
pub use pool::{ExecutorPool, PoolConfig};
pub use allocation::{AllocationRequest, AllocationGrant, AllocationManager};
