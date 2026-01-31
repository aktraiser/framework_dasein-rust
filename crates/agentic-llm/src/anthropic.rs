//! Anthropic Claude adapter implementation.

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

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic adapter for Claude models.
pub struct AnthropicAdapter {
    client: Client,
    api_key: String,
    model: String,
    temperature: f32,
    max_tokens: u32,
}

impl AnthropicAdapter {
    /// Create a new Anthropic adapter.
    ///
    /// # Arguments
    ///
    /// * `api_key` - Anthropic API key
    /// * `model` - Model to use (e.g., "claude-sonnet-4-20250514", "claude-3-5-haiku-20241022")
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            temperature: 0.7,
            max_tokens: 4096,
        }
    }

    /// Set the temperature for generation.
    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Set the maximum tokens for generation.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Extract system message and convert remaining messages.
    fn prepare_messages(messages: &[LLMMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        let system = messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.clone());

        let messages = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(AnthropicMessage::from)
            .collect();

        (system, messages)
    }
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    model: String,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<AnthropicDelta>,
    /// Message with usage info (reserved for future use)
    #[serde(default)]
    #[allow(dead_code)]
    message: Option<AnthropicStreamMessage>,
}

#[derive(Deserialize, Default)]
struct AnthropicDelta {
    #[serde(default)]
    text: String,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AnthropicStreamMessage {
    /// Token usage (reserved for future use)
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct AnthropicError {
    error: AnthropicErrorDetail,
}

#[derive(Deserialize)]
struct AnthropicErrorDetail {
    message: String,
}

impl From<&LLMMessage> for AnthropicMessage {
    fn from(msg: &LLMMessage) -> Self {
        Self {
            role: match msg.role {
                Role::System => "user".to_string(), // System handled separately
                Role::User => "user".to_string(),
                Role::Assistant => "assistant".to_string(),
            },
            content: msg.content.clone(),
        }
    }
}

#[async_trait]
impl LLMAdapter for AnthropicAdapter {
    fn provider(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
    }

    #[instrument(skip(self, messages), fields(provider = "anthropic", model = %self.model))]
    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        debug!("Generating completion with {} messages", messages.len());

        let (system, api_messages) = Self::prepare_messages(messages);

        let request = AnthropicRequest {
            model: self.model.clone(),
            messages: api_messages,
            max_tokens: self.max_tokens,
            system,
            temperature: Some(self.temperature),
            stream: None,
        };

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

        if !response.status().is_success() {
            let error: AnthropicError = response
                .json()
                .await
                .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;
            return Err(LLMError::ApiError(error.error.message));
        }

        let api_response: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;

        let content = api_response
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");

        let finish_reason = match api_response.stop_reason.as_deref() {
            Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        Ok(LLMResponse {
            content,
            tokens_used: TokenUsage {
                prompt: api_response.usage.input_tokens,
                completion: api_response.usage.output_tokens,
                total: api_response.usage.input_tokens + api_response.usage.output_tokens,
            },
            finish_reason,
            model: api_response.model,
        })
    }

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        let (system, api_messages) = Self::prepare_messages(messages);

        let request = AnthropicRequest {
            model: self.model.clone(),
            messages: api_messages,
            max_tokens: self.max_tokens,
            system,
            temperature: Some(self.temperature),
            stream: Some(true),
        };

        let client = self.client.clone();
        let api_key = self.api_key.clone();

        Box::pin(async_stream::try_stream! {
            let response = client
                .post(ANTHROPIC_API_URL)
                .header("x-api-key", &api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
                .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

            let status = response.status();
            if !status.is_success() {
                Err(LLMError::ApiError(format!("API returned status {}", status)))?;
            }

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.map_err(|e| LLMError::ConnectionError(e.to_string()))?;
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // Process complete SSE events
                while let Some(event_end) = buffer.find("\n\n") {
                    let event_data = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    for line in event_data.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(data) {
                                match event.event_type.as_str() {
                                    "content_block_delta" => {
                                        if let Some(delta) = event.delta {
                                            yield StreamChunk {
                                                content: delta.text,
                                                done: false,
                                                tokens_used: None,
                                                finish_reason: None,
                                            };
                                        }
                                    }
                                    "message_delta" => {
                                        if let Some(delta) = event.delta {
                                            let finish_reason = match delta.stop_reason.as_deref() {
                                                Some("end_turn") => Some(FinishReason::Stop),
                                                Some("max_tokens") => Some(FinishReason::Length),
                                                _ => None,
                                            };
                                            yield StreamChunk {
                                                content: String::new(),
                                                done: true,
                                                tokens_used: None,
                                                finish_reason,
                                            };
                                        }
                                    }
                                    "message_stop" => {
                                        yield StreamChunk {
                                            content: String::new(),
                                            done: true,
                                            tokens_used: None,
                                            finish_reason: Some(FinishReason::Stop),
                                        };
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    async fn health_check(&self) -> Result<bool, LLMError> {
        // Send a minimal request to check API connectivity
        let request = AnthropicRequest {
            model: self.model.clone(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
            }],
            max_tokens: 1,
            system: None,
            temperature: None,
            stream: None,
        };

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

        Ok(response.status().is_success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_preparation() {
        let messages = vec![
            LLMMessage::system("You are helpful."),
            LLMMessage::user("Hello"),
            LLMMessage::assistant("Hi there!"),
        ];

        let (system, api_messages) = AnthropicAdapter::prepare_messages(&messages);

        assert_eq!(system, Some("You are helpful.".to_string()));
        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[1].role, "assistant");
    }
}
