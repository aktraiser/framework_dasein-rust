//! Response types for the Agent Layer.
//!
//! Defines `AgentResponse` for complete responses and `AgentChunk` for streaming.

use serde::{Deserialize, Serialize};

use super::types::ChatMessage;

// ============================================================================
// AGENT RESPONSE
// ============================================================================

/// Complete response from an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// The generated content.
    pub content: String,

    /// Full conversation history including this response.
    pub messages: Vec<ChatMessage>,

    /// Tokens used for generation (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u32>,

    /// Model used for generation (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Whether the response was truncated due to max tokens.
    #[serde(default)]
    pub truncated: bool,

    /// Tool calls requested by the agent (for function calling).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

impl AgentResponse {
    /// Create a new response with content.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            messages: vec![],
            tokens_used: None,
            model: None,
            truncated: false,
            tool_calls: vec![],
        }
    }

    /// Create a response with content and messages.
    pub fn with_messages(content: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            content: content.into(),
            messages,
            tokens_used: None,
            model: None,
            truncated: false,
            tool_calls: vec![],
        }
    }

    /// Add token usage information.
    pub fn with_tokens(mut self, tokens: u32) -> Self {
        self.tokens_used = Some(tokens);
        self
    }

    /// Add model information.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Mark as truncated.
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    /// Add tool calls.
    pub fn with_tool_calls(mut self, calls: Vec<ToolCall>) -> Self {
        self.tool_calls = calls;
        self
    }

    /// Check if there are pending tool calls.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    /// Get the last assistant message from history.
    pub fn last_assistant_message(&self) -> Option<&ChatMessage> {
        self.messages.iter().rev().find(|m| m.is_assistant())
    }
}

impl Default for AgentResponse {
    fn default() -> Self {
        Self::new("")
    }
}

// ============================================================================
// AGENT CHUNK (Streaming)
// ============================================================================

/// A chunk from a streaming agent response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentChunk {
    /// Partial content (may be None for metadata-only chunks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// Whether this is the final chunk.
    #[serde(default)]
    pub done: bool,

    /// Token usage (only available on final chunk).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u32>,

    /// Model used (only available on final chunk).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Tool calls (only available when agent requests tool use).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,

    /// Error message if chunk represents an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AgentChunk {
    /// Create a content chunk.
    pub fn content(text: impl Into<String>) -> Self {
        Self {
            content: Some(text.into()),
            done: false,
            ..Default::default()
        }
    }

    /// Create a final chunk.
    pub fn done() -> Self {
        Self {
            done: true,
            ..Default::default()
        }
    }

    /// Create a final chunk with content.
    pub fn final_content(text: impl Into<String>) -> Self {
        Self {
            content: Some(text.into()),
            done: true,
            ..Default::default()
        }
    }

    /// Create an error chunk.
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            error: Some(msg.into()),
            done: true,
            ..Default::default()
        }
    }

    /// Add token usage to final chunk.
    pub fn with_tokens(mut self, tokens: u32) -> Self {
        self.tokens_used = Some(tokens);
        self
    }

    /// Add model info to chunk.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Check if this chunk has content.
    pub fn has_content(&self) -> bool {
        self.content
            .as_ref()
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    }

    /// Check if this chunk is an error.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

// ============================================================================
// TOOL CALL
// ============================================================================

/// A tool call requested by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this tool call.
    pub id: String,

    /// Name of the tool to invoke.
    pub name: String,

    /// Arguments as JSON.
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Create a new tool call.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_response_new() {
        let response = AgentResponse::new("Hello, world!");
        assert_eq!(response.content, "Hello, world!");
        assert!(response.messages.is_empty());
        assert!(!response.truncated);
    }

    #[test]
    fn test_agent_response_with_tokens() {
        let response = AgentResponse::new("Hi")
            .with_tokens(150)
            .with_model("gpt-4");

        assert_eq!(response.tokens_used, Some(150));
        assert_eq!(response.model, Some("gpt-4".into()));
    }

    #[test]
    fn test_agent_response_with_tool_calls() {
        let call = ToolCall::new("call-1", "calculator", serde_json::json!({"expr": "2+2"}));
        let response = AgentResponse::new("Let me calculate...").with_tool_calls(vec![call]);

        assert!(response.has_tool_calls());
        assert_eq!(response.tool_calls.len(), 1);
    }

    #[test]
    fn test_agent_chunk_content() {
        let chunk = AgentChunk::content("Hello");
        assert!(chunk.has_content());
        assert!(!chunk.done);
    }

    #[test]
    fn test_agent_chunk_done() {
        let chunk = AgentChunk::done().with_tokens(100);
        assert!(chunk.done);
        assert_eq!(chunk.tokens_used, Some(100));
    }

    #[test]
    fn test_agent_chunk_error() {
        let chunk = AgentChunk::error("Connection failed");
        assert!(chunk.is_error());
        assert!(chunk.done);
    }

    #[test]
    fn test_tool_call() {
        let call = ToolCall::new(
            "call-123",
            "search",
            serde_json::json!({"query": "rust async"}),
        );
        assert_eq!(call.name, "search");
        assert_eq!(call.arguments["query"], "rust async");
    }
}
