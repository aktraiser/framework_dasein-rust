//! Reusable Graph Executors - MAF-style Dynamic Dispatch
//!
//! This module provides executors that use **dynamic dispatch** via `serde_json::Value`,
//! matching the Microsoft Agent Framework (MAF) pattern. This allows mixing any
//! executors in a single workflow without type conflicts.
//!
//! # MAF Pattern
//!
//! In MAF (Python), executors can handle multiple message types dynamically:
//! ```python
//! class SampleExecutor(Executor):
//!     @handler
//!     async def handle_str(self, text: str, ctx: WorkflowContext[str]): ...
//!     @handler
//!     async def handle_int(self, num: int, ctx: WorkflowContext[int]): ...
//! ```
//!
//! In Rust, we achieve this with `serde_json::Value`:
//! - All executors use `Input = serde_json::Value`
//! - All executors use `Message = serde_json::Value`
//! - Deserialization happens inside each handler
//!
//! # Available Executors
//!
//! | Executor | Kind | Purpose |
//! |----------|------|---------|
//! | [`LLMGeneratorExecutor`] | Worker | Code generation via LLM |
//! | [`CodeAssemblerExecutor`] | Worker | Merge code fragments |
//! | [`CompileValidatorExecutor`] | Validator | Compile-time validation |
//! | [`TestValidatorExecutor`] | Validator | Test execution |
//! | [`SubWorkflowExecutor`] | Orchestrator | Nested sub-workflows |
//!
//! # Example: Building a Mixed Pipeline
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::executors::*;
//! use dasein_agentic_core::distributed::graph::{WorkflowBuilder, ExecutorRegistry, Workflow};
//!
//! // All executors use Value - they can be mixed freely!
//! let mut registry: ExecutorRegistry<serde_json::Value, String> = ExecutorRegistry::new();
//!
//! registry.register(LLMGeneratorExecutor::new("generator", llm_executor));
//! registry.register(CompileValidatorExecutor::new("compiler", sandbox.clone()));
//! registry.register(TestValidatorExecutor::new("tester", sandbox));
//! registry.register(CodeAssemblerExecutor::new("assembler"));
//!
//! // Build workflow - any executor can connect to any other
//! let definition = WorkflowBuilder::<serde_json::Value>::new("pipeline")
//!     .set_start("generator")
//!     .add_direct_edge("generator", "compiler")
//!     .add_conditional_edge("compiler", "tester", |v| {
//!         v.get("passed").and_then(|p| p.as_bool()).unwrap_or(false)
//!     }, "on_compile_success")
//!     .add_direct_edge("tester", "assembler")
//!     .build()?;
//!
//! let workflow = Workflow::new(definition, registry);
//! let result = workflow.run(json!({"prompt": "Write fibonacci", "language": "rust"})).await?;
//! ```
//!
//! # Input/Output Conventions
//!
//! Each executor documents its expected input JSON schema and output schema.
//! Use the helper types (e.g., `GeneratorInput`, `CompileInput`) for type-safe
//! construction, then serialize to `serde_json::Value`.

mod code_assembler;
mod compile_validator;
mod llm_generator;
mod sub_workflow;
mod test_validator;

pub use code_assembler::{AssembledCode, AssemblyInput, CodeAssemblerExecutor, CodePart};
pub use compile_validator::{CompileInput, CompileOutput, CompileValidatorExecutor};
pub use llm_generator::{GeneratedCode, GeneratorInput, LLMGeneratorExecutor};
pub use sub_workflow::{SubWorkflowExecutor, SubWorkflowInput, SubWorkflowOutput};
pub use test_validator::{TestInput, TestOutput, TestValidatorExecutor};

// Re-export common types for convenience
pub use super::{ExecutorError, ExecutorId, ExecutorKind, ValidationResult};
