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

impl ProviderError {
    /// Whether sending the same request again is both useful and safe: the
    /// provider could not be reached, was overloaded or asked us to slow
    /// down. A request it rejected (bad input, missing permission) will be
    /// rejected again; one that timed out or broke after it was sent may
    /// already have been processed and billed, so it is not replayed.
    pub fn is_retriable(&self) -> bool {
        match self {
            Self::Http(error) => error.is_connect(),
            Self::RateLimited { .. } => true,
            Self::Api { status, .. } => *status >= 500,
            Self::AuthFailed | Self::ModelNotSupported(_) | Self::Unavailable(_) => false,
        }
    }
}

/// Delay a provider asks for in its `Retry-After` header (in seconds), or one
/// second when it gives none.
pub(crate) fn retry_after_ms(response: &reqwest::Response) -> u64 {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or(1000, |seconds| seconds.saturating_mul(1000))
}

pub type CompletionStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = ProviderResult<String>> + Send>>;

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
        self.supported_models().contains(&model)
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
    model_routes: HashMap<String, String>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            model_routes: HashMap::new(),
        }
    }

    /// Associate a configured model with its explicitly selected provider.
    pub fn route_model(&mut self, model: String, provider: String) {
        self.model_routes.insert(model, provider);
    }

    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(name).cloned()
    }

    pub fn find_for_model(&self, model: &str) -> Option<Arc<dyn Provider>> {
        if let Some(provider) = self.model_routes.get(model) {
            return self.get(provider);
        }
        let mut providers: Vec<_> = self
            .providers
            .values()
            .filter(|p| p.supports_model(model))
            .collect();
        providers.sort_by_key(|p| p.name());
        providers.first().map(|p| Arc::clone(p))
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

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn only_calls_that_never_reached_the_provider_are_replayed() {
        // Nothing listens here: the request was never sent.
        let refused = reqwest::Client::new()
            .get("http://127.0.0.1:1/")
            .send()
            .await
            .unwrap_err();
        assert!(ProviderError::Http(refused).is_retriable());

        // The provider received the request and is slow to answer: it may
        // well be working on it (and billing it).
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(5)))
            .mount(&server)
            .await;
        let timed_out = reqwest::Client::new()
            .get(server.uri())
            .timeout(std::time::Duration::from_millis(100))
            .send()
            .await
            .unwrap_err();
        assert!(timed_out.is_timeout());
        assert!(!ProviderError::Http(timed_out).is_retriable());

        assert!(ProviderError::RateLimited { retry_after_ms: 1 }.is_retriable());
        let api = |status| ProviderError::Api {
            status,
            message: String::new(),
        };
        assert!(api(503).is_retriable());
        assert!(!api(400).is_retriable());
        assert!(!ProviderError::AuthFailed.is_retriable());
    }
}
