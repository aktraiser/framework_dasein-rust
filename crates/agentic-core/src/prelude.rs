//! Prelude - Import everything you need with one line.
//!
//! # Usage
//!
//! ```rust
//! use agentic_core::prelude::*;
//! ```
//!
//! This imports the most commonly used types for building agents:
//!
//! ## Core Types
//! - [`Agent`] - Autonomous entity that processes tasks
//! - [`Supervisor`] - Orchestrates executors in distributed systems
//! - [`Executor`] - Worker that processes tasks with LLM
//!
//! ## Validation
//! - [`Validator`] - Rule-based validation (fast, for text/JSON)
//! - [`SandboxValidator`] - Ground truth validation (for code)
//! - [`ValidatorPipeline`] - Chain multiple validators
//!
//! ## Configuration
//! - [`LLMConfig`] - LLM provider configuration
//! - [`ValidationRule`] - Rules for Validator
//!
//! # Example
//!
//! ```rust,no_run
//! use agentic_core::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a supervisor with executors
//!     let sup = Supervisor::new("my-sup")
//!         .executors(4)
//!         .llm_gemini("gemini-2.0-flash")
//!         .build_async()
//!         .await;
//!
//!     // Create a standalone executor
//!     let exe = Executor::new("exe-001", "sup-001")
//!         .llm_gemini("gemini-2.0-flash")
//!         .build();
//!
//!     // Execute a task
//!     let result = exe.execute("You are helpful.", "What is 2+2?").await?;
//!     println!("{}", result.content);
//!
//!     Ok(())
//! }
//! ```

// Core agent
pub use crate::Agent;
pub use crate::AgentError;

// Distributed architecture - essential types only
pub use crate::distributed::{
    // Core components
    Supervisor,
    Executor,

    // Validation
    Validator,
    ValidationRule,
    ValidationResult,
    SandboxValidator,
    SandboxValidationResult,
    ValidatorPipeline,
    ValidatorInput,

    // Configuration
    LLMConfig,
    SandboxConfig,
    Capability,

    // Results
    ExecutionResult,
    Language,
};

// Bus components for advanced users
pub use crate::distributed::bus::{
    BusCoordinator,
    BusLinter,
    RollbackManager,
    ErrorFingerprinter,
    ModelTier,

    // Audit trail (new)
    AuditCollector,
    AuditEvent,
    AuditEventType,
    AuditReport,
    TraceSequencer,
};
