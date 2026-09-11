//! Core traits and types for LLM adapters.

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

use crate::error::LLMError;

/// Role in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// System message (instructions)
    System,
    /// User message
    User,
    /// Assistant response
    Assistant,
    /// Tool result (function output)
    Tool,
}

/// A message in a conversation.
#[derive(Debug, Clone)]
pub struct LLMMessage {
    /// Role of the message sender
    pub role: Role,
    /// Content of the message
    pub content: String,
    /// Tool call ID (for tool results)
    pub tool_call_id: Option<String>,
    /// Tool calls made by assistant
    pub tool_calls: Vec<ToolCall>,
}

impl LLMMessage {
    /// Create a system message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    /// Create a user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    /// Create an assistant message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    /// Create an assistant message with tool calls.
    #[must_use]
    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_calls,
        }
    }

    /// Create a tool result message.
    #[must_use]
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: vec![],
        }
    }
}

// ============================================================================
// TOOL TYPES
// ============================================================================

/// A tool that can be called by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// Parameters schema (JSON Schema)
    pub parameters: Value,
}

impl Tool {
    /// Create a new tool.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    /// Add a required string parameter.
    pub fn with_string_param(mut self, name: &str, description: &str) -> Self {
        if let Some(props) = self.parameters.get_mut("properties") {
            props[name] = serde_json::json!({
                "type": "string",
                "description": description
            });
        }
        if let Some(req) = self
            .parameters
            .get_mut("required")
            .and_then(|v| v.as_array_mut())
        {
            req.push(Value::String(name.to_string()));
        }
        self
    }

    /// Add an optional parameter.
    pub fn with_optional_param(mut self, name: &str, param_type: &str, description: &str) -> Self {
        if let Some(props) = self.parameters.get_mut("properties") {
            props[name] = serde_json::json!({
                "type": param_type,
                "description": description
            });
        }
        self
    }
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this call
    pub id: String,
    /// Name of the tool to call
    pub name: String,
    /// Arguments as JSON
    pub arguments: Value,
}

impl ToolCall {
    /// Create a new tool call.
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

/// Token usage statistics.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Tokens in the prompt
    pub prompt: u32,
    /// Tokens in the completion
    pub completion: u32,
    /// Total tokens used
    pub total: u32,
}

/// Reason for completion finishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// Normal stop (end of response)
    Stop,
    /// Hit max tokens limit
    Length,
    /// The model requested tool execution.
    ToolCalls,
    /// Error occurred
    Error,
}

/// Response from an LLM.
#[derive(Debug, Clone)]
pub struct LLMResponse {
    /// Generated content
    pub content: String,
    /// Token usage statistics
    pub tokens_used: TokenUsage,
    /// Reason for finishing
    pub finish_reason: FinishReason,
    /// Model that generated the response
    pub model: String,
    /// Tool calls requested by the LLM
    pub tool_calls: Vec<ToolCall>,
}

impl LLMResponse {
    /// Check if the response contains tool calls.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// A chunk from streaming response.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// Partial content
    pub content: String,
    /// Whether this is the final chunk
    pub done: bool,
    /// Token usage (only available on final chunk)
    pub tokens_used: Option<TokenUsage>,
    /// Finish reason (only available on final chunk)
    pub finish_reason: Option<FinishReason>,
}

/// Trait for LLM adapters.
///
/// Implement this trait to add support for a new LLM provider.
#[async_trait]
pub trait LLMAdapter: Send + Sync {
    /// Get the provider name (e.g., "openai", "anthropic").
    fn provider(&self) -> &str;

    /// Get the model name being used.
    fn model(&self) -> &str;

    /// Generate a completion from messages.
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError>;

    /// Generate a completion with tool support.
    ///
    /// The LLM can request to call tools, which should be executed
    /// and results passed back in subsequent messages.
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    async fn generate_with_tools(
        &self,
        messages: &[LLMMessage],
        tools: &[Tool],
    ) -> Result<LLMResponse, LLMError> {
        // Default: ignore tools and call regular generate
        let _ = tools;
        self.generate(messages).await
    }

    /// Generate a streaming completion.
    ///
    /// Returns a stream of chunks that can be processed as they arrive.
    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>>;

    /// Check if the LLM provider is accessible.
    ///
    /// # Errors
    ///
    /// Returns an error if the health check fails.
    async fn health_check(&self) -> Result<bool, LLMError>;

    /// Check if this adapter supports tool calling.
    fn supports_tools(&self) -> bool {
        false
    }
}

// ============================================================================
// EMBEDDINGS
// ============================================================================

/// Trait for text embedding models.
///
/// Embeddings convert text into dense vector representations that capture
/// semantic meaning. These vectors can be used for:
/// - Semantic search (finding similar content)
/// - Skill routing (matching user queries to skills)
/// - Clustering and classification
///
/// # Example
///
/// ```rust,ignore
/// use dasein_agentic_llm::{Embedder, OpenAIEmbedder};
///
/// let embedder = OpenAIEmbedder::new("sk-...");
/// let vector = embedder.embed("Hello world").await?;
/// println!("Embedding dimension: {}", vector.len());
/// ```
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Get the provider name (e.g., "openai", "gemini").
    fn provider(&self) -> &str;

    /// Get the model name being used.
    fn model(&self) -> &str;

    /// Get the dimension of the embedding vectors.
    fn dimension(&self) -> usize;

    /// Generate an embedding for a single text.
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LLMError>;

    /// Generate embeddings for multiple texts (batch).
    ///
    /// More efficient than calling `embed` multiple times.
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LLMError> {
        // Default implementation: call embed for each text
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }
}
