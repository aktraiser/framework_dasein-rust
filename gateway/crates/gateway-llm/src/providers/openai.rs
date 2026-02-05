//! OpenAI provider implementation

use crate::provider::{Provider, ProviderError, ProviderResult};
use async_trait::async_trait;
use gateway_core::llm::{
    ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse,
};
use reqwest::Client;
use std::pin::Pin;
use tracing::{debug, instrument};

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

const SUPPORTED_MODELS: &[&str] = &[
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-4-turbo",
    "gpt-4",
    "gpt-3.5-turbo",
    "text-embedding-3-small",
    "text-embedding-3-large",
    "text-embedding-ada-002",
];

/// OpenAI provider
pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    api_base: String,
}

impl OpenAIProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_base: OPENAI_API_BASE.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.api_base = base_url.into();
        self
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn supported_models(&self) -> &[&str] {
        SUPPORTED_MODELS
    }

    #[instrument(skip(self, request), fields(model = %request.model))]
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> ProviderResult<ChatCompletionResponse> {
        debug!("Sending chat completion request to OpenAI");

        let response = self
            .client
            .post(format!("{}/chat/completions", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                return Err(ProviderError::RateLimited {
                    retry_after_ms: 1000,
                });
            }
            if status.as_u16() == 401 {
                return Err(ProviderError::AuthFailed);
            }

            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let completion: ChatCompletionResponse = response.json().await?;
        Ok(completion)
    }

    async fn chat_completion_stream(
        &self,
        _request: ChatCompletionRequest,
    ) -> ProviderResult<Pin<Box<dyn futures::Stream<Item = ProviderResult<String>> + Send>>> {
        // Streaming implementation would use SSE
        Err(ProviderError::Unavailable(
            "Streaming not yet implemented".to_string(),
        ))
    }

    #[instrument(skip(self, request), fields(model = %request.model))]
    async fn embeddings(&self, request: EmbeddingRequest) -> ProviderResult<EmbeddingResponse> {
        debug!("Sending embeddings request to OpenAI");

        let response = self
            .client
            .post(format!("{}/embeddings", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let embeddings: EmbeddingResponse = response.json().await?;
        Ok(embeddings)
    }

    async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/models", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}
