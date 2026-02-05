//! Agent Thread - Conversation state container.
//!
//! The `AgentThread` holds the state of a conversation, including message history
//! and metadata. Agents are stateless; threads carry the conversation state.
//!
//! # Current Implementation
//!
//! This is an **in-memory** implementation. NATS KV persistence will be added
//! in Phase 8 (Agent Memory).
//!
//! # Example
//!
//! ```rust,ignore
//! let mut thread = AgentThread::new();
//! thread.add_message(ChatMessage::user("Hello"));
//!
//! let response = agent.run(vec![ChatMessage::user("Hi")], &mut thread).await?;
//! // Thread now contains the full conversation history
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::types::{AgentId, ChatMessage, ThreadId};

// ============================================================================
// AGENT THREAD
// ============================================================================

/// A conversation thread that holds message history and state.
///
/// Threads are the stateful counterpart to stateless agents.
/// Multiple agents can operate on the same thread (handoff pattern).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentThread {
    /// Unique thread identifier.
    pub id: ThreadId,

    /// Agent that created this thread (may differ from current agent).
    pub created_by: Option<AgentId>,

    /// Conversation messages.
    pub messages: Vec<ChatMessage>,

    /// Thread creation timestamp.
    pub created_at: DateTime<Utc>,

    /// Last activity timestamp.
    pub updated_at: DateTime<Utc>,

    /// Custom metadata (user_id, session_id, etc.).
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,

    /// System prompt (prepended to every LLM call).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Turn count (number of user-assistant exchanges).
    #[serde(default)]
    pub turn_count: u32,

    /// Total tokens used in this thread.
    #[serde(default)]
    pub total_tokens: u64,
}

impl AgentThread {
    /// Create a new empty thread.
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            id: ThreadId::generate(),
            created_by: None,
            messages: vec![],
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
            system_prompt: None,
            turn_count: 0,
            total_tokens: 0,
        }
    }

    /// Create a thread with a specific ID.
    pub fn with_id(id: impl Into<ThreadId>) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            created_by: None,
            messages: vec![],
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
            system_prompt: None,
            turn_count: 0,
            total_tokens: 0,
        }
    }

    /// Create a thread for a specific agent.
    pub fn for_agent(agent_id: impl Into<AgentId>) -> Self {
        let mut thread = Self::new();
        thread.created_by = Some(agent_id.into());
        thread
    }

    /// Set the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Add a message to the thread.
    pub fn add_message(&mut self, message: ChatMessage) {
        // Track turns (user message followed by assistant)
        if message.is_user() && self.messages.last().map(|m| m.is_assistant()).unwrap_or(true) {
            self.turn_count += 1;
        }

        self.messages.push(message);
        self.updated_at = Utc::now();
    }

    /// Add multiple messages.
    pub fn add_messages(&mut self, messages: impl IntoIterator<Item = ChatMessage>) {
        for msg in messages {
            self.add_message(msg);
        }
    }

    /// Add an assistant response and track tokens.
    pub fn add_response(&mut self, content: impl Into<String>, tokens: Option<u32>) {
        self.add_message(ChatMessage::assistant(content));
        if let Some(t) = tokens {
            self.total_tokens += u64::from(t);
        }
    }

    /// Get all messages for LLM call (with system prompt if set).
    pub fn messages_for_llm(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(self.messages.len() + 1);

        // Add system prompt first if set
        if let Some(ref prompt) = self.system_prompt {
            messages.push(ChatMessage::system(prompt));
        }

        // Add conversation messages (excluding system messages which are in system_prompt)
        messages.extend(
            self.messages
                .iter()
                .filter(|m| !m.is_system())
                .cloned(),
        );

        messages
    }

    /// Get the last N messages.
    pub fn last_messages(&self, n: usize) -> &[ChatMessage] {
        let start = self.messages.len().saturating_sub(n);
        &self.messages[start..]
    }

    /// Get the last user message.
    pub fn last_user_message(&self) -> Option<&ChatMessage> {
        self.messages.iter().rev().find(|m| m.is_user())
    }

    /// Get the last assistant message.
    pub fn last_assistant_message(&self) -> Option<&ChatMessage> {
        self.messages.iter().rev().find(|m| m.is_assistant())
    }

    /// Clear all messages (but keep metadata).
    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.turn_count = 0;
        self.updated_at = Utc::now();
    }

    /// Get message count.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Check if thread is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Get metadata value.
    pub fn get_metadata(&self, key: &str) -> Option<&serde_json::Value> {
        self.metadata.get(key)
    }

    /// Set metadata value.
    pub fn set_metadata(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.metadata.insert(key.into(), value);
        self.updated_at = Utc::now();
    }

    /// Serialize thread to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize thread from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize thread to bytes (for persistence).
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserialize thread from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

impl Default for AgentThread {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// THREAD SUMMARY
// ============================================================================

/// Summary of a thread (for listing without full message history).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub created_by: Option<AgentId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    pub turn_count: u32,
    pub preview: Option<String>,
}

impl From<&AgentThread> for ThreadSummary {
    fn from(thread: &AgentThread) -> Self {
        let preview = thread
            .last_user_message()
            .map(|m| m.content.chars().take(100).collect());

        Self {
            id: thread.id.clone(),
            created_by: thread.created_by.clone(),
            created_at: thread.created_at,
            updated_at: thread.updated_at,
            message_count: thread.messages.len(),
            turn_count: thread.turn_count,
            preview,
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
    fn test_thread_new() {
        let thread = AgentThread::new();
        assert!(thread.id.as_str().starts_with("thread-"));
        assert!(thread.is_empty());
        assert_eq!(thread.turn_count, 0);
    }

    #[test]
    fn test_thread_for_agent() {
        let thread = AgentThread::for_agent("writer");
        assert_eq!(thread.created_by, Some(AgentId::new("writer")));
    }

    #[test]
    fn test_thread_with_system_prompt() {
        let thread = AgentThread::new().with_system_prompt("You are helpful.");
        assert_eq!(thread.system_prompt, Some("You are helpful.".into()));
    }

    #[test]
    fn test_add_messages() {
        let mut thread = AgentThread::new();
        thread.add_message(ChatMessage::user("Hello"));
        thread.add_message(ChatMessage::assistant("Hi there!"));

        assert_eq!(thread.message_count(), 2);
        assert_eq!(thread.turn_count, 1);
    }

    #[test]
    fn test_turn_counting() {
        let mut thread = AgentThread::new();

        // Turn 1
        thread.add_message(ChatMessage::user("First question"));
        assert_eq!(thread.turn_count, 1);
        thread.add_message(ChatMessage::assistant("First answer"));
        assert_eq!(thread.turn_count, 1);

        // Turn 2
        thread.add_message(ChatMessage::user("Second question"));
        assert_eq!(thread.turn_count, 2);
    }

    #[test]
    fn test_messages_for_llm() {
        let mut thread = AgentThread::new().with_system_prompt("Be helpful.");
        thread.add_message(ChatMessage::user("Hello"));

        let messages = thread.messages_for_llm();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].is_system());
        assert!(messages[1].is_user());
    }

    #[test]
    fn test_last_messages() {
        let mut thread = AgentThread::new();
        for i in 0..5 {
            thread.add_message(ChatMessage::user(format!("Message {}", i)));
        }

        let last = thread.last_messages(3);
        assert_eq!(last.len(), 3);
        assert_eq!(last[0].content, "Message 2");
    }

    #[test]
    fn test_metadata() {
        let mut thread = AgentThread::new();
        thread.set_metadata("user_id", serde_json::json!("user-123"));

        assert_eq!(
            thread.get_metadata("user_id"),
            Some(&serde_json::json!("user-123"))
        );
    }

    #[test]
    fn test_serialization() {
        let mut thread = AgentThread::new();
        thread.add_message(ChatMessage::user("Test"));

        let json = thread.to_json().unwrap();
        let restored = AgentThread::from_json(&json).unwrap();

        assert_eq!(restored.id, thread.id);
        assert_eq!(restored.message_count(), 1);
    }

    #[test]
    fn test_thread_summary() {
        let mut thread = AgentThread::for_agent("test");
        thread.add_message(ChatMessage::user("Hello world"));
        thread.add_message(ChatMessage::assistant("Hi!"));

        let summary = ThreadSummary::from(&thread);
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.turn_count, 1);
        assert_eq!(summary.preview, Some("Hello world".into()));
    }
}
