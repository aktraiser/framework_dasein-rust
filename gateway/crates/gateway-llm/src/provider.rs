//! LLM Provider abstraction

use async_trait::async_trait;
use gateway_core::llm::{
    ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse,
};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

/// Provider error types
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error: {status} - {message}")]
    Api { status: u16, message: String },

    #[error("Rate limited: retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Model not supported: {0}")]
    ModelNotSupported(String),

    #[error("Provider unavailable: {0}")]
    Unavailable(String),
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// Provider trait for LLM backends
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider name (e.g., "openai", "anthropic", "groq")
    fn name(&self) -> &str;

    /// List of supported models
    fn supported_models(&self) -> &[&str];

    /// Check if provider supports a specific model
    fn supports_model(&self, model: &str) -> bool {
        self.supported_models().iter().any(|m| *m == model)
    }

    /// Send chat completion request
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> ProviderResult<ChatCompletionResponse>;

    /// Send chat completion with streaming
    async fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
    ) -> ProviderResult<std::pin::Pin<Box<dyn futures::Stream<Item = ProviderResult<String>> + Send>>>;

    /// Generate embeddings
    async fn embeddings(&self, request: EmbeddingRequest) -> ProviderResult<EmbeddingResponse>;

    /// Health check
    async fn health_check(&self) -> bool {
        true
    }
}

/// Registry of available providers
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(name).cloned()
    }

    pub fn find_for_model(&self, model: &str) -> Option<Arc<dyn Provider>> {
        self.providers
            .values()
            .find(|p| p.supports_model(model))
            .cloned()
    }

    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn Provider>> {
        self.providers.values()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
