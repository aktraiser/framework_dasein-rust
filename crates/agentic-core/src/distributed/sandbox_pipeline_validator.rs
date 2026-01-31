//! Sandbox Pipeline Validator - Wraps SandboxValidator for the pipeline.

use async_trait::async_trait;
use std::path::PathBuf;

use agentic_sandbox::Sandbox;

use super::sandbox_validator::{Language, SandboxValidator};
use super::validator_pipeline::{PipelineValidator, ValidatorInput, ValidatorOutput};

/// Wraps SandboxValidator for use in ValidatorPipeline.
pub struct SandboxPipelineValidator<S: Sandbox> {
    inner: SandboxValidator<S>,
}

impl<S: Sandbox + Send + Sync + 'static> SandboxPipelineValidator<S> {
    pub fn new(sandbox: S) -> Self {
        Self {
            inner: SandboxValidator::new(sandbox),
        }
    }

    pub fn workspace(mut self, path: PathBuf) -> Self {
        self.inner = self.inner.workspace(path);
        self
    }

    pub fn run_tests(mut self, enabled: bool) -> Self {
        self.inner = self.inner.run_tests(enabled);
        self
    }
}

#[async_trait]
impl<S: Sandbox + Send + Sync + 'static> PipelineValidator for SandboxPipelineValidator<S> {
    fn name(&self) -> &str {
        "sandbox"
    }

    async fn validate(&self, input: &ValidatorInput) -> Result<ValidatorOutput, String> {
        let language = match input.language.as_str() {
            "rust" => Language::Rust,
            "python" => Language::Python,
            "typescript" => Language::TypeScript,
            lang => return Err(format!("Unsupported language: {}", lang)),
        };

        let result = self.inner.validate_code(&input.code, language).await;

        match result {
            Ok(validation) => {
                if validation.passed {
                    Ok(ValidatorOutput::success("sandbox").with_feedback(format!(
                        "Compilation OK, {} tests passed",
                        validation.tests_ok
                    )))
                } else {
                    let mut errors = validation.compiler_errors.clone();
                    errors.extend(validation.test_errors.clone());

                    let mut recommendations = Vec::new();

                    // Generate recommendations based on error patterns
                    for err in &errors {
                        if err.contains("unresolved") {
                            recommendations.push(
                                "Check that all required dependencies are in Cargo.toml"
                                    .to_string(),
                            );
                        }
                        if err.contains("trait bound") {
                            recommendations.push(
                                "Verify trait implementations match expected bounds".to_string(),
                            );
                        }
                        if err.contains("cannot borrow") || err.contains("borrowed") {
                            recommendations
                                .push("Review ownership and borrowing rules".to_string());
                        }
                        if err.contains("lifetime") {
                            recommendations.push("Check lifetime annotations".to_string());
                        }
                        if err.contains("unstable") || err.contains("E0658") {
                            recommendations.push(
                                "IMPORTANT: Use only stable Rust features. Avoid LinkedList.retain() - use Vec or VecDeque with manual filtering instead.".to_string()
                            );
                        }
                        if err.contains("linked_list") {
                            recommendations.push(
                                "Replace LinkedList with Vec<(K, Instant)> for LRU tracking - it's simpler and stable.".to_string()
                            );
                        }
                        if err.contains("unclosed delimiter") || err.contains("unexpected closing")
                        {
                            recommendations.push(
                                "SYNTAX ERROR: Check all braces {}, brackets [], and parentheses () are balanced. Count them carefully.".to_string()
                            );
                        }
                        if err.contains("recursion in an async") {
                            recommendations.push(
                                "Async recursion requires Box::pin(). Use: Box::pin(async move { ... })".to_string()
                            );
                        }
                        if err.contains("await") && err.contains("only allowed inside") {
                            recommendations.push(
                                "Move .await inside an async block or async fn. Don't use .await in regular closures.".to_string()
                            );
                        }
                    }

                    // Dedupe recommendations
                    recommendations.sort();
                    recommendations.dedup();

                    Ok(ValidatorOutput::failure("sandbox", errors)
                        .with_recommendations(recommendations)
                        .with_feedback(validation.feedback.unwrap_or_default()))
                }
            }
            Err(e) => Err(format!("Sandbox error: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_sandbox::ProcessSandbox;

    #[tokio::test]
    async fn test_sandbox_pipeline_validator() {
        let sandbox = ProcessSandbox::new().with_timeout(60000);
        let validator =
            SandboxPipelineValidator::new(sandbox).workspace(PathBuf::from("/tmp/test-pipeline"));

        let input = ValidatorInput::new("fn main() {}", "rust");
        let result = validator.validate(&input).await;

        assert!(result.is_ok());
    }
}
