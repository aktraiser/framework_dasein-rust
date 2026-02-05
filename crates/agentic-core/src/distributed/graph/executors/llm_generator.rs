//! LLM Generator Executor - Code generation via LLM (MAF Pattern)
//!
//! Uses dynamic dispatch with `serde_json::Value` for MAF compatibility.
//!
//! # Input Schema
//!
//! ```json
//! {
//!   "prompt": "Write a fibonacci function",
//!   "language": "rust",
//!   "context": "Optional additional context",
//!   "previous_errors": ["Optional error from previous attempt"]
//! }
//! ```
//!
//! # Output Schema
//!
//! ```json
//! {
//!   "code": "fn fibonacci(n: u32) -> u32 { ... }",
//!   "language": "rust",
//!   "prompt": "Original prompt",
//!   "tokens_used": 150,
//!   "truncated": false
//! }
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! let generator = LLMGeneratorExecutor::new("generator", llm_executor)
//!     .with_system_prompt("You are a Rust expert.");
//!
//! // Input as Value
//! let input = serde_json::json!({
//!     "prompt": "Write fibonacci",
//!     "language": "rust"
//! });
//!
//! // Register in workflow
//! registry.register(generator);
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::distributed::graph::{
    Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind,
};
use crate::distributed::Executor as LLMExecutor;

// ============================================================================
// INPUT/OUTPUT TYPES (for type-safe construction)
// ============================================================================

/// Input to the LLM generator (type-safe helper).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorInput {
    /// The prompt/task for generation.
    pub prompt: String,
    /// Target language.
    pub language: String,
    /// Additional context (e.g., existing code, types).
    #[serde(default)]
    pub context: Option<String>,
    /// Errors from previous attempts (for retry).
    #[serde(default)]
    pub previous_errors: Vec<String>,
}

impl GeneratorInput {
    /// Create new generator input.
    pub fn new(prompt: impl Into<String>, language: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            language: language.into(),
            context: None,
            previous_errors: vec![],
        }
    }

    /// Add context.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Add errors from previous attempts.
    pub fn with_errors(mut self, errors: Vec<String>) -> Self {
        self.previous_errors = errors;
        self
    }

    /// Convert to JSON Value for executor input.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

/// Output from the LLM generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedCode {
    /// The generated code content.
    pub code: String,
    /// Language of the generated code.
    pub language: String,
    /// Original prompt that generated this code.
    pub prompt: String,
    /// Tokens used for generation.
    #[serde(default)]
    pub tokens_used: u32,
    /// Whether the output was truncated.
    #[serde(default)]
    pub truncated: bool,
    /// Previous errors (if this is a retry).
    #[serde(default)]
    pub previous_errors: Vec<String>,
}

impl GeneratedCode {
    /// Create new generated code.
    pub fn new(code: impl Into<String>, language: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            language: language.into(),
            prompt: String::new(),
            tokens_used: 0,
            truncated: false,
            previous_errors: vec![],
        }
    }

    /// Add the prompt that generated this code.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    /// Add token usage.
    pub fn with_tokens(mut self, tokens: u32) -> Self {
        self.tokens_used = tokens;
        self
    }

    /// Convert to JSON Value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

// ============================================================================
// LLM GENERATOR EXECUTOR
// ============================================================================

/// Graph Executor that generates code using an LLM.
///
/// Uses MAF-style dynamic dispatch with `serde_json::Value`.
pub struct LLMGeneratorExecutor {
    id: ExecutorId,
    /// The underlying LLM executor (needs Mutex for internal state).
    llm: Arc<Mutex<LLMExecutor>>,
    /// System prompt for the LLM.
    system_prompt: String,
    /// Whether to extract code from markdown blocks.
    extract_code: bool,
}

impl LLMGeneratorExecutor {
    /// Create a new LLM generator executor.
    pub fn new(id: impl Into<String>, llm: LLMExecutor) -> Self {
        Self {
            id: ExecutorId::new(id),
            llm: Arc::new(Mutex::new(llm)),
            system_prompt: default_system_prompt(),
            extract_code: true,
        }
    }

    /// Create with shared LLM executor.
    pub fn with_shared_llm(id: impl Into<String>, llm: Arc<Mutex<LLMExecutor>>) -> Self {
        Self {
            id: ExecutorId::new(id),
            llm,
            system_prompt: default_system_prompt(),
            extract_code: true,
        }
    }

    /// Set the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Disable code extraction from markdown.
    pub fn without_code_extraction(mut self) -> Self {
        self.extract_code = false;
        self
    }

    /// Extract code from markdown code blocks.
    fn extract_code_from_response(&self, content: &str) -> String {
        if !self.extract_code {
            return content.to_string();
        }

        let re = regex::Regex::new(r"```(?:rust|python|typescript|javascript|go|java)?\n([\s\S]*?)```")
            .unwrap();

        if let Some(captures) = re.captures(content) {
            if let Some(code) = captures.get(1) {
                return code.as_str().trim().to_string();
            }
        }

        content.to_string()
    }

    /// Build the user prompt with context and errors.
    fn build_user_prompt(&self, input: &GeneratorInput) -> String {
        let mut parts = vec![input.prompt.clone()];

        if let Some(ctx) = &input.context {
            parts.push(format!("\n\n## Context\n\n{}", ctx));
        }

        if !input.previous_errors.is_empty() {
            parts.push(format!(
                "\n\n## Previous Errors (FIX THESE)\n\n{}",
                input.previous_errors.join("\n")
            ));
        }

        parts.join("")
    }
}

#[async_trait]
impl Executor for LLMGeneratorExecutor {
    // MAF Pattern: All types are Value for dynamic dispatch
    type Input = Value;
    type Message = Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    fn name(&self) -> &str {
        "LLM Generator"
    }

    fn description(&self) -> Option<&str> {
        Some("Generates code using an LLM with configurable prompts")
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Deserialize input dynamically
        let typed_input: GeneratorInput = serde_json::from_value(input).map_err(|e| {
            ExecutorError::new(
                self.id.clone(),
                format!("Invalid input (expected GeneratorInput): {}", e),
            )
        })?;

        // Build prompts
        let user_prompt = self.build_user_prompt(&typed_input);

        // Execute LLM
        let llm = self.llm.lock().await;
        let result = llm
            .execute(&self.system_prompt, &user_prompt)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("LLM error: {}", e)))?;
        drop(llm);

        // Extract code
        let code = self.extract_code_from_response(&result.content);

        // Log event
        ctx.add_event(
            "llm_generation",
            serde_json::json!({
                "tokens_used": result.tokens_used,
                "truncated": result.truncated,
                "code_length": code.len(),
            }),
        )
        .await?;

        // Build output
        let generated = GeneratedCode::new(&code, &typed_input.language)
            .with_prompt(&typed_input.prompt)
            .with_tokens(result.tokens_used);

        // Send as Value to next executors
        ctx.send_message(generated.to_value()).await?;

        // Yield progress output
        ctx.yield_output(format!("Generated {} bytes", code.len()))
            .await?;

        Ok(())
    }
}

// ============================================================================
// DEFAULT PROMPTS
// ============================================================================

fn default_system_prompt() -> String {
    r#"You are an expert programmer. Generate clean, well-documented code.

RULES:
1. Return ONLY valid, compilable code
2. NO markdown formatting unless asked
3. Include proper error handling
4. Add doc comments for public APIs
5. Follow language idioms and best practices"#
        .to_string()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generator_input() {
        let input = GeneratorInput::new("Write fibonacci", "rust")
            .with_context("pub trait Sequence { fn next(&mut self) -> u64; }")
            .with_errors(vec!["error[E0308]: expected i32".into()]);

        assert_eq!(input.prompt, "Write fibonacci");
        assert_eq!(input.language, "rust");
        assert!(input.context.is_some());
        assert_eq!(input.previous_errors.len(), 1);
    }

    #[test]
    fn test_generator_input_to_value() {
        let input = GeneratorInput::new("test", "rust");
        let value = input.to_value();

        assert_eq!(value["prompt"], "test");
        assert_eq!(value["language"], "rust");
    }

    #[test]
    fn test_generated_code() {
        let code = GeneratedCode::new("fn main() {}", "rust").with_tokens(100);

        assert_eq!(code.language, "rust");
        assert_eq!(code.tokens_used, 100);
    }

    #[test]
    fn test_generated_code_to_value() {
        let code = GeneratedCode::new("fn main() {}", "rust");
        let value = code.to_value();

        assert_eq!(value["code"], "fn main() {}");
        assert_eq!(value["language"], "rust");
    }

    #[test]
    fn test_code_extraction() {
        let executor = LLMGeneratorExecutor {
            id: ExecutorId::new("test"),
            llm: Arc::new(Mutex::new(
                crate::distributed::Executor::new("test", "test").build(),
            )),
            system_prompt: String::new(),
            extract_code: true,
        };

        let response = "Here's the code:\n```rust\nfn main() {}\n```\nThat's it.";
        let extracted = executor.extract_code_from_response(response);
        assert_eq!(extracted, "fn main() {}");
    }

    #[test]
    fn test_deserialize_from_value() {
        let value = serde_json::json!({
            "prompt": "test prompt",
            "language": "rust"
        });

        let input: GeneratorInput = serde_json::from_value(value).unwrap();
        assert_eq!(input.prompt, "test prompt");
        assert_eq!(input.language, "rust");
        assert!(input.context.is_none());
        assert!(input.previous_errors.is_empty());
    }
}
