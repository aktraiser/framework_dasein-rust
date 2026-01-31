//! Compile Validator Executor - Validates code compiles correctly (MAF Pattern)
//!
//! Uses dynamic dispatch with `serde_json::Value` for MAF compatibility.
//!
//! # Input Schema
//!
//! ```json
//! {
//!   "code": "fn main() { ... }",
//!   "language": "rust",
//!   "task": "Optional task description"
//! }
//! ```
//!
//! # Output Schema
//!
//! ```json
//! {
//!   "code": "fn main() { ... }",
//!   "language": "rust",
//!   "passed": true,
//!   "errors": [],
//!   "feedback": null,
//!   "execution_time_ms": 150
//! }
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! let validator = CompileValidatorExecutor::new("validator", sandbox)
//!     .with_language(Language::Rust);
//!
//! // Use with conditional edges:
//! builder.add_conditional_edge(
//!     "validator", "retry",
//!     |v: &Value| !v.get("passed").and_then(|p| p.as_bool()).unwrap_or(true),
//!     "on_failure"
//! );
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use crate::distributed::graph::{
    Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind, ValidationResult,
};
use crate::distributed::{Language, SandboxValidationResult, SandboxValidator};
use dasein_agentic_sandbox::Sandbox;

// ============================================================================
// INPUT/OUTPUT TYPES
// ============================================================================

/// Input to the compile validator (type-safe helper).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileInput {
    /// The code to validate.
    pub code: String,
    /// Language of the code.
    pub language: String,
    /// Optional: task description for context.
    #[serde(default)]
    pub task: Option<String>,
}

impl CompileInput {
    /// Create new compile input.
    pub fn new(code: impl Into<String>, language: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            language: language.into(),
            task: None,
        }
    }

    /// Add task description.
    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(task.into());
        self
    }

    /// Convert to JSON Value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

/// Output from the compile validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileOutput {
    /// The original code (for passing to next executor).
    pub code: String,
    /// Language of the code.
    pub language: String,
    /// Whether compilation passed.
    pub passed: bool,
    /// Compiler errors if any.
    #[serde(default)]
    pub errors: Vec<String>,
    /// Structured feedback for retry.
    #[serde(default)]
    pub feedback: Option<String>,
    /// Execution time in milliseconds.
    #[serde(default)]
    pub execution_time_ms: u64,
}

impl CompileOutput {
    /// Check if compilation passed.
    pub fn is_passed(&self) -> bool {
        self.passed
    }

    /// Convert to JSON Value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

// ============================================================================
// COMPILE VALIDATOR EXECUTOR
// ============================================================================

/// Graph Executor that validates code compilation.
///
/// Uses MAF-style dynamic dispatch with `serde_json::Value`.
pub struct CompileValidatorExecutor<S: Sandbox> {
    id: ExecutorId,
    /// The underlying sandbox validator.
    validator: SandboxValidator<S>,
    /// Default language if not specified in input.
    default_language: Language,
    /// Workspace path for temporary files.
    #[allow(dead_code)]
    workspace: PathBuf,
}

impl<S: Sandbox + Send + Sync + 'static> CompileValidatorExecutor<S> {
    /// Create a new compile validator executor.
    pub fn new(id: impl Into<String>, sandbox: S) -> Self {
        Self {
            id: ExecutorId::new(id),
            validator: SandboxValidator::new(sandbox),
            default_language: Language::Rust,
            workspace: PathBuf::from("/tmp/compile-validation"),
        }
    }

    /// Set the default language.
    pub fn with_language(mut self, language: Language) -> Self {
        self.default_language = language;
        self
    }

    /// Set the workspace path.
    pub fn with_workspace(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace = path.into();
        self
    }

    /// Parse language string to Language enum.
    fn parse_language(&self, lang: &str) -> Language {
        match lang.to_lowercase().as_str() {
            "rust" => Language::Rust,
            "python" | "py" => Language::Python,
            "typescript" | "ts" => Language::TypeScript,
            "go" | "golang" => Language::Go,
            _ => self.default_language,
        }
    }

    /// Validate code based on language.
    async fn validate_code(
        &self,
        code: &str,
        language: Language,
    ) -> Result<SandboxValidationResult, ExecutorError> {
        self.validator
            .validate_code(code, language)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("Sandbox error: {}", e)))
    }
}

#[async_trait]
impl<S: Sandbox + Send + Sync + 'static> Executor for CompileValidatorExecutor<S> {
    // MAF Pattern: All types are Value for dynamic dispatch
    type Input = Value;
    type Message = Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Validator
    }

    fn name(&self) -> &str {
        "Compile Validator"
    }

    fn description(&self) -> Option<&str> {
        Some("Validates code compilation using real compiler")
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Deserialize input dynamically
        let typed_input: CompileInput = serde_json::from_value(input).map_err(|e| {
            ExecutorError::new(
                self.id.clone(),
                format!("Invalid input (expected CompileInput): {}", e),
            )
        })?;

        let language = self.parse_language(&typed_input.language);

        // Run compilation
        let sandbox_result = self.validate_code(&typed_input.code, language).await?;

        // Convert to ValidationResult
        let result = if sandbox_result.compiles {
            ValidationResult::success()
        } else {
            ValidationResult::failure(sandbox_result.compiler_errors.clone())
                .with_feedback(sandbox_result.feedback.clone().unwrap_or_default())
        };

        // Log event
        ctx.add_event(
            "compilation",
            serde_json::json!({
                "passed": sandbox_result.compiles,
                "error_count": sandbox_result.compiler_errors.len(),
                "language": typed_input.language,
                "execution_time_ms": sandbox_result.execution_time_ms,
            }),
        )
        .await?;

        // Build output
        let output = CompileOutput {
            code: typed_input.code.clone(),
            language: typed_input.language.clone(),
            passed: result.passed,
            errors: sandbox_result.compiler_errors,
            feedback: result.feedback,
            execution_time_ms: sandbox_result.execution_time_ms,
        };

        // Send as Value to next executors
        ctx.send_message(output.to_value()).await?;

        // Yield status output
        let status = if output.passed {
            "Compilation: PASSED".to_string()
        } else {
            format!("Compilation: FAILED ({} errors)", output.errors.len())
        };
        ctx.yield_output(status).await?;

        Ok(())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_input() {
        let input = CompileInput::new("fn main() {}", "rust").with_task("Write a function");

        assert_eq!(input.code, "fn main() {}");
        assert_eq!(input.language, "rust");
        assert!(input.task.is_some());
    }

    #[test]
    fn test_compile_input_to_value() {
        let input = CompileInput::new("fn main() {}", "rust");
        let value = input.to_value();

        assert_eq!(value["code"], "fn main() {}");
        assert_eq!(value["language"], "rust");
    }

    #[test]
    fn test_compile_output_passed() {
        let output = CompileOutput {
            code: "fn main() {}".into(),
            language: "rust".into(),
            passed: true,
            errors: vec![],
            feedback: None,
            execution_time_ms: 100,
        };

        assert!(output.is_passed());
    }

    #[test]
    fn test_compile_output_to_value() {
        let output = CompileOutput {
            code: "fn main() {}".into(),
            language: "rust".into(),
            passed: true,
            errors: vec![],
            feedback: None,
            execution_time_ms: 100,
        };

        let value = output.to_value();
        assert_eq!(value["passed"], true);
        assert_eq!(value["code"], "fn main() {}");
    }

    #[test]
    fn test_deserialize_from_value() {
        let value = serde_json::json!({
            "code": "fn main() {}",
            "language": "rust"
        });

        let input: CompileInput = serde_json::from_value(value).unwrap();
        assert_eq!(input.code, "fn main() {}");
        assert_eq!(input.language, "rust");
        assert!(input.task.is_none());
    }
}
