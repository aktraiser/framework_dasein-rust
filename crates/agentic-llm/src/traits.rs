//! Core traits and types for LLM adapters.

use async_trait::async_trait;
use futures::Stream;
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
}

/// A message in a conversation.
#[derive(Debug, Clone)]
pub struct LLMMessage {
    /// Role of the message sender
    pub role: Role,
    /// Content of the message
    pub content: String,
}

impl LLMMessage {
    /// Create a system message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// Create a user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Create an assistant message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
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
}
