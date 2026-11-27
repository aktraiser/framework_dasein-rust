//! Anthropic Claude adapter implementation.

use async_trait::async_trait;
use futures::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tracing::{debug, instrument};

use crate::{
    error::LLMError,
    traits::{
        FinishReason, LLMAdapter, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage, Tool,
        ToolCall,
    },
};

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic adapter for Claude models.
pub struct AnthropicAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    temperature: f32,
    max_tokens: u32,
    max_retries: u32,
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
            client: crate::http::client(crate::http::DEFAULT_IDLE_TIMEOUT),
            api_key: api_key.into(),
            base_url: ANTHROPIC_BASE_URL.to_string(),
            model: model.into(),
            temperature: 0.7,
            max_tokens: 4096,
            max_retries: crate::http::DEFAULT_MAX_RETRIES,
        }
    }

    /// Send requests to another server (a proxy, a gateway, a test server)
    /// instead of `https://api.anthropic.com`. The adapter adds
    /// `/v1/messages` to it.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    /// Set how long the API may stay silent before a call fails (default 10
    /// minutes).
    #[must_use]
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.client = crate::http::client(timeout);
        self
    }

    /// Set how many times a call is repeated when the API cannot be reached,
    /// limits the rate or is overloaded.
    #[must_use]
    pub const fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// A POST to the Messages endpoint with the authentication headers.
    fn post(&self) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
    }

    /// All system messages, in order. The API takes one system prompt.
    fn system_prompt(messages: &[LLMMessage]) -> Option<String> {
        let parts: Vec<&str> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    fn usage(usage: &AnthropicUsage) -> TokenUsage {
        // Cached tokens are input too; the API reports them separately.
        let prompt = usage
            .input_tokens
            .saturating_add(usage.cache_read_input_tokens.unwrap_or(0))
            .saturating_add(usage.cache_creation_input_tokens.unwrap_or(0));
        TokenUsage {
            details: serde_json::to_value(usage).ok(),
            prompt,
            completion: usage.output_tokens,
            total: prompt.saturating_add(usage.output_tokens),
        }
    }

    fn finish_reason(stop_reason: Option<&str>) -> FinishReason {
        match stop_reason {
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        }
    }

    /// Set the temperature for generation.
    #[must_use]
    pub const fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Set the maximum tokens for generation.
    #[must_use]
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Extract system message and convert remaining messages.
    fn prepare_messages(messages: &[LLMMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        // No tool definitions go with these calls, so the API would reject
        // tool blocks: earlier tool rounds are written out as text.
        let api_messages = crate::history::tool_rounds_as_text(messages)
            .iter()
            .filter(|m| m.role != Role::System)
            .map(AnthropicMessage::from)
            .collect();

        (Self::system_prompt(messages), api_messages)
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
    /// Absent on blocks that are not text (thinking, tool use...).
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, serde::Serialize, Default, Clone)]
struct AnthropicUsage {
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    // Both default: stream events report input and output separately.
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<AnthropicDelta>,
    /// `message_start`: the message so far, with the input token counts.
    #[serde(default)]
    message: Option<AnthropicStreamMessage>,
    /// `message_delta`: the output token count.
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    /// `error`: the API failed after the stream had started.
    #[serde(default)]
    error: Option<AnthropicErrorDetail>,
}

#[derive(Deserialize, Default)]
struct AnthropicDelta {
    #[serde(default)]
    text: String,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicStreamMessage {
    #[serde(default)]
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct AnthropicErrorDetail {
    message: String,
}

// ============================================================================
// TOOL CALLING SUPPORT
// ============================================================================

#[derive(Serialize)]
struct AnthropicRequestWithTools {
    model: String,
    messages: Vec<AnthropicMessageWithContent>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct AnthropicMessageWithContent {
    role: String,
    content: AnthropicContentType,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicContentType {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
    /// A block type this adapter does not use (thinking, server tools...).
    /// Without it one such block would make the whole response unreadable.
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct AnthropicResponseWithTools {
    content: Vec<AnthropicContentBlock>,
    model: String,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

impl From<&LLMMessage> for AnthropicMessage {
    fn from(msg: &LLMMessage) -> Self {
        Self {
            role: match msg.role {
                Role::Assistant => "assistant".to_string(),
                // System handled separately, Tool acts as user for now
                Role::System | Role::User | Role::Tool => "user".to_string(),
            },
            content: msg.content.clone(),
        }
    }
}

#[async_trait]
impl LLMAdapter for AnthropicAdapter {
    fn provider(&self) -> &'static str {
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

        let response = crate::http::send(|| self.post().json(&request), self.max_retries).await?;

        let api_response: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.without_url().to_string()))?;

        let content = api_response
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<String>();

        Ok(LLMResponse {
            content,
            tokens_used: Self::usage(&api_response.usage),
            finish_reason: Self::finish_reason(api_response.stop_reason.as_deref()),
            model: api_response.model,
            tool_calls: vec![],
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

        Box::pin(async_stream::try_stream! {
            let mut response =
                crate::http::send(|| self.post().json(&request), self.max_retries).await?;

            let mut events = crate::http::SseBuffer::default();
            // Input tokens come with `message_start`, output tokens and the
            // stop reason with `message_delta`; the answer is over at
            // `message_stop`, which gets the one final chunk.
            let mut usage = AnthropicUsage::default();
            let mut stop_reason: Option<String> = None;
            let mut finished = false;
            loop {
                let chunk = response.chunk().await.map_err(crate::http::transport_error)?;
                let ready = match &chunk {
                    Some(bytes) => events.push(bytes)?,
                    None => events.finish().into_iter().collect(),
                };
                for data in ready {
                    let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(&data) else {
                        debug!("Skipping a stream event this adapter cannot read");
                        continue;
                    };
                    match event.event_type.as_str() {
                        "message_start" => {
                            if let Some(message) = event.message {
                                usage = message.usage;
                            }
                        }
                        "content_block_delta" => {
                            let text = event.delta.map(|delta| delta.text).unwrap_or_default();
                            // Thinking and tool-input deltas carry no text.
                            if !text.is_empty() {
                                yield StreamChunk {
                                    content: text,
                                    done: false,
                                    tokens_used: None,
                                    finish_reason: None,
                                };
                            }
                        }
                        "message_delta" => {
                            if let Some(delta) = event.delta {
                                stop_reason = delta.stop_reason.or(stop_reason);
                            }
                            if let Some(delta_usage) = event.usage {
                                usage.output_tokens = delta_usage.output_tokens;
                            }
                        }
                        "message_stop" => {
                            finished = true;
                            yield StreamChunk {
                                content: String::new(),
                                done: true,
                                tokens_used: Some(Self::usage(&usage)),
                                finish_reason: Some(Self::finish_reason(stop_reason.as_deref())),
                            };
                        }
                        "error" => {
                            let message = event
                                .error
                                .map_or_else(|| data.clone(), |error| error.message);
                            Err(LLMError::ApiError(message))?;
                        }
                        _ => {}
                    }
                }
                if chunk.is_none() {
                    break;
                }
            }
            if !finished {
                // Not a complete answer: do not let it pass for one.
                Err(LLMError::ConnectionError(
                    "stream ended before the model finished".into(),
                ))?;
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
            .post()
            .json(&request)
            .send()
            .await
            .map_err(crate::http::transport_error)?;

        Ok(response.status().is_success())
    }

    fn supports_tools(&self) -> bool {
        true
    }

    #[instrument(skip(self, messages, tools), fields(provider = "anthropic", model = %self.model))]
    async fn generate_with_tools(
        &self,
        messages: &[LLMMessage],
        tools: &[Tool],
    ) -> Result<LLMResponse, LLMError> {
        if tools.is_empty() {
            return self.generate(messages).await;
        }

        debug!("Generating with {} tools", tools.len());

        let system = Self::system_prompt(messages);

        // Convert messages to Anthropic format with content blocks
        let api_messages: Vec<AnthropicMessageWithContent> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let role = match m.role {
                    Role::Assistant => "assistant".to_string(),
                    Role::User | Role::System => "user".to_string(),
                    Role::Tool => "user".to_string(),
                };

                // Handle tool results and assistant tool calls
                let content = if m.role == Role::Tool {
                    // Tool result
                    AnthropicContentType::Blocks(vec![AnthropicContentBlock::ToolResult {
                        tool_use_id: m
                            .tool_call_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                        content: m.content.clone(),
                    }])
                } else if m.role == Role::Assistant && !m.tool_calls.is_empty() {
                    // Assistant with tool calls
                    let mut blocks = Vec::new();
                    if !m.content.is_empty() {
                        blocks.push(AnthropicContentBlock::Text {
                            text: m.content.clone(),
                        });
                    }
                    for tc in &m.tool_calls {
                        blocks.push(AnthropicContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            input: tc.arguments.clone(),
                        });
                    }
                    AnthropicContentType::Blocks(blocks)
                } else {
                    AnthropicContentType::Text(m.content.clone())
                };

                AnthropicMessageWithContent { role, content }
            })
            .collect();

        // Convert tools to Anthropic format
        let anthropic_tools: Vec<AnthropicTool> = tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect();

        let request = AnthropicRequestWithTools {
            model: self.model.clone(),
            messages: api_messages,
            max_tokens: self.max_tokens,
            system,
            temperature: Some(self.temperature),
            tools: Some(anthropic_tools),
        };

        let response = crate::http::send(|| self.post().json(&request), self.max_retries).await?;

        let api_response: AnthropicResponseWithTools = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.without_url().to_string()))?;

        // Extract text content and tool calls
        let mut content = String::new();
        let mut tool_calls = Vec::new();

        for block in &api_response.content {
            match block {
                AnthropicContentBlock::Text { text } => {
                    content.push_str(text);
                }
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall::new(id.clone(), name.clone(), input.clone()));
                }
                // Tool results are ours to send; other blocks are not used.
                AnthropicContentBlock::ToolResult { .. } | AnthropicContentBlock::Other => {}
            }
        }

        Ok(LLMResponse {
            content,
            tokens_used: Self::usage(&api_response.usage),
            finish_reason: Self::finish_reason(api_response.stop_reason.as_deref()),
            model: api_response.model,
            tool_calls,
        })
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
