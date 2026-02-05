//! Core types for the Agent Layer.
//!
//! Defines identifiers, messages, and roles for agent conversations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

// ============================================================================
// IDENTIFIERS
// ============================================================================

/// Unique identifier for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(String);

impl AgentId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// Unique identifier for a conversation thread.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(String);

impl ThreadId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(format!("thread-{}", uuid::Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ThreadId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

// ============================================================================
// CHAT ROLES
// ============================================================================

/// Role in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// System instructions (invisible to user)
    System,
    /// User message
    User,
    /// Assistant (agent) response
    Assistant,
    /// Tool/function result
    Tool,
}

impl Default for ChatRole {
    fn default() -> Self {
        Self::User
    }
}

impl fmt::Display for ChatRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChatRole::System => write!(f, "system"),
            ChatRole::User => write!(f, "user"),
            ChatRole::Assistant => write!(f, "assistant"),
            ChatRole::Tool => write!(f, "tool"),
        }
    }
}

// ============================================================================
// CHAT MESSAGE
// ============================================================================

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Role of the message sender.
    pub role: ChatRole,
    /// Content of the message.
    pub content: String,
    /// Optional name (for multi-agent scenarios).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// Timestamp when the message was created.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
}

impl ChatMessage {
    /// Create a new message.
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            name: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(ChatRole::System, content)
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(ChatRole::User, content)
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(ChatRole::Assistant, content)
    }

    /// Create a tool result message.
    pub fn tool(content: impl Into<String>) -> Self {
        Self::new(ChatRole::Tool, content)
    }

    /// Add a name to the message.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add metadata to the message.
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Check if this is a system message.
    pub fn is_system(&self) -> bool {
        self.role == ChatRole::System
    }

    /// Check if this is a user message.
    pub fn is_user(&self) -> bool {
        self.role == ChatRole::User
    }

    /// Check if this is an assistant message.
    pub fn is_assistant(&self) -> bool {
        self.role == ChatRole::Assistant
    }
}

impl fmt::Display for ChatMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = &self.name {
            write!(f, "[{}:{}] {}", self.role, name, self.content)
        } else {
            write!(f, "[{}] {}", self.role, self.content)
        }
    }
}

// ============================================================================
// CONVERSION FROM LLM TYPES
// ============================================================================

impl From<agentic_llm::LLMMessage> for ChatMessage {
    fn from(msg: agentic_llm::LLMMessage) -> Self {
        let role = match msg.role {
            agentic_llm::Role::System => ChatRole::System,
            agentic_llm::Role::User => ChatRole::User,
            agentic_llm::Role::Assistant => ChatRole::Assistant,
        };
        Self::new(role, msg.content)
    }
}

impl From<&ChatMessage> for agentic_llm::LLMMessage {
    fn from(msg: &ChatMessage) -> Self {
        match msg.role {
            ChatRole::System => agentic_llm::LLMMessage::system(&msg.content),
            ChatRole::User => agentic_llm::LLMMessage::user(&msg.content),
            ChatRole::Assistant => agentic_llm::LLMMessage::assistant(&msg.content),
            ChatRole::Tool => agentic_llm::LLMMessage::user(&msg.content), // Tool → User for LLM
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
    fn test_agent_id() {
        let id1 = AgentId::new("writer");
        let id2: AgentId = "writer".into();
        assert_eq!(id1, id2);
        assert_eq!(id1.as_str(), "writer");
    }

    #[test]
    fn test_thread_id_generate() {
        let id = ThreadId::generate();
        assert!(id.as_str().starts_with("thread-"));
    }

    #[test]
    fn test_chat_message_user() {
        let msg = ChatMessage::user("Hello");
        assert!(msg.is_user());
        assert!(!msg.is_assistant());
        assert_eq!(msg.content, "Hello");
    }

    #[test]
    fn test_chat_message_with_name() {
        let msg = ChatMessage::assistant("Hi there").with_name("Claude");
        assert_eq!(msg.name, Some("Claude".into()));
    }

    #[test]
    fn test_chat_message_display() {
        let msg = ChatMessage::user("Test").with_name("Alice");
        assert_eq!(format!("{}", msg), "[user:Alice] Test");
    }

    #[test]
    fn test_llm_message_conversion() {
        let chat_msg = ChatMessage::user("Hello");
        let llm_msg: agentic_llm::LLMMessage = (&chat_msg).into();
        assert_eq!(llm_msg.content, "Hello");
    }

    #[test]
    fn test_chat_role_serialize() {
        let role = ChatRole::Assistant;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, "\"assistant\"");
    }
}
