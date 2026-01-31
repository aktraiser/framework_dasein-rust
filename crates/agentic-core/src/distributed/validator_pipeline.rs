//! Validator Pipeline - Chain multiple validators together.
//!
//! Validators enrich the context for Executors by providing:
//! - Compilation errors (SandboxValidator)
//! - Documentation (MCPDocValidator)
//! - Security analysis (future)
//!
//! # Architecture
//!
//! ```text
//!                    EXECUTOR
//!                       ▲
//!                       │ enriched feedback
//!        ┌──────────────┴──────────────┐
//!        │      VALIDATOR PIPELINE      │
//!        │                              │
//!        │  SandboxValidator ──────┐   │
//!        │  (compile errors)       │   │
//!        │                         ▼   │
//!        │  MCPDocValidator ───────┼───│
//!        │  (docs for errors)      │   │
//!        │                         ▼   │
//!        │  Combined Feedback ─────────│
//!        └─────────────────────────────┘
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Input to a validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorInput {
    /// The code to validate
    pub code: String,
    /// Programming language
    pub language: String,
    /// Previous validation results (for chaining)
    pub previous_results: Vec<ValidatorOutput>,
    /// Original task description
    pub task: Option<String>,
}

impl ValidatorInput {
    pub fn new(code: impl Into<String>, language: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            language: language.into(),
            previous_results: vec![],
            task: None,
        }
    }

    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(task.into());
        self
    }

    pub fn with_previous(mut self, results: Vec<ValidatorOutput>) -> Self {
        self.previous_results = results;
        self
    }
}

/// Output from a validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorOutput {
    /// Validator name
    pub validator: String,
    /// Did validation pass?
    pub passed: bool,
    /// Errors found
    pub errors: Vec<String>,
    /// Recommendations for the LLM
    pub recommendations: Vec<String>,
    /// Documentation snippets (from MCP)
    pub documentation: Vec<DocSnippet>,
    /// Raw feedback string
    pub feedback: Option<String>,
}

/// Documentation snippet from MCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocSnippet {
    /// Source (e.g., "context7", "rust-docs")
    pub source: String,
    /// Topic/query
    pub topic: String,
    /// The documentation content
    pub content: String,
}

impl ValidatorOutput {
    pub fn success(validator: impl Into<String>) -> Self {
        Self {
            validator: validator.into(),
            passed: true,
            errors: vec![],
            recommendations: vec![],
            documentation: vec![],
            feedback: None,
        }
    }

    pub fn failure(validator: impl Into<String>, errors: Vec<String>) -> Self {
        Self {
            validator: validator.into(),
            passed: false,
            errors,
            recommendations: vec![],
            documentation: vec![],
            feedback: None,
        }
    }

    pub fn with_recommendations(mut self, recs: Vec<String>) -> Self {
        self.recommendations = recs;
        self
    }

    pub fn with_documentation(mut self, docs: Vec<DocSnippet>) -> Self {
        self.documentation = docs;
        self
    }

    pub fn with_feedback(mut self, feedback: impl Into<String>) -> Self {
        self.feedback = Some(feedback.into());
        self
    }
}

/// Combined result from the entire pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    /// Did all validators pass?
    pub passed: bool,
    /// Results from each validator
    pub results: Vec<ValidatorOutput>,
    /// Combined feedback for the LLM
    pub combined_feedback: String,
    /// Total execution time
    pub execution_time_ms: u64,
}

impl PipelineResult {
    /// Build combined feedback from all validator results.
    pub fn build_feedback(&self) -> String {
        let mut feedback = String::new();

        for result in &self.results {
            if !result.passed {
                feedback.push_str(&format!("\n=== {} ===\n", result.validator.to_uppercase()));

                // Errors
                if !result.errors.is_empty() {
                    feedback.push_str("ERRORS:\n");
                    for err in &result.errors {
                        feedback.push_str(&format!("  - {}\n", err));
                    }
                }

                // Recommendations
                if !result.recommendations.is_empty() {
                    feedback.push_str("\nRECOMMENDATIONS:\n");
                    for rec in &result.recommendations {
                        feedback.push_str(&format!("  - {}\n", rec));
                    }
                }

                // Documentation
                if !result.documentation.is_empty() {
                    feedback.push_str("\nRELEVANT DOCUMENTATION:\n");
                    for doc in &result.documentation {
                        feedback.push_str(&format!("  [{}/{}]\n", doc.source, doc.topic));
                        // First 500 chars of doc
                        let content: String = doc.content.chars().take(500).collect();
                        feedback.push_str(&format!("  {}\n", content));
                    }
                }
            }
        }

        if feedback.is_empty() {
            "All validations passed.".to_string()
        } else {
            feedback
        }
    }
}

/// Trait for validators in the pipeline.
#[async_trait]
pub trait PipelineValidator: Send + Sync {
    /// Validator name.
    fn name(&self) -> &str;

    /// Validate the input and return output.
    async fn validate(&self, input: &ValidatorInput) -> Result<ValidatorOutput, String>;
}

/// Pipeline that chains validators together.
pub struct ValidatorPipeline {
    validators: Vec<Box<dyn PipelineValidator>>,
}

impl ValidatorPipeline {
    pub fn new() -> Self {
        Self { validators: vec![] }
    }

    /// Add a validator to the pipeline.
    pub fn add<V: PipelineValidator + 'static>(mut self, validator: V) -> Self {
        self.validators.push(Box::new(validator));
        self
    }

    /// Run all validators in sequence.
    pub async fn validate(&self, mut input: ValidatorInput) -> PipelineResult {
        let start = std::time::Instant::now();
        let mut results = Vec::new();
        let mut all_passed = true;

        for validator in &self.validators {
            match validator.validate(&input).await {
                Ok(output) => {
                    if !output.passed {
                        all_passed = false;
                    }
                    results.push(output.clone());
                    // Chain: next validator sees previous results
                    input.previous_results.push(output);
                }
                Err(e) => {
                    let output = ValidatorOutput::failure(
                        validator.name(),
                        vec![format!("Validator error: {}", e)],
                    );
                    all_passed = false;
                    results.push(output);
                }
            }
        }

        let mut result = PipelineResult {
            passed: all_passed,
            results,
            combined_feedback: String::new(),
            execution_time_ms: start.elapsed().as_millis() as u64,
        };

        result.combined_feedback = result.build_feedback();
        result
    }
}

impl Default for ValidatorPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockPassValidator;

    #[async_trait]
    impl PipelineValidator for MockPassValidator {
        fn name(&self) -> &str {
            "mock-pass"
        }

        async fn validate(&self, _input: &ValidatorInput) -> Result<ValidatorOutput, String> {
            Ok(ValidatorOutput::success("mock-pass"))
        }
    }

    struct MockFailValidator;

    #[async_trait]
    impl PipelineValidator for MockFailValidator {
        fn name(&self) -> &str {
            "mock-fail"
        }

        async fn validate(&self, _input: &ValidatorInput) -> Result<ValidatorOutput, String> {
            Ok(
                ValidatorOutput::failure("mock-fail", vec!["Test error".to_string()])
                    .with_recommendations(vec!["Fix the error".to_string()]),
            )
        }
    }

    #[tokio::test]
    async fn test_pipeline_all_pass() {
        let pipeline = ValidatorPipeline::new()
            .add(MockPassValidator)
            .add(MockPassValidator);

        let input = ValidatorInput::new("fn main() {}", "rust");
        let result = pipeline.validate(input).await;

        assert!(result.passed);
        assert_eq!(result.results.len(), 2);
    }

    #[tokio::test]
    async fn test_pipeline_with_failure() {
        let pipeline = ValidatorPipeline::new()
            .add(MockPassValidator)
            .add(MockFailValidator);

        let input = ValidatorInput::new("fn main() {}", "rust");
        let result = pipeline.validate(input).await;

        assert!(!result.passed);
        assert!(result.combined_feedback.contains("Test error"));
        assert!(result.combined_feedback.contains("Fix the error"));
    }
}
