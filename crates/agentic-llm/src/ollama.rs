//! Ollama adapter for local LLM models.

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

/// Ollama adapter for local models.
pub struct OllamaAdapter {
    client: Client,
    base_url: String,
    model: String,
    temperature: f32,
    max_retries: u32,
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
            client: crate::http::client(crate::http::DEFAULT_IDLE_TIMEOUT),
            base_url: "http://localhost:11434".to_string(),
            model: model.into(),
            temperature: 0.7,
            max_retries: crate::http::DEFAULT_MAX_RETRIES,
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

    /// Set how long the server may stay silent before a call fails (default
    /// 10 minutes). A non-streamed generation sends nothing until it is
    /// complete, so slow hardware or long answers may need more.
    #[must_use]
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.client = crate::http::client(timeout);
        self
    }

    /// Set how many times a call is repeated when the server cannot be
    /// reached, limits the rate or is overloaded.
    #[must_use]
    pub const fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
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
    #[serde(default)]
    message: OllamaResponseMessage,
    #[serde(default)]
    done: bool,
    /// Why generation stopped: "stop", "length"...
    #[serde(default)]
    done_reason: Option<String>,
    /// Set instead of a message when the server fails mid-stream.
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize, Default)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

#[derive(Deserialize)]
struct OllamaToolCall {
    function: OllamaFunctionCall,
}

#[derive(Deserialize)]
struct OllamaFunctionCall {
    name: String,
    /// An object; some models send it as a JSON string.
    #[serde(default)]
    arguments: serde_json::Value,
}

/// A request that declares tools. Messages are built by hand: tool rounds
/// have a shape of their own.
#[derive(Serialize)]
struct OllamaToolRequest {
    model: String,
    messages: Vec<serde_json::Value>,
    tools: Vec<serde_json::Value>,
    stream: bool,
    options: OllamaOptions,
}

/// The conversation as Ollama takes it when tools are declared: the model's
/// calls stay calls, and each result names the tool it came from.
fn tool_messages(messages: &[LLMMessage]) -> Vec<serde_json::Value> {
    use serde_json::json;
    // Ollama has no call IDs: a result is matched to its tool by name.
    let mut names = std::collections::HashMap::new();
    messages
        .iter()
        .map(|message| match message.role {
            Role::System => json!({"role": "system", "content": message.content}),
            Role::User => json!({"role": "user", "content": message.content}),
            Role::Assistant => {
                let calls: Vec<_> = message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        names.insert(call.id.clone(), call.name.clone());
                        json!({"type": "function", "function": {
                            "name": call.name, "arguments": call.arguments,
                        }})
                    })
                    .collect();
                if calls.is_empty() {
                    json!({"role": "assistant", "content": message.content})
                } else {
                    json!({"role": "assistant", "content": message.content, "tool_calls": calls})
                }
            }
            Role::Tool => {
                let name = message
                    .tool_call_id
                    .as_ref()
                    .and_then(|id| names.get(id))
                    .cloned()
                    .unwrap_or_default();
                json!({"role": "tool", "tool_name": name, "content": message.content})
            }
        })
        .collect()
}

impl OllamaChatResponse {
    fn usage(&self) -> TokenUsage {
        let prompt = self.prompt_eval_count.unwrap_or(0);
        let completion = self.eval_count.unwrap_or(0);
        TokenUsage {
            details: None,
            prompt,
            completion,
            total: prompt.saturating_add(completion),
        }
    }

    fn finish_reason(&self) -> FinishReason {
        match self.done_reason.as_deref() {
            Some("length") => FinishReason::Length,
            _ => FinishReason::Stop,
        }
    }
}

impl From<&LLMMessage> for OllamaMessage {
    fn from(msg: &LLMMessage) -> Self {
        Self {
            role: match msg.role {
                Role::System => "system".to_string(),
                Role::User => "user".to_string(),
                Role::Assistant => "assistant".to_string(),
                Role::Tool => "user".to_string(), // Ollama doesn't support tool role
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
            // This adapter does not declare tools: earlier tool rounds are
            // written out as text.
            messages: crate::history::tool_rounds_as_text(messages)
                .iter()
                .map(OllamaMessage::from)
                .collect(),
            stream: false,
            options: OllamaOptions {
                temperature: self.temperature,
            },
        };

        let url = format!("{}/api/chat", self.base_url);
        let response =
            crate::http::send(|| self.client.post(&url).json(&request), self.max_retries).await?;

        let chat_response: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.without_url().to_string()))?;
        if let Some(error) = &chat_response.error {
            return Err(LLMError::ApiError(error.clone()));
        }

        Ok(LLMResponse {
            tokens_used: chat_response.usage(),
            finish_reason: chat_response.finish_reason(),
            content: chat_response.message.content,
            model: self.model.clone(),
            tool_calls: vec![],
        })
    }

    /// Whether tool calls work depends on the model: Ollama refuses a
    /// request with tools for a model that does not have the capability.
    fn supports_tools(&self) -> bool {
        true
    }

    #[instrument(skip(self, messages, tools), fields(provider = "ollama", model = %self.model))]
    async fn generate_with_tools(
        &self,
        messages: &[LLMMessage],
        tools: &[Tool],
    ) -> Result<LLMResponse, LLMError> {
        if tools.is_empty() {
            return self.generate(messages).await;
        }
        debug!("Generating with {} tools", tools.len());
        let request = OllamaToolRequest {
            model: self.model.clone(),
            messages: tool_messages(messages),
            tools: tools
                .iter()
                .map(|tool| {
                    serde_json::json!({"type": "function", "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }})
                })
                .collect(),
            stream: false,
            options: OllamaOptions {
                temperature: self.temperature,
            },
        };

        let url = format!("{}/api/chat", self.base_url);
        let response =
            crate::http::send(|| self.client.post(&url).json(&request), self.max_retries).await?;
        let chat_response: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.without_url().to_string()))?;
        if let Some(error) = &chat_response.error {
            return Err(LLMError::ApiError(error.clone()));
        }

        let tokens_used = chat_response.usage();
        let mut finish_reason = chat_response.finish_reason();
        let mut tool_calls = Vec::new();
        for (index, call) in chat_response.message.tool_calls.into_iter().enumerate() {
            let arguments = match call.function.arguments {
                serde_json::Value::String(text) => serde_json::from_str(&text).map_err(|e| {
                    LLMError::InvalidResponse(format!(
                        "arguments of the call to `{}` are not JSON: {e}",
                        call.function.name
                    ))
                })?,
                serde_json::Value::Null => serde_json::json!({}),
                other => other,
            };
            // Ollama gives calls no ID; the rest of the framework needs one
            // to pair a call with its result. Unique in the conversation:
            // its length so far, and the call's place in this answer.
            tool_calls.push(ToolCall::new(
                format!("call_{}_{index}", messages.len()),
                call.function.name,
                arguments,
            ));
        }
        if !tool_calls.is_empty() {
            finish_reason = FinishReason::ToolCalls;
        }
        Ok(LLMResponse {
            tokens_used,
            finish_reason,
            content: chat_response.message.content,
            model: self.model.clone(),
            tool_calls,
        })
    }

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        let request = OllamaChatRequest {
            model: self.model.clone(),
            // This adapter does not declare tools: earlier tool rounds are
            // written out as text.
            messages: crate::history::tool_rounds_as_text(messages)
                .iter()
                .map(OllamaMessage::from)
                .collect(),
            stream: true,
            options: OllamaOptions {
                temperature: self.temperature,
            },
        };

        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url);
        let max_retries = self.max_retries;

        Box::pin(async_stream::try_stream! {
            let mut response =
                crate::http::send(|| client.post(&url).json(&request), max_retries).await?;

            // One JSON object per line; a line may arrive in several pieces.
            let mut lines = crate::http::LineBuffer::default();
            let mut finished = false;
            loop {
                let chunk = response.chunk().await.map_err(crate::http::transport_error)?;
                let ready = match &chunk {
                    Some(bytes) => lines.push(bytes)?,
                    None => lines.finish().into_iter().collect(),
                };
                for line in ready.iter().filter(|line| !line.trim().is_empty()) {
                    let part: OllamaChatResponse = serde_json::from_str(line)
                        .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;
                    if let Some(error) = &part.error {
                        Err(LLMError::ApiError(error.clone()))?;
                    }
                    finished |= part.done;
                    yield StreamChunk {
                        tokens_used: part.done.then(|| part.usage()),
                        finish_reason: part.done.then(|| part.finish_reason()),
                        done: part.done,
                        content: part.message.content,
                    };
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
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map_err(crate::http::transport_error)?;

        Ok(response.status().is_success())
    }
}
