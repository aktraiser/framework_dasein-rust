//! Test Validator Executor - Validates code passes tests (MAF Pattern)
//!
//! Uses dynamic dispatch with `serde_json::Value` for MAF compatibility.
//!
//! # Input Schema
//!
//! ```json
//! {
//!   "code": "fn add(a: i32, b: i32) -> i32 { a + b }",
//!   "language": "rust",
//!   "test_filter": "test_add",
//!   "task": "Optional task description"
//! }
//! ```
//!
//! # Output Schema
//!
//! ```json
//! {
//!   "code": "...",
//!   "language": "rust",
//!   "passed": true,
//!   "test_count": 5,
//!   "tests_passed": 5,
//!   "tests_failed": 0,
//!   "errors": [],
//!   "feedback": null,
//!   "execution_time_ms": 2500
//! }
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! let validator = TestValidatorExecutor::new("test-runner", sandbox)
//!     .with_language(Language::Rust);
//!
//! // Chain after compile validator
//! builder.add_conditional_edge(
//!     "compile", "test-runner",
//!     |v| v.get("passed").and_then(|p| p.as_bool()).unwrap_or(false),
//!     "if_compiles"
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

/// Input to the test validator (type-safe helper).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestInput {
    /// The code to test.
    pub code: String,
    /// Language of the code.
    pub language: String,
    /// Optional: test filter pattern.
    #[serde(default)]
    pub test_filter: Option<String>,
    /// Optional: task description for context.
    #[serde(default)]
    pub task: Option<String>,
}

impl TestInput {
    /// Create new test input.
    pub fn new(code: impl Into<String>, language: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            language: language.into(),
            test_filter: None,
            task: None,
        }
    }

    /// Add test filter pattern.
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.test_filter = Some(filter.into());
        self
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

/// Output from the test validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestOutput {
    /// The original code.
    pub code: String,
    /// Language of the code.
    pub language: String,
    /// Whether all tests passed.
    pub passed: bool,
    /// Number of tests run.
    #[serde(default)]
    pub test_count: u32,
    /// Number of tests passed.
    #[serde(default)]
    pub tests_passed: u32,
    /// Number of tests failed.
    #[serde(default)]
    pub tests_failed: u32,
    /// Test errors if any.
    #[serde(default)]
    pub errors: Vec<String>,
    /// Structured feedback for retry.
    #[serde(default)]
    pub feedback: Option<String>,
    /// Execution time in milliseconds.
    #[serde(default)]
    pub execution_time_ms: u64,
}

impl TestOutput {
    /// Check if all tests passed.
    pub fn is_passed(&self) -> bool {
        self.passed
    }

    /// Get pass rate as percentage.
    pub fn pass_rate(&self) -> f64 {
        if self.test_count == 0 {
            100.0
        } else {
            (self.tests_passed as f64 / self.test_count as f64) * 100.0
        }
    }

    /// Convert to JSON Value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

// ============================================================================
// TEST VALIDATOR EXECUTOR
// ============================================================================

/// Graph Executor that validates code by running tests.
///
/// Uses MAF-style dynamic dispatch with `serde_json::Value`.
pub struct TestValidatorExecutor<S: Sandbox> {
    id: ExecutorId,
    /// The underlying sandbox validator.
    validator: SandboxValidator<S>,
    /// Default language if not specified in input.
    default_language: Language,
    /// Workspace path for temporary files.
    #[allow(dead_code)]
    workspace: PathBuf,
    /// Whether to run tests (can be disabled for compile-only).
    #[allow(dead_code)]
    run_tests: bool,
}

impl<S: Sandbox + Send + Sync + 'static> TestValidatorExecutor<S> {
    /// Create a new test validator executor.
    pub fn new(id: impl Into<String>, sandbox: S) -> Self {
        Self {
            id: ExecutorId::new(id),
            validator: SandboxValidator::new(sandbox),
            default_language: Language::Rust,
            workspace: PathBuf::from("/tmp/test-validation"),
            run_tests: true,
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

    /// Disable test execution (compile-only mode).
    pub fn compile_only(mut self) -> Self {
        self.run_tests = false;
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

    /// Validate code with tests based on language.
    async fn validate_with_tests(
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
impl<S: Sandbox + Send + Sync + 'static> Executor for TestValidatorExecutor<S> {
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
        "Test Validator"
    }

    fn description(&self) -> Option<&str> {
        Some("Validates code by running tests in a sandbox")
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Deserialize input dynamically
        let typed_input: TestInput = serde_json::from_value(input).map_err(|e| {
            ExecutorError::new(
                self.id.clone(),
                format!("Invalid input (expected TestInput): {}", e),
            )
        })?;

        let language = self.parse_language(&typed_input.language);

        // Run tests
        let sandbox_result = self
            .validate_with_tests(&typed_input.code, language)
            .await?;

        // Convert to ValidationResult
        let result = if sandbox_result.tests_passed {
            ValidationResult::success()
        } else if !sandbox_result.compiles {
            ValidationResult::failure(sandbox_result.compiler_errors.clone())
                .with_feedback("Code must compile before tests can run.")
        } else {
            ValidationResult::failure(sandbox_result.test_errors.clone())
                .with_feedback(sandbox_result.feedback.clone().unwrap_or_default())
        };

        // Log event
        ctx.add_event(
            "test_execution",
            serde_json::json!({
                "passed": sandbox_result.tests_passed,
                "test_count": sandbox_result.test_count,
                "tests_ok": sandbox_result.tests_ok,
                "tests_failed": sandbox_result.tests_failed,
                "language": typed_input.language,
                "execution_time_ms": sandbox_result.execution_time_ms,
            }),
        )
        .await?;

        // Build output
        let output = TestOutput {
            code: typed_input.code.clone(),
            language: typed_input.language.clone(),
            passed: result.passed,
            test_count: sandbox_result.test_count,
            tests_passed: sandbox_result.tests_ok,
            tests_failed: sandbox_result.tests_failed,
            errors: sandbox_result.test_errors,
            feedback: result.feedback,
            execution_time_ms: sandbox_result.execution_time_ms,
        };

        // Send as Value to next executors
        ctx.send_message(output.to_value()).await?;

        // Yield status output
        let status = if output.passed {
            format!(
                "Tests: PASSED ({}/{} ok)",
                output.tests_passed, output.test_count
            )
        } else {
            format!(
                "Tests: FAILED ({}/{} ok, {} failures)",
                output.tests_passed, output.test_count, output.tests_failed
            )
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
    fn test_test_input() {
        let input = TestInput::new("fn main() {}", "rust")
            .with_filter("test_")
            .with_task("Write tests");

        assert_eq!(input.code, "fn main() {}");
        assert_eq!(input.language, "rust");
        assert_eq!(input.test_filter, Some("test_".into()));
    }

    #[test]
    fn test_test_input_to_value() {
        let input = TestInput::new("fn main() {}", "rust");
        let value = input.to_value();

        assert_eq!(value["code"], "fn main() {}");
        assert_eq!(value["language"], "rust");
    }

    #[test]
    fn test_test_output_passed() {
        let output = TestOutput {
            code: "fn main() {}".into(),
            language: "rust".into(),
            passed: true,
            test_count: 5,
            tests_passed: 5,
            tests_failed: 0,
            errors: vec![],
            feedback: None,
            execution_time_ms: 100,
        };

        assert!(output.is_passed());
        assert_eq!(output.pass_rate(), 100.0);
    }

    #[test]
    fn test_test_output_partial_failure() {
        let output = TestOutput {
            code: "fn main() {}".into(),
            language: "rust".into(),
            passed: false,
            test_count: 5,
            tests_passed: 4,
            tests_failed: 1,
            errors: vec!["test_add failed".into()],
            feedback: None,
            execution_time_ms: 100,
        };

        assert!(!output.is_passed());
        assert_eq!(output.pass_rate(), 80.0);
    }

    #[test]
    fn test_test_output_to_value() {
        let output = TestOutput {
            code: "fn main() {}".into(),
            language: "rust".into(),
            passed: true,
            test_count: 5,
            tests_passed: 5,
            tests_failed: 0,
            errors: vec![],
            feedback: None,
            execution_time_ms: 100,
        };

        let value = output.to_value();
        assert_eq!(value["passed"], true);
        assert_eq!(value["test_count"], 5);
    }

    #[test]
    fn test_deserialize_from_value() {
        let value = serde_json::json!({
            "code": "fn main() {}",
            "language": "rust"
        });

        let input: TestInput = serde_json::from_value(value).unwrap();
        assert_eq!(input.code, "fn main() {}");
        assert_eq!(input.language, "rust");
        assert!(input.test_filter.is_none());
    }
}
