//! Anthropic Claude provider implementation

use crate::provider::{Provider, ProviderError, ProviderResult};
use async_trait::async_trait;
use gateway_core::llm::{
    ChatCompletionRequest, ChatCompletionResponse, ChatChoice, ChatMessage, ChatRole,
    EmbeddingRequest, EmbeddingResponse, Usage,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tracing::{debug, instrument};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

const SUPPORTED_MODELS: &[&str] = &[
    "claude-3-5-sonnet-20241022",
    "claude-3-5-haiku-20241022",
    "claude-3-opus-20240229",
    "claude-3-sonnet-20240229",
    "claude-3-haiku-20240307",
];

/// Anthropic Claude provider
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    api_base: String,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_base: ANTHROPIC_API_BASE.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.api_base = base_url.into();
        self
    }

    fn convert_to_anthropic_request(&self, request: &ChatCompletionRequest) -> AnthropicRequest {
        let mut system = None;
        let mut messages = Vec::new();

        for msg in &request.messages {
            match msg.role {
                ChatRole::System => {
                    system = Some(msg.content.clone());
                }
                ChatRole::User => {
                    messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: msg.content.clone(),
                    });
                }
                ChatRole::Assistant => {
                    messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: msg.content.clone(),
                    });
                }
                ChatRole::Tool => {
                    // Tool messages handled differently in Anthropic
                    messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: format!("[Tool Result]: {}", msg.content),
                    });
                }
            }
        }

        AnthropicRequest {
            model: request.model.clone(),
            max_tokens: request.max_tokens.unwrap_or(4096),
            messages,
            system,
            temperature: request.temperature,
            stop_sequences: request.stop.clone(),
        }
    }

    fn convert_from_anthropic_response(
        &self,
        response: AnthropicResponse,
        model: String,
    ) -> ChatCompletionResponse {
        let content = response
            .content
            .into_iter()
            .filter_map(|c| {
                if c.content_type == "text" {
                    Some(c.text)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        ChatCompletionResponse {
            id: response.id,
            object: "chat.completion".to_string(),
            created: chrono::Utc::now().timestamp(),
            model,
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content,
                    name: None,
                },
                finish_reason: Some(response.stop_reason.unwrap_or_else(|| "stop".to_string())),
            }],
            usage: Some(Usage {
                prompt_tokens: response.usage.input_tokens,
                completion_tokens: response.usage.output_tokens,
                total_tokens: response.usage.input_tokens + response.usage.output_tokens,
            }),
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn supported_models(&self) -> &[&str] {
        SUPPORTED_MODELS
    }

    #[instrument(skip(self, request), fields(model = %request.model))]
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> ProviderResult<ChatCompletionResponse> {
        debug!("Sending chat completion request to Anthropic");

        let model = request.model.clone();
        let anthropic_request = self.convert_to_anthropic_request(&request);

        let response = self
            .client
            .post(format!("{}/messages", self.api_base))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
            .json(&anthropic_request)
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

        let anthropic_response: AnthropicResponse = response.json().await?;
        Ok(self.convert_from_anthropic_response(anthropic_response, model))
    }

    async fn chat_completion_stream(
        &self,
        _request: ChatCompletionRequest,
    ) -> ProviderResult<Pin<Box<dyn futures::Stream<Item = ProviderResult<String>> + Send>>> {
        Err(ProviderError::Unavailable(
            "Streaming not yet implemented".to_string(),
        ))
    }

    async fn embeddings(&self, _request: EmbeddingRequest) -> ProviderResult<EmbeddingResponse> {
        Err(ProviderError::Unavailable(
            "Anthropic does not support embeddings".to_string(),
        ))
    }

    async fn health_check(&self) -> bool {
        // Anthropic doesn't have a simple health endpoint
        // We could try a minimal request, but for now just return true
        true
    }
}

// Anthropic API types

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    content: Vec<ContentBlock>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}
