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

const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// Google Gemini adapter.
pub struct GeminiAdapter {
    client: Client,
    api_key: String,
    model: String,
    temperature: f32,
    max_tokens: Option<u32>,
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
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            temperature: 0.7,
            max_tokens: None,
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
        format!(
            "{}/{}:{}?key={}",
            GEMINI_API_BASE, self.model, action, self.api_key
        )
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
        let system_instruction =
            messages
                .iter()
                .find(|m| m.role == Role::System)
                .map(|m| GeminiSystemInstruction {
                    parts: vec![GeminiPart {
                        text: m.content.clone(),
                    }],
                });

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

#[derive(Serialize, Deserialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize)]
struct GeminiPart {
    text: String,
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
    candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default)]
    model_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
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
    prompt_token_count: u32,
    candidates_token_count: u32,
    total_token_count: u32,
}

#[derive(Deserialize)]
struct GeminiError {
    error: GeminiErrorDetail,
}

#[derive(Deserialize)]
struct GeminiErrorDetail {
    message: String,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GeminiContentWithFunction {
    role: String,
    parts: Vec<GeminiPartWithFunction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponseWithFunction {
    candidates: Vec<GeminiCandidateWithFunction>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default)]
    model_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidateWithFunction {
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

        let response = self
            .client
            .post(self.api_url(false))
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

        if !response.status().is_success() {
            let error: GeminiError = response
                .json()
                .await
                .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;
            return Err(LLMError::ApiError(error.error.message));
        }

        let api_response: GeminiResponse = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;

        let candidate = api_response
            .candidates
            .first()
            .ok_or(LLMError::EmptyResponse)?;

        let content = candidate
            .content
            .parts
            .iter()
            .map(|p| p.text.clone())
            .collect::<String>();

        let finish_reason = match candidate.finish_reason.as_deref() {
            Some("MAX_TOKENS") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        let usage = api_response.usage_metadata.unwrap_or(GeminiUsageMetadata {
            cached_content_token_count: None,
            thoughts_token_count: None,
            prompt_token_count: 0,
            candidates_token_count: 0,
            total_token_count: 0,
        });

        Ok(LLMResponse {
            content,
            tokens_used: TokenUsage {
                details: serde_json::to_value(&usage).ok(),
                prompt: usage.prompt_token_count,
                completion: usage.candidates_token_count,
                total: usage.total_token_count,
            },
            finish_reason,
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

        let client = self.client.clone();
        let url = self.api_url(true);

        Box::pin(async_stream::try_stream! {
            let response = client
                .post(&url)
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
                .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

            let status = response.status();
            if !status.is_success() {
                Err(LLMError::ApiError(format!("API returned status {status}")))?;
            }

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.map_err(|e| LLMError::ConnectionError(e.to_string()))?;
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // Gemini streams JSON array elements
                // Try to parse complete JSON objects from the buffer
                while let Some(obj_start) = buffer.find('{') {
                    // Find matching closing brace
                    let mut depth = 0;
                    let mut obj_end = None;

                    for (i, c) in buffer[obj_start..].char_indices() {
                        match c {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    obj_end = Some(obj_start + i + 1);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }

                    if let Some(end) = obj_end {
                        let json_str = &buffer[obj_start..end];

                        if let Ok(response) = serde_json::from_str::<GeminiResponse>(json_str) {
                            if let Some(candidate) = response.candidates.first() {
                                let content = candidate
                                    .content
                                    .parts
                                    .iter()
                                    .map(|p| p.text.clone())
                                    .collect::<String>();

                                let done = candidate.finish_reason.is_some();
                                let finish_reason = candidate.finish_reason.as_ref().map(|r| {
                                    match r.as_str() {
                                        "MAX_TOKENS" => FinishReason::Length,
                                        _ => FinishReason::Stop,
                                    }
                                });

                                let tokens_used = if done {
                                    response.usage_metadata.map(|u| TokenUsage { details: serde_json::to_value(&u).ok(),
                                        prompt: u.prompt_token_count,
                                        completion: u.candidates_token_count,
                                        total: u.total_token_count,
                                    })
                                } else {
                                    None
                                };

                                yield StreamChunk {
                                    content,
                                    done,
                                    tokens_used,
                                    finish_reason,
                                };
                            }
                        }

                        buffer = buffer[end..].to_string();
                    } else {
                        // Incomplete JSON object, wait for more data
                        break;
                    }
                }
            }
        })
    }

    async fn health_check(&self) -> Result<bool, LLMError> {
        // List models endpoint to check API connectivity
        let url = format!("{}/{}?key={}", GEMINI_API_BASE, self.model, self.api_key);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

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

        let (system_instruction, _) = Self::convert_messages(messages);

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

        let response = self
            .client
            .post(self.api_url(false))
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| LLMError::ConnectionError(e.to_string()))?;

        if !response.status().is_success() {
            let error: GeminiError = response
                .json()
                .await
                .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;
            return Err(LLMError::ApiError(error.error.message));
        }

        let api_response: GeminiResponseWithFunction = response
            .json()
            .await
            .map_err(|e| LLMError::InvalidResponse(e.to_string()))?;

        let candidate = api_response
            .candidates
            .first()
            .ok_or(LLMError::EmptyResponse)?;

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

        let finish_reason = match candidate.finish_reason.as_deref() {
            Some("MAX_TOKENS") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        let usage = api_response.usage_metadata.unwrap_or(GeminiUsageMetadata {
            cached_content_token_count: None,
            thoughts_token_count: None,
            prompt_token_count: 0,
            candidates_token_count: 0,
            total_token_count: 0,
        });

        Ok(LLMResponse {
            content,
            tokens_used: TokenUsage {
                details: serde_json::to_value(&usage).ok(),
                prompt: usage.prompt_token_count,
                completion: usage.candidates_token_count,
                total: usage.total_token_count,
            },
            finish_reason,
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
