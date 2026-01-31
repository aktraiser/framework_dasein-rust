//! OpenAI adapter implementation.

use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
        ChatCompletionRequestUserMessage, CreateChatCompletionRequest,
    },
    Client,
};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use tracing::{debug, instrument};

use crate::{
    error::LLMError,
    traits::{FinishReason, LLMAdapter, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage},
};

/// OpenAI adapter for GPT models.
pub struct OpenAIAdapter {
    client: Client<OpenAIConfig>,
    model: String,
    temperature: f32,
    max_tokens: Option<u32>,
}

impl OpenAIAdapter {
    /// Create a new OpenAI adapter.
    ///
    /// # Arguments
    ///
    /// * `api_key` - OpenAI API key
    /// * `model` - Model to use (e.g., "gpt-4o", "gpt-4o-mini")
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        Self {
            client: Client::with_config(config),
            model: model.into(),
            temperature: 0.7,
            max_tokens: None,
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
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Convert our message format to OpenAI's format.
    fn convert_messages(messages: &[LLMMessage]) -> Vec<ChatCompletionRequestMessage> {
        messages
            .iter()
            .map(|msg| match msg.role {
                Role::System => ChatCompletionRequestMessage::System(
                    ChatCompletionRequestSystemMessage {
                        content: msg.content.clone().into(),
                        ..Default::default()
                    },
                ),
                Role::User => {
                    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                        content: msg.content.clone().into(),
                        ..Default::default()
                    })
                }
                Role::Assistant => ChatCompletionRequestMessage::Assistant(
                    ChatCompletionRequestAssistantMessage {
                        content: Some(ChatCompletionRequestAssistantMessageContent::Text(
                            msg.content.clone(),
                        )),
                        ..Default::default()
                    },
                ),
            })
            .collect()
    }
}

#[async_trait]
impl LLMAdapter for OpenAIAdapter {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model(&self) -> &str {
        &self.model
    }

    #[instrument(skip(self, messages), fields(provider = "openai", model = %self.model))]
    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        debug!("Generating completion with {} messages", messages.len());

        let request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: Self::convert_messages(messages),
            temperature: Some(self.temperature),
            max_completion_tokens: self.max_tokens,
            ..Default::default()
        };

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;

        let choice = response.choices.first().ok_or(LLMError::EmptyResponse)?;

        let content = choice.message.content.clone().unwrap_or_default();

        let usage = response.usage.as_ref();

        Ok(LLMResponse {
            content,
            tokens_used: TokenUsage {
                prompt: usage.map_or(0, |u| u.prompt_tokens),
                completion: usage.map_or(0, |u| u.completion_tokens),
                total: usage.map_or(0, |u| u.total_tokens),
            },
            finish_reason: match choice.finish_reason {
                Some(async_openai::types::FinishReason::Stop) => FinishReason::Stop,
                Some(async_openai::types::FinishReason::Length) => FinishReason::Length,
                _ => FinishReason::Stop,
            },
            model: response.model,
        })
    }

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        let request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: Self::convert_messages(messages),
            temperature: Some(self.temperature),
            max_completion_tokens: self.max_tokens,
            stream: Some(true),
            ..Default::default()
        };

        Box::pin(async_stream::try_stream! {
            let mut stream = self
                .client
                .chat()
                .create_stream(request)
                .await
                .map_err(|e| LLMError::ApiError(e.to_string()))?;

            while let Some(result) = stream.next().await {
                let response = result.map_err(|e| LLMError::ApiError(e.to_string()))?;

                if let Some(choice) = response.choices.first() {
                    let content = choice.delta.content.clone().unwrap_or_default();
                    let done = choice.finish_reason.is_some();

                    yield StreamChunk {
                        content,
                        done,
                        tokens_used: None,
                        finish_reason: choice.finish_reason.map(|r| match r {
                            async_openai::types::FinishReason::Stop => FinishReason::Stop,
                            async_openai::types::FinishReason::Length => FinishReason::Length,
                            _ => FinishReason::Stop,
                        }),
                    };
                }
            }
        })
    }

    async fn health_check(&self) -> Result<bool, LLMError> {
        self.client
            .models()
            .list()
            .await
            .map(|_| true)
            .map_err(|e| LLMError::ConnectionError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_conversion() {
        let messages = vec![
            LLMMessage::system("You are helpful."),
            LLMMessage::user("Hello"),
        ];

        let converted = OpenAIAdapter::convert_messages(&messages);
        assert_eq!(converted.len(), 2);
    }
}
