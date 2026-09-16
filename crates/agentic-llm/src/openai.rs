//! `OpenAI` adapter implementation.

use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessage,
        ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessage, ChatCompletionRequestToolMessage,
        ChatCompletionRequestUserMessage, ChatCompletionTool, ChatCompletionToolType,
        CreateChatCompletionRequest, CreateEmbeddingRequestArgs, EmbeddingInput, FunctionCall,
        FunctionObject,
    },
    Client,
};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use tracing::{debug, instrument};

use crate::{
    error::LLMError,
    traits::{
        Embedder, FinishReason, LLMAdapter, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage,
        Tool, ToolCall,
    },
};

/// `OpenAI` adapter for GPT models.
pub struct OpenAIAdapter {
    client: Client<OpenAIConfig>,
    responses: Option<crate::OpenAIResponsesAdapter>,
    model: String,
    temperature: f32,
    max_tokens: Option<u32>,
}

impl OpenAIAdapter {
    /// Create a new `OpenAI` adapter.
    ///
    /// # Arguments
    ///
    /// * `api_key` - `OpenAI` API key
    /// * `model` - Model to use (e.g., "gpt-4o", "gpt-4o-mini")
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let api_key = api_key.into();
        let model = model.into();
        let responses = crate::OpenAIResponsesAdapter::required(&model)
            .then(|| crate::OpenAIResponsesAdapter::new(&api_key, &model));
        let config = OpenAIConfig::new().with_api_key(api_key);
        Self {
            client: Client::with_config(config),
            model,
            responses,
            temperature: 0.7,
            max_tokens: None,
        }
    }

    /// Set an OpenAI-compatible base URL, including its API version prefix.
    #[must_use]
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        let api_base = api_base.into();
        self.responses = self.responses.map(|r| r.with_api_base(&api_base));
        self.client = Client::with_config(self.client.config().clone().with_api_base(api_base));
        self
    }

    /// Set temperature for legacy Chat Completions. Responses reasoning models
    /// omit temperature and keep their provider defaults.
    #[must_use]
    pub const fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Set the maximum tokens for generation.
    #[must_use]
    pub fn with_prompt_cache_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.responses = self.responses.map(|r| r.with_prompt_cache_key(key));
        self
    }
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.responses = self.responses.map(|r| r.with_max_tokens(max_tokens));
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Convert our message format to `OpenAI`'s format.
    fn convert_messages(messages: &[LLMMessage]) -> Vec<ChatCompletionRequestMessage> {
        messages
            .iter()
            .map(|msg| match msg.role {
                Role::System => {
                    ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                        content: msg.content.clone().into(),
                        ..Default::default()
                    })
                }
                Role::User => {
                    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                        content: msg.content.clone().into(),
                        ..Default::default()
                    })
                }
                Role::Assistant => {
                    ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                        content: Some(ChatCompletionRequestAssistantMessageContent::Text(
                            msg.content.clone(),
                        )),
                        tool_calls: if msg.tool_calls.is_empty() {
                            None
                        } else {
                            Some(
                                msg.tool_calls
                                    .iter()
                                    .map(|t| ChatCompletionMessageToolCall {
                                        id: t.id.clone(),
                                        r#type: ChatCompletionToolType::Function,
                                        function: FunctionCall {
                                            name: t.name.clone(),
                                            arguments: t.arguments.to_string(),
                                        },
                                    })
                                    .collect(),
                            )
                        },
                        ..Default::default()
                    })
                }
                Role::Tool => {
                    ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                        content: msg.content.clone().into(),
                        tool_call_id: msg.tool_call_id.clone().unwrap_or_default(),
                    })
                }
            })
            .collect()
    }
}

#[async_trait]
impl LLMAdapter for OpenAIAdapter {
    fn provider(&self) -> &'static str {
        "openai"
    }

    fn model(&self) -> &str {
        &self.model
    }

    #[instrument(skip(self, messages), fields(provider = "openai", model = %self.model))]
    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        self.generate_with_tools(messages, &[]).await
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn generate_with_tools(
        &self,
        messages: &[LLMMessage],
        tools: &[Tool],
    ) -> Result<LLMResponse, LLMError> {
        if let Some(r) = &self.responses {
            return r.generate_with_tools(messages, tools).await;
        }
        debug!("Generating completion with {} messages", messages.len());

        let request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: Self::convert_messages(messages),
            temperature: Some(self.temperature),
            max_completion_tokens: self.max_tokens,
            tools: if tools.is_empty() {
                None
            } else {
                Some(
                    tools
                        .iter()
                        .map(|t| ChatCompletionTool {
                            r#type: ChatCompletionToolType::Function,
                            function: FunctionObject {
                                name: t.name.clone(),
                                description: Some(t.description.clone()),
                                parameters: Some(t.parameters.clone()),
                                strict: None,
                            },
                        })
                        .collect(),
                )
            },
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
                details: usage.and_then(|u| serde_json::to_value(u).ok()),
                prompt: usage.map_or(0, |u| u.prompt_tokens),
                completion: usage.map_or(0, |u| u.completion_tokens),
                total: usage.map_or(0, |u| u.total_tokens),
            },
            finish_reason: match choice.finish_reason {
                Some(async_openai::types::FinishReason::Length) => FinishReason::Length,
                Some(async_openai::types::FinishReason::ToolCalls) => FinishReason::ToolCalls,
                _ => FinishReason::Stop,
            },
            model: response.model,
            tool_calls: choice
                .message
                .tool_calls
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|t| {
                    let arguments = serde_json::from_str(&t.function.arguments)
                        .map_err(|e| LLMError::ApiError(format!("Invalid tool arguments: {e}")))?;
                    Ok(ToolCall::new(&t.id, &t.function.name, arguments))
                })
                .collect::<Result<_, LLMError>>()?,
        })
    }

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        if let Some(r) = &self.responses {
            return r.generate_stream(messages);
        }
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
                            async_openai::types::FinishReason::Length => FinishReason::Length,
                            _ => FinishReason::Stop,
                        }),
                    };
                }
            }
        })
    }

    async fn health_check(&self) -> Result<bool, LLMError> {
        if let Some(r) = &self.responses {
            return r.health_check().await;
        }
        self.client
            .models()
            .list()
            .await
            .map(|_| true)
            .map_err(|e| LLMError::ConnectionError(e.to_string()))
    }
}

// ============================================================================
// EMBEDDINGS
// ============================================================================

/// OpenAI embedder for text-embedding models.
///
/// Converts text to dense vector representations using OpenAI's embedding models.
/// These vectors can be used for semantic search, skill routing, and similarity matching.
///
/// # Example
///
/// ```rust,ignore
/// use dasein_agentic_llm::OpenAIEmbedder;
///
/// let embedder = OpenAIEmbedder::new("sk-...");
/// let vector = embedder.embed("Find emails about meetings").await?;
/// ```
pub struct OpenAIEmbedder {
    client: Client<OpenAIConfig>,
    model: String,
}

impl OpenAIEmbedder {
    /// Create a new OpenAI embedder with the default model (text-embedding-3-small).
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_model(api_key, "text-embedding-3-small")
    }

    /// Create a new OpenAI embedder with a specific model.
    ///
    /// # Arguments
    ///
    /// * `api_key` - OpenAI API key
    /// * `model` - Model to use (e.g., "text-embedding-3-small", "text-embedding-3-large")
    #[must_use]
    pub fn with_model(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        Self {
            client: Client::with_config(config),
            model: model.into(),
        }
    }

    /// Get the embedding dimension for the current model.
    fn get_dimension(&self) -> usize {
        match self.model.as_str() {
            "text-embedding-3-large" => 3072,
            "text-embedding-3-small" => 1536,
            "text-embedding-ada-002" => 1536,
            _ => 1536, // Default
        }
    }
}

#[async_trait]
impl Embedder for OpenAIEmbedder {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn dimension(&self) -> usize {
        self.get_dimension()
    }

    #[instrument(skip(self, text), fields(provider = "openai", model = %self.model))]
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LLMError> {
        debug!("Generating embedding for text of {} chars", text.len());

        let request = CreateEmbeddingRequestArgs::default()
            .model(&self.model)
            .input(EmbeddingInput::String(text.to_string()))
            .build()
            .map_err(|e| LLMError::ConfigError(e.to_string()))?;

        let response = self
            .client
            .embeddings()
            .create(request)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;

        response
            .data
            .first()
            .map(|e| e.embedding.clone())
            .ok_or(LLMError::EmptyResponse)
    }

    #[instrument(skip(self, texts), fields(provider = "openai", model = %self.model, batch_size = texts.len()))]
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LLMError> {
        debug!("Generating embeddings for {} texts", texts.len());

        let input: Vec<String> = texts.iter().map(|s| (*s).to_string()).collect();

        let request = CreateEmbeddingRequestArgs::default()
            .model(&self.model)
            .input(EmbeddingInput::StringArray(input))
            .build()
            .map_err(|e| LLMError::ConfigError(e.to_string()))?;

        let response = self
            .client
            .embeddings()
            .create(request)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;

        Ok(response.data.into_iter().map(|e| e.embedding).collect())
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

#[cfg(test)]
mod audit_regressions {
    #[tokio::test]
    async fn tools_are_sent_and_parsed_over_http() {
        use super::*;
        use wiremock::{
            matchers::{body_partial_json, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({"tools":[{"type":"function", "function":{"name":"lookup","parameters":{"type":"object"}}}]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id":"test", "object":"chat.completion", "created":0, "model":"mock-model",
                "choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call1","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"}],
                "usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}
            }))).expect(1).mount(&server).await;
        let adapter = OpenAIAdapter::new("fake-local-test-key", "mock-model")
            .with_api_base(format!("{}/v1", server.uri()));
        let response = adapter
            .generate_with_tools(
                &[LLMMessage::user("lookup")],
                &[Tool::new("lookup", "lookup")],
            )
            .await
            .unwrap();
        assert_eq!(response.tool_calls[0].id, "call1");
        assert_eq!(response.tool_calls[0].arguments, serde_json::json!({}));
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        server.verify().await;
    }

    use super::*;
    #[test]
    fn audit_tool_result_survives_conversion() {
        let messages = vec![
            LLMMessage::user("question"),
            LLMMessage::tool_result("call1", "42"),
        ];
        let converted = OpenAIAdapter::convert_messages(&messages);
        assert_eq!(converted.len(), 2, "tool result silently dropped");
    }
    #[test]
    fn audit_assistant_tool_call_survives_conversion() {
        let messages = vec![LLMMessage::assistant_with_tool_calls(
            "",
            vec![crate::traits::ToolCall::new(
                "call1",
                "lookup",
                serde_json::json!({}),
            )],
        )];
        let converted = OpenAIAdapter::convert_messages(&messages);
        let json = serde_json::to_value(&converted[0]).unwrap();
        assert_eq!(
            json["tool_calls"][0]["id"], "call1",
            "assistant tool call silently dropped"
        );
    }
}
