//! Ollama adapter for local LLM models.

use async_trait::async_trait;
use futures::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tracing::{debug, instrument};

use crate::{
    error::LLMError,
    traits::{FinishReason, LLMAdapter, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage},
};

/// Ollama adapter for local models.
pub struct OllamaAdapter {
    client: Client,
    base_url: String,
    model: String,
    temperature: f32,
}

impl OllamaAdapter {
    /// Create a new Ollama adapter.
    ///
    /// # Arguments
    ///
    /// * `model` - Model to use (e.g., "llama3.2", "qwen2.5-coder")
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: "http://localhost:11434".to_string(),
            model: model.into(),
            temperature: 0.7,
        }
    }

    /// Set the base URL for Ollama server.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Set the temperature for generation.
    #[must_use]
    pub const fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }
}

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

impl From<&LLMMessage> for OllamaMessage {
    fn from(msg: &LLMMessage) -> Self {
        Self {
            role: match msg.role {
                Role::System => "system".to_string(),
                Role::User => "user".to_string(),
                Role::Assistant => "assistant".to_string(),
            },
            content: msg.content.clone(),
        }
    }
}

#[async_trait]
impl LLMAdapter for OllamaAdapter {
    fn provider(&self) -> &'static str {
        "ollama"
    }

    fn model(&self) -> &str {
        &self.model
    }

    #[instrument(skip(self, messages), fields(provider = "ollama", model = %self.model))]
    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        debug!("Generating completion with {} messages", messages.len());

        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: messages.iter().map(OllamaMessage::from).collect(),
            stream: false,
            options: OllamaOptions {
                temperature: self.temperature,
            },
        };

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await
            .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(LLMError::ApiError(format!(
                "Ollama returned status {}",
                response.status()
            )));
        }

        let chat_response: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;

        let prompt_tokens = chat_response.prompt_eval_count.unwrap_or(0);
        let completion_tokens = chat_response.eval_count.unwrap_or(0);

        Ok(LLMResponse {
            content: chat_response.message.content,
            tokens_used: TokenUsage {
                prompt: prompt_tokens,
                completion: completion_tokens,
                total: prompt_tokens + completion_tokens,
            },
            finish_reason: FinishReason::Stop,
            model: self.model.clone(),
        })
    }

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: messages.iter().map(OllamaMessage::from).collect(),
            stream: true,
            options: OllamaOptions {
                temperature: self.temperature,
            },
        };

        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url);

        Box::pin(async_stream::try_stream! {
            let response = client
                .post(&url)
                .json(&request)
                .send()
                .await
                .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

            let mut stream = response.bytes_stream();

            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.map_err(|e| LLMError::ConnectionError(e.to_string()))?;
                let text = String::from_utf8_lossy(&bytes);

                for line in text.lines() {
                    if line.is_empty() {
                        continue;
                    }

                    if let Ok(response) = serde_json::from_str::<OllamaChatResponse>(line) {
                        yield StreamChunk {
                            content: response.message.content,
                            done: response.done,
                            tokens_used: if response.done {
                                Some(TokenUsage {
                                    prompt: response.prompt_eval_count.unwrap_or(0),
                                    completion: response.eval_count.unwrap_or(0),
                                    total: response.prompt_eval_count.unwrap_or(0)
                                        + response.eval_count.unwrap_or(0),
                                })
                            } else {
                                None
                            },
                            finish_reason: if response.done {
                                Some(FinishReason::Stop)
                            } else {
                                None
                            },
                        };
                    }
                }
            }
        })
    }

    async fn health_check(&self) -> Result<bool, LLMError> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

        Ok(response.status().is_success())
    }
}
