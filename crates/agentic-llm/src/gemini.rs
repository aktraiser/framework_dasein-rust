//! Google Gemini adapter implementation.

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

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const API_KEY_HEADER: &str = "x-goog-api-key";

/// Google Gemini adapter.
pub struct GeminiAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    temperature: f32,
    max_tokens: Option<u32>,
    max_retries: u32,
}

impl GeminiAdapter {
    /// Create a new Gemini adapter.
    ///
    /// # Arguments
    ///
    /// * `api_key` - Google AI API key
    /// * `model` - Model to use (e.g., "gemini-2.0-flash", "gemini-1.5-pro")
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: crate::http::client(crate::http::DEFAULT_IDLE_TIMEOUT),
            api_key: api_key.into(),
            base_url: GEMINI_BASE_URL.to_string(),
            model: model.into(),
            temperature: 0.7,
            max_tokens: None,
            max_retries: crate::http::DEFAULT_MAX_RETRIES,
        }
    }

    /// Send requests to another server (a proxy, a gateway, a test server)
    /// instead of `https://generativelanguage.googleapis.com`. The adapter
    /// adds `/v1beta/models/{model}:{action}` to it.
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

    /// A POST to a model action with the authentication header.
    fn post(&self, stream: bool) -> reqwest::RequestBuilder {
        let request = self
            .client
            .post(self.api_url(stream))
            .header(API_KEY_HEADER, &self.api_key);
        // `alt=sse` frames the stream as server-sent events; the default is
        // one large JSON array, which cannot be parsed reliably piece by piece.
        if stream {
            request.query(&[("alt", "sse")])
        } else {
            request
        }
    }

    /// All system messages, in order, as the one instruction the API takes.
    fn system_instruction(messages: &[LLMMessage]) -> Option<GeminiSystemInstruction> {
        let parts: Vec<&str> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect();
        (!parts.is_empty()).then(|| GeminiSystemInstruction {
            parts: vec![GeminiPart {
                text: parts.join("\n\n"),
            }],
        })
    }

    fn usage(usage: Option<&GeminiUsageMetadata>) -> TokenUsage {
        usage.map_or(
            TokenUsage {
                details: None,
                prompt: 0,
                completion: 0,
                total: 0,
            },
            |usage| TokenUsage {
                details: serde_json::to_value(usage).ok(),
                prompt: usage.prompt_token_count,
                completion: usage.candidates_token_count,
                total: usage.total_token_count,
            },
        )
    }

    /// `has_tool_calls`: Gemini reports `STOP` when it asks for a tool.
    fn finish_reason(reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
        match reason {
            Some("MAX_TOKENS") => FinishReason::Length,
            // The answer was withheld or cut by a filter: not a normal end.
            Some(
                "SAFETY"
                | "RECITATION"
                | "BLOCKLIST"
                | "PROHIBITED_CONTENT"
                | "SPII"
                | "MALFORMED_FUNCTION_CALL"
                | "OTHER",
            ) => FinishReason::Error,
            _ if has_tool_calls => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        }
    }

    /// The error for a response without any candidate: usually the prompt
    /// itself was blocked, and the API says why.
    fn no_candidate(feedback: Option<&GeminiPromptFeedback>) -> LLMError {
        match feedback.and_then(|feedback| feedback.block_reason.as_deref()) {
            Some(reason) => LLMError::ApiError(format!("Prompt blocked by Gemini: {reason}")),
            None => LLMError::EmptyResponse,
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
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Build the API URL for the model.
    fn api_url(&self, stream: bool) -> String {
        let action = if stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        // The key travels in the `x-goog-api-key` header, never in the URL:
        // URLs end up in error messages, logs and proxies.
        format!("{}/v1beta/models/{}:{action}", self.base_url, self.model)
    }

    /// Preserve native tool calls and group the corresponding function responses.
    fn convert_tool_messages(messages: &[LLMMessage]) -> Result<Vec<serde_json::Value>, LLMError> {
        use serde_json::json;
        let mut contents: Vec<serde_json::Value> = Vec::new();
        let mut pending = std::collections::HashMap::new();
        let mut last_was_tool = false;
        for message in messages {
            match message.role {
                Role::System => {}
                Role::Assistant => {
                    let mut parts = Vec::new();
                    if !message.content.is_empty() {
                        parts.push(json!({"text":message.content}));
                    }
                    for call in &message.tool_calls {
                        if pending.insert(call.id.clone(), call.name.clone()).is_some() {
                            return Err(LLMError::InvalidResponse(
                                "Duplicate pending tool call ID".into(),
                            ));
                        }
                        parts
                            .push(json!({"functionCall":{"name":call.name,"args":call.arguments}}));
                    }
                    if parts.is_empty() {
                        parts.push(json!({"text":""}));
                    }
                    contents.push(json!({"role":"model","parts":parts}));
                    last_was_tool = false;
                }
                Role::Tool => {
                    let name = message
                        .tool_call_id
                        .as_ref()
                        .and_then(|id| pending.remove(id))
                        .ok_or_else(|| {
                            LLMError::InvalidResponse(
                                "Tool response has no matching function call".into(),
                            )
                        })?;
                    let value: serde_json::Value = serde_json::from_str(&message.content)
                        .unwrap_or_else(|_| json!({"output":message.content}));
                    let value = if value.is_object() {
                        value
                    } else {
                        json!({"result":value})
                    };
                    let part = json!({"functionResponse":{"name":name,"response":value}});
                    if last_was_tool {
                        contents.last_mut().unwrap()["parts"]
                            .as_array_mut()
                            .unwrap()
                            .push(part);
                    } else {
                        contents.push(json!({"role":"user","parts":[part]}));
                    }
                    last_was_tool = true;
                }
                Role::User => {
                    contents.push(json!({"role":"user","parts":[{"text":message.content}]}));
                    last_was_tool = false;
                }
            }
        }
        if !pending.is_empty() {
            return Err(LLMError::InvalidResponse(
                "Missing function responses".into(),
            ));
        }
        Ok(contents)
    }

    /// Convert messages to Gemini format.
    fn convert_messages(
        messages: &[LLMMessage],
    ) -> (Option<GeminiSystemInstruction>, Vec<GeminiContent>) {
        let system_instruction = Self::system_instruction(messages);

        let contents = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| GeminiContent {
                role: match m.role {
                    Role::Assistant => "model".to_string(),
                    // System filtered above, Tool acts as user
                    Role::User | Role::System | Role::Tool => "user".to_string(),
                },
                parts: vec![GeminiPart {
                    text: m.content.clone(),
                }],
            })
            .collect();

        (system_instruction, contents)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize, Default)]
struct GeminiContent {
    #[serde(default)]
    role: String,
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize)]
struct GeminiPart {
    /// Absent on parts that are not text (function calls, thoughts...).
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPromptFeedback {
    #[serde(default)]
    block_reason: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    /// Absent when the prompt was blocked.
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    prompt_feedback: Option<GeminiPromptFeedback>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default)]
    model_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    /// Absent when the answer was withheld by a filter.
    #[serde(default)]
    content: GeminiContent,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)] // Field names match Gemini API response
struct GeminiUsageMetadata {
    #[serde(default)]
    cached_content_token_count: Option<u32>,
    #[serde(default)]
    thoughts_token_count: Option<u32>,
    // All default: intermediate stream chunks report only some of them.
    #[serde(default)]
    prompt_token_count: u32,
    #[serde(default)]
    candidates_token_count: u32,
    #[serde(default)]
    total_token_count: u32,
}

// ============================================================================
// TOOL CALLING SUPPORT
// ============================================================================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequestWithTools {
    contents: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiToolDeclaration>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiToolDeclaration {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPartWithFunction {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    function_call: Option<GeminiFunctionCall>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GeminiContentWithFunction {
    #[serde(default)]
    role: String,
    #[serde(default)]
    parts: Vec<GeminiPartWithFunction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponseWithFunction {
    #[serde(default)]
    candidates: Vec<GeminiCandidateWithFunction>,
    #[serde(default)]
    prompt_feedback: Option<GeminiPromptFeedback>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default)]
    model_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidateWithFunction {
    #[serde(default)]
    content: GeminiContentWithFunction,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[async_trait]
impl LLMAdapter for GeminiAdapter {
    fn provider(&self) -> &'static str {
        "gemini"
    }

    fn model(&self) -> &str {
        &self.model
    }

    #[instrument(skip(self, messages), fields(provider = "gemini", model = %self.model))]
    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        debug!("Generating completion with {} messages", messages.len());

        let (system_instruction, contents) = Self::convert_messages(messages);

        let request = GeminiRequest {
            contents,
            system_instruction,
            generation_config: Some(GeminiGenerationConfig {
                temperature: Some(self.temperature),
                max_output_tokens: self.max_tokens,
            }),
        };

        let response =
            crate::http::send(|| self.post(false).json(&request), self.max_retries).await?;

        let api_response: GeminiResponse = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.without_url().to_string()))?;

        let candidate = api_response
            .candidates
            .first()
            .ok_or_else(|| Self::no_candidate(api_response.prompt_feedback.as_ref()))?;

        let content = candidate
            .content
            .parts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<String>();

        Ok(LLMResponse {
            content,
            tokens_used: Self::usage(api_response.usage_metadata.as_ref()),
            finish_reason: Self::finish_reason(candidate.finish_reason.as_deref(), false),
            model: api_response
                .model_version
                .unwrap_or_else(|| self.model.clone()),
            tool_calls: vec![],
        })
    }

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        let (system_instruction, contents) = Self::convert_messages(messages);

        let request = GeminiRequest {
            contents,
            system_instruction,
            generation_config: Some(GeminiGenerationConfig {
                temperature: Some(self.temperature),
                max_output_tokens: self.max_tokens,
            }),
        };

        Box::pin(async_stream::try_stream! {
            let mut response =
                crate::http::send(|| self.post(true).json(&request), self.max_retries).await?;

            // One JSON response per server-sent event.
            let mut events = crate::http::SseBuffer::default();
            let mut finished = false;
            // Usage may come with any chunk; the last one seen is the total.
            let mut usage: Option<GeminiUsageMetadata> = None;
            loop {
                let chunk = response.chunk().await.map_err(crate::http::transport_error)?;
                let ready = match &chunk {
                    Some(bytes) => events.push(bytes)?,
                    None => events.finish().into_iter().collect(),
                };
                for data in ready {
                    let event: serde_json::Value = serde_json::from_str(&data)
                        .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;
                    // The API can fail after the stream has started.
                    if let Some(error) = event.get("error") {
                        let status = error
                            .get("code")
                            .and_then(serde_json::Value::as_u64)
                            .and_then(|code| u16::try_from(code).ok())
                            .and_then(|code| reqwest::StatusCode::from_u16(code).ok())
                            .unwrap_or(reqwest::StatusCode::INTERNAL_SERVER_ERROR);
                        Err(crate::http::status_error(status, &data))?;
                    }
                    let part: GeminiResponse = serde_json::from_value(event)
                        .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;
                    usage = part.usage_metadata.or(usage);
                    let Some(candidate) = part.candidates.first() else {
                        if part.prompt_feedback.is_some() {
                            Err(Self::no_candidate(part.prompt_feedback.as_ref()))?;
                        }
                        // A chunk with usage only: nothing to pass on.
                        continue;
                    };
                    let done = candidate.finish_reason.is_some();
                    finished |= done;
                    yield StreamChunk {
                        content: candidate
                            .content
                            .parts
                            .iter()
                            .map(|p| p.text.as_str())
                            .collect(),
                        done,
                        tokens_used: done.then(|| Self::usage(usage.as_ref())),
                        finish_reason: done.then(|| {
                            Self::finish_reason(candidate.finish_reason.as_deref(), false)
                        }),
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
        // List models endpoint to check API connectivity
        let url = format!("{}/v1beta/models/{}", self.base_url, self.model);

        let response = self
            .client
            .get(&url)
            .header(API_KEY_HEADER, &self.api_key)
            .send()
            .await
            .map_err(crate::http::transport_error)?;

        Ok(response.status().is_success())
    }

    fn supports_tools(&self) -> bool {
        true
    }

    #[instrument(skip(self, messages, tools), fields(provider = "gemini", model = %self.model))]
    async fn generate_with_tools(
        &self,
        messages: &[LLMMessage],
        tools: &[Tool],
    ) -> Result<LLMResponse, LLMError> {
        if tools.is_empty() {
            return self.generate(messages).await;
        }

        debug!("Generating with {} tools", tools.len());

        let system_instruction = Self::system_instruction(messages);

        let contents = Self::convert_tool_messages(messages)?;

        // Convert tools to Gemini format
        let gemini_tools = vec![GeminiToolDeclaration {
            function_declarations: tools
                .iter()
                .map(|t| GeminiFunctionDeclaration {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                })
                .collect(),
        }];

        let request = GeminiRequestWithTools {
            contents,
            system_instruction,
            generation_config: Some(GeminiGenerationConfig {
                temperature: Some(self.temperature),
                max_output_tokens: self.max_tokens,
            }),
            tools: Some(gemini_tools),
        };

        let response =
            crate::http::send(|| self.post(false).json(&request), self.max_retries).await?;

        let api_response: GeminiResponseWithFunction = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.without_url().to_string()))?;

        let candidate = api_response
            .candidates
            .first()
            .ok_or_else(|| Self::no_candidate(api_response.prompt_feedback.as_ref()))?;

        // Extract text content and tool calls
        let mut content = String::new();
        let mut tool_calls = Vec::new();

        for (i, part) in candidate.content.parts.iter().enumerate() {
            if let Some(text) = &part.text {
                content.push_str(text);
            }
            if let Some(fc) = &part.function_call {
                tool_calls.push(ToolCall::new(
                    format!("call_{}_{}", messages.len(), i),
                    fc.name.clone(),
                    fc.args.clone(),
                ));
            }
        }

        Ok(LLMResponse {
            content,
            tokens_used: Self::usage(api_response.usage_metadata.as_ref()),
            finish_reason: Self::finish_reason(
                candidate.finish_reason.as_deref(),
                !tool_calls.is_empty(),
            ),
            model: api_response
                .model_version
                .unwrap_or_else(|| self.model.clone()),
            tool_calls,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_never_in_url() {
        let adapter = GeminiAdapter::new("secret-key-123", "gemini-2.0-flash");
        assert!(!adapter.api_url(false).contains("secret-key-123"));
        assert!(!adapter.api_url(true).contains("secret-key-123"));
    }

    #[test]
    fn test_message_conversion() {
        let messages = vec![
            LLMMessage::system("You are helpful."),
            LLMMessage::user("Hello"),
            LLMMessage::assistant("Hi there!"),
        ];

        let (system, contents) = GeminiAdapter::convert_messages(&messages);

        assert!(system.is_some());
        assert_eq!(system.unwrap().parts[0].text, "You are helpful.");
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0].role, "user");
        assert_eq!(contents[1].role, "model");
    }
    #[test]
    fn tool_history_preserves_parallel_calls_and_matches_successive_rounds() {
        use serde_json::json;
        let messages = vec![
            LLMMessage::user("task"),
            LLMMessage::assistant_with_tool_calls(
                "read",
                vec![
                    ToolCall::new("a", "read_document", json!({"id":1})),
                    ToolCall::new("b", "read_document", json!({"id":2})),
                ],
            ),
            LLMMessage::tool_result("a", r#"{"value":18}"#),
            LLMMessage::tool_result("b", r#"{"value":5}"#),
            LLMMessage::assistant_with_tool_calls(
                "publish",
                vec![ToolCall::new(
                    "c",
                    "publish_finding",
                    json!({"text":"claim"}),
                )],
            ),
            LLMMessage::tool_result("c", r#"{"id":"F1"}"#),
        ];
        let v = GeminiAdapter::convert_tool_messages(&messages).unwrap();
        assert_eq!(v.len(), 5);
        assert_eq!(v[1]["parts"][1]["functionCall"]["name"], "read_document");
        assert_eq!(v[2]["parts"].as_array().unwrap().len(), 2);
        assert_eq!(
            v[2]["parts"][0]["functionResponse"]["response"]["value"],
            18
        );
        assert_eq!(
            v[4]["parts"][0]["functionResponse"]["name"],
            "publish_finding"
        );
        assert!(GeminiAdapter::convert_tool_messages(&messages[..2]).is_err());
        assert!(
            GeminiAdapter::convert_tool_messages(&[LLMMessage::tool_result("unknown", "{}")])
                .is_err()
        );
    }
}
