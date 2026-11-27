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

/// The tags models put around their reasoning, matched in any letter case.
const THINKING_TAGS: [(&str, &str); 2] = [("<think>", "</think>"), ("<thinking>", "</thinking>")];

/// If `text` starts with a reasoning tag: its length and its closing tag.
fn opening_tag(text: &str) -> Option<(usize, &'static str)> {
    THINKING_TAGS.iter().find_map(|(open, close)| {
        text.get(..open.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(open))
            .then_some((open.len(), *close))
    })
}

/// Where `close` starts in `text`, in any letter case, from byte `from` on.
fn closing_tag(text: &str, close: &str, from: usize) -> Option<usize> {
    // Lower-casing ASCII keeps every byte position.
    text.get(from..)?
        .to_ascii_lowercase()
        .find(close)
        .map(|at| from + at)
}

/// The answer itself, after the reasoning: the blank lines between the two
/// go, the indentation of its first line stays.
fn after_reasoning(text: &str) -> &str {
    text.trim_start_matches(['\n', '\r'])
}

/// The answer without the reasoning some models write before it.
///
/// Models with a thinking phase (qwen3, deepseek-r1...) put it at the start
/// of the content, between `<think>` or `<thinking>` tags, unless the server
/// is asked to separate it. It is not part of the answer.
///
/// Only reasoning that opens the content is removed, and only when it is
/// closed: a reply cut off inside its reasoning is returned as it came, so
/// that the caller sees what there was. Tags further into the answer are
/// left alone: they may be the answer's own text.
fn without_thinking(content: String) -> String {
    let start = content.trim_start();
    if let Some((opened, close)) = opening_tag(start) {
        if let Some(closed) = closing_tag(start, close, opened) {
            return after_reasoning(&start[closed + close.len()..]).to_owned();
        }
    }
    content
}

/// The same for an answer that arrives in pieces, with the same result as
/// [`without_thinking`] on the whole, wherever the pieces are cut.
///
/// Until enough has arrived to know whether the answer opens with a tag,
/// nothing is passed on. Reasoning is then held back up to its closing tag,
/// and given out at the end if that tag never comes.
#[derive(Default)]
enum ThinkingFilter {
    /// Nothing has arrived yet.
    #[default]
    Undecided,
    /// Not enough yet to know how the answer starts.
    Held(String),
    /// Inside the reasoning: everything so far, the tag that will close it,
    /// and how far it was searched for.
    Thinking {
        seen: String,
        close: &'static str,
        searched: usize,
    },
    /// Past the reasoning, before the first character of the answer.
    Leading,
    Answer,
}

impl ThinkingFilter {
    /// The part of `piece` that belongs to the answer, as far as is known.
    fn push(&mut self, piece: &str) -> String {
        match std::mem::take(self) {
            Self::Undecided => self.decide(piece.to_owned()),
            Self::Held(held) => self.decide(held + piece),
            Self::Thinking {
                seen,
                close,
                searched,
            } => self.think(seen + piece, close, searched),
            Self::Leading => self.lead(piece),
            Self::Answer => {
                *self = Self::Answer;
                piece.to_owned()
            }
        }
    }

    fn decide(&mut self, seen: String) -> String {
        let start = seen.trim_start();
        let before = seen.len() - start.len();
        if let Some((opened, close)) = opening_tag(start) {
            return self.think(seen, close, before + opened);
        }
        // Could it still become a tag?
        let may_open = THINKING_TAGS.iter().any(|(open, _)| {
            open.get(..start.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(start))
        });
        if may_open {
            *self = Self::Held(seen);
            String::new()
        } else {
            *self = Self::Answer;
            seen
        }
    }

    fn think(&mut self, seen: String, close: &'static str, searched: usize) -> String {
        if let Some(closed) = closing_tag(&seen, close, searched) {
            return self.lead(&seen[closed + close.len()..]);
        }
        // Look again from where a closing tag cut in two would start, on a
        // character boundary.
        let mut searched = seen.len().saturating_sub(close.len() - 1).max(searched);
        while !seen.is_char_boundary(searched) {
            searched += 1;
        }
        *self = Self::Thinking {
            seen,
            close,
            searched,
        };
        String::new()
    }

    fn lead(&mut self, piece: &str) -> String {
        let answer = after_reasoning(piece);
        *self = if answer.is_empty() {
            Self::Leading
        } else {
            Self::Answer
        };
        answer.to_owned()
    }

    /// What was held back and turns out to be all there is, at the end: a
    /// start too short to be a tag, or reasoning that was never closed.
    fn finish(&mut self) -> String {
        match std::mem::take(self) {
            Self::Held(held) => held,
            Self::Thinking { seen, .. } => seen,
            _ => String::new(),
        }
    }
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
            content: without_thinking(chat_response.message.content),
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
            content: without_thinking(chat_response.message.content),
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
            let mut answer = ThinkingFilter::default();
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
                    let mut content = answer.push(&part.message.content);
                    if part.done {
                        content.push_str(&answer.finish());
                    } else if content.is_empty() {
                        // Reasoning, or not yet known: nothing to pass on.
                        continue;
                    }
                    yield StreamChunk {
                        tokens_used: part.done.then(|| part.usage()),
                        finish_reason: part.done.then(|| part.finish_reason()),
                        done: part.done,
                        content,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer a filter lets through for an answer cut into `pieces`.
    fn streamed(pieces: &[&str]) -> String {
        let mut filter = ThinkingFilter::default();
        let mut answer: String = pieces.iter().map(|piece| filter.push(piece)).collect();
        answer.push_str(&filter.finish());
        answer
    }

    /// `whole` gives `expected` in one piece, and the same streamed, cut at
    /// every position and one character at a time.
    fn gives(whole: &str, expected: &str) {
        assert_eq!(
            without_thinking(whole.to_owned()),
            expected,
            "{whole:?} whole"
        );
        for cut in (0..=whole.len()).filter(|cut| whole.is_char_boundary(*cut)) {
            assert_eq!(
                streamed(&[&whole[..cut], &whole[cut..]]),
                expected,
                "{whole:?} cut at {cut}"
            );
        }
        let characters: Vec<String> = whole.chars().map(String::from).collect();
        let pieces: Vec<&str> = characters.iter().map(String::as_str).collect();
        assert_eq!(streamed(&pieces), expected, "{whole:?} by character");
    }

    #[test]
    fn reasoning_is_dropped_whole_or_streamed_wherever_the_pieces_are_cut() {
        gives(
            "<think>\nThe user wants a city. é\n</think>\n\nParis, of course.",
            "Paris, of course.",
        );
        // Other spellings models use.
        gives("<thinking>hm</thinking>\nParis.", "Paris.");
        gives("  <THINK>hm</Think>Paris.", "Paris.");
        // The answer's own indentation is kept.
        gives(
            "<think>code</think>\n\n    def f():\n        pass",
            "    def f():\n        pass",
        );
        // Nothing but reasoning.
        gives("<think>all of it</think>", "");
        gives("<think>all of it</think>\n", "");
    }

    #[test]
    fn a_reply_cut_off_inside_its_reasoning_is_returned_as_it_came() {
        // The model stopped before closing: the caller sees what there was,
        // the same way with and without streaming.
        gives("<think>\nstill thinking é", "<think>\nstill thinking é");
        gives("\n<think>", "\n<think>");
        gives("<think>almost</thin", "<think>almost</thin");
    }

    #[test]
    fn answers_without_reasoning_pass_whole() {
        for answer in [
            "Paris.",
            "  leading spaces stay",
            "<thin air> is not a tag",
            "<th",
            "<",
            "",
            "   ",
            "Use <think> tags sparingly.",
            // A block that does not open the answer is the answer's own text.
            "Let me check.\n<think>hm</think>\nParis.",
        ] {
            gives(answer, answer);
        }
    }
}
