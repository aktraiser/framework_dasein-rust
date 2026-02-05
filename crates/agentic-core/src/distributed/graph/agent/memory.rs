//! Agent Memory - Long-term memory providers for agents.
//!
//! Memory providers inject context before agent invocations and extract
//! memories after responses. This enables long-term user preferences,
//! fact retention, and RAG-style retrieval.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    MEMORY ARCHITECTURE                      │
//! │                                                             │
//! │  ┌───────────────────────────────────────────────────────┐  │
//! │  │                 SHORT-TERM MEMORY                     │  │
//! │  │            (Conversation - AgentThread)               │  │
//! │  │   Thread = [msg1, msg2, ...]  →  NATS KV persistence  │  │
//! │  └───────────────────────────────────────────────────────┘  │
//! │                            │                                │
//! │                            ▼                                │
//! │  ┌───────────────────────────────────────────────────────┐  │
//! │  │                  LONG-TERM MEMORY                     │  │
//! │  │        (Persistent memories via MemoryProvider)       │  │
//! │  │   before_invoke() → inject relevant memories          │  │
//! │  │   after_invoke() → extract new memories               │  │
//! │  └───────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::agent::{
//!     MemoryProvider, MemoryContext, InMemoryProvider,
//! };
//!
//! // Create a simple in-memory provider
//! let provider = InMemoryProvider::new();
//!
//! // Before invoking agent
//! let mut ctx = MemoryContext::default();
//! provider.before_invoke(&agent_id, &thread, &mut ctx).await?;
//!
//! // After agent response
//! provider.after_invoke(&agent_id, &thread, &response).await?;
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

use super::response::AgentResponse;
use super::thread::AgentThread;
use super::types::{AgentId, ChatMessage};

// ============================================================================
// MEMORY ERROR
// ============================================================================

/// Errors that can occur during memory operations.
#[derive(Debug, Error)]
pub enum MemoryError {
    /// Storage backend error (NATS, file, etc.).
    #[error("Storage error: {0}")]
    StorageError(String),

    /// Memory not found.
    #[error("Memory not found: {key}")]
    NotFound { key: String },

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Extraction failed.
    #[error("Extraction error: {0}")]
    ExtractionError(String),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(String),
}

impl MemoryError {
    pub fn storage(msg: impl Into<String>) -> Self {
        Self::StorageError(msg.into())
    }

    pub fn not_found(key: impl Into<String>) -> Self {
        Self::NotFound { key: key.into() }
    }

    pub fn serialization(msg: impl Into<String>) -> Self {
        Self::SerializationError(msg.into())
    }

    pub fn extraction(msg: impl Into<String>) -> Self {
        Self::ExtractionError(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl From<serde_json::Error> for MemoryError {
    fn from(err: serde_json::Error) -> Self {
        Self::SerializationError(err.to_string())
    }
}

/// Result type for memory operations.
pub type MemoryResult<T> = Result<T, MemoryError>;

// ============================================================================
// MEMORY CONTEXT
// ============================================================================

/// Context that can be injected into agent invocations.
///
/// Memory providers populate this before each agent call to inject
/// relevant memories, user preferences, or additional context.
#[derive(Debug, Clone, Default)]
pub struct MemoryContext {
    /// Extra instructions to prepend to the system prompt.
    pub extra_instructions: Option<String>,

    /// Additional system messages to inject.
    pub system_messages: Vec<ChatMessage>,

    /// Retrieved memories (for debugging/inspection).
    pub retrieved_memories: Vec<Memory>,
}

impl MemoryContext {
    /// Create a new empty memory context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add extra instructions.
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.extra_instructions = Some(instructions.into());
        self
    }

    /// Add a system message.
    pub fn add_system_message(&mut self, content: impl Into<String>) {
        self.system_messages.push(ChatMessage::system(content));
    }

    /// Add a retrieved memory for tracking.
    pub fn add_memory(&mut self, memory: Memory) {
        self.retrieved_memories.push(memory);
    }

    /// Check if context is empty (no instructions or messages).
    pub fn is_empty(&self) -> bool {
        self.extra_instructions.is_none()
            && self.system_messages.is_empty()
            && self.retrieved_memories.is_empty()
    }

    /// Build the context string for injection into prompts.
    pub fn to_context_string(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }

        let mut parts = vec![];

        if let Some(ref instructions) = self.extra_instructions {
            parts.push(instructions.clone());
        }

        if !self.retrieved_memories.is_empty() {
            let memories_str = self
                .retrieved_memories
                .iter()
                .map(|m| format!("- {}", m.content))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("Relevant memories:\n{}", memories_str));
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }
}

// ============================================================================
// MEMORY TYPES
// ============================================================================

/// A single memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Unique memory identifier.
    pub id: String,

    /// Memory content.
    pub content: String,

    /// Memory category (user_preference, fact, learned, etc.).
    #[serde(default)]
    pub category: MemoryCategory,

    /// Importance score (0.0 - 1.0).
    #[serde(default)]
    pub importance: f32,

    /// When the memory was created.
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// When the memory was last accessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accessed: Option<chrono::DateTime<chrono::Utc>>,

    /// Optional metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Memory {
    /// Create a new memory.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.into(),
            category: MemoryCategory::default(),
            importance: 0.5,
            created_at: chrono::Utc::now(),
            last_accessed: None,
            metadata: HashMap::new(),
        }
    }

    /// Set the category.
    pub fn with_category(mut self, category: MemoryCategory) -> Self {
        self.category = category;
        self
    }

    /// Set the importance.
    pub fn with_importance(mut self, importance: f32) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// Categories of memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// User preference (e.g., "prefers dark mode").
    UserPreference,
    /// Factual information (e.g., "user's name is Alice").
    #[default]
    Fact,
    /// Learned behavior or pattern.
    Learned,
    /// Task-specific context.
    Context,
    /// Conversation summary.
    Summary,
}

/// Collection of memories for a user/agent pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMemories {
    /// Agent ID these memories belong to.
    pub agent_id: AgentId,

    /// User ID these memories belong to.
    pub user_id: String,

    /// The memories.
    pub memories: Vec<Memory>,
}

impl UserMemories {
    /// Create a new empty collection.
    pub fn new(agent_id: impl Into<AgentId>, user_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            user_id: user_id.into(),
            memories: vec![],
        }
    }

    /// Add a memory.
    pub fn add(&mut self, memory: Memory) {
        self.memories.push(memory);
    }

    /// Extend with multiple memories.
    pub fn extend(&mut self, memories: impl IntoIterator<Item = Memory>) {
        self.memories.extend(memories);
    }

    /// Get memories by category.
    pub fn by_category(&self, category: MemoryCategory) -> Vec<&Memory> {
        self.memories
            .iter()
            .filter(|m| m.category == category)
            .collect()
    }

    /// Get top N memories by importance.
    pub fn top_by_importance(&self, n: usize) -> Vec<&Memory> {
        let mut sorted: Vec<_> = self.memories.iter().collect();
        sorted.sort_by(|a, b| b.importance.partial_cmp(&a.importance).unwrap());
        sorted.into_iter().take(n).collect()
    }

    /// Build a context string from memories.
    pub fn to_context_string(&self) -> String {
        self.memories
            .iter()
            .map(|m| format!("- [{}] {}", m.category_str(), m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Memory {
    /// Get a short string representation of the category.
    pub fn category_str(&self) -> &'static str {
        match self.category {
            MemoryCategory::UserPreference => "pref",
            MemoryCategory::Fact => "fact",
            MemoryCategory::Learned => "learned",
            MemoryCategory::Context => "ctx",
            MemoryCategory::Summary => "summary",
        }
    }
}

// ============================================================================
// MEMORY PROVIDER TRAIT
// ============================================================================

/// Provider of long-term memory for agents.
///
/// Implementations can use various backends: in-memory, NATS KV, Redis, etc.
///
/// # Lifecycle
///
/// 1. `before_invoke()` - Called before each agent invocation to inject memories
/// 2. Agent processes the request
/// 3. `after_invoke()` - Called after to extract and store new memories
#[async_trait]
pub trait MemoryProvider: Send + Sync {
    /// Called BEFORE each agent invocation to inject relevant memories.
    ///
    /// Implementations should:
    /// - Retrieve relevant memories for the user/agent
    /// - Populate `ctx.extra_instructions` with memory context
    /// - Optionally add `ctx.system_messages` for specific injections
    async fn before_invoke(
        &self,
        agent_id: &AgentId,
        thread: &AgentThread,
        ctx: &mut MemoryContext,
    ) -> MemoryResult<()>;

    /// Called AFTER each agent invocation to extract and store memories.
    ///
    /// Implementations should:
    /// - Analyze the response for facts worth remembering
    /// - Extract user preferences or important information
    /// - Store new memories for future retrieval
    async fn after_invoke(
        &self,
        agent_id: &AgentId,
        thread: &AgentThread,
        response: &AgentResponse,
    ) -> MemoryResult<()>;

    /// Retrieve all memories for a user/agent pair.
    async fn get_memories(
        &self,
        agent_id: &AgentId,
        user_id: &str,
    ) -> MemoryResult<Vec<Memory>>;

    /// Store a memory explicitly.
    async fn store_memory(
        &self,
        agent_id: &AgentId,
        user_id: &str,
        memory: Memory,
    ) -> MemoryResult<()>;

    /// Clear all memories for a user/agent pair.
    async fn clear_memories(
        &self,
        agent_id: &AgentId,
        user_id: &str,
    ) -> MemoryResult<()>;
}

// ============================================================================
// IN-MEMORY PROVIDER
// ============================================================================

/// Simple in-memory implementation of MemoryProvider.
///
/// Useful for testing and single-instance applications.
/// Does NOT persist across restarts.
pub struct InMemoryProvider {
    /// Storage: (agent_id, user_id) -> memories
    storage: Arc<RwLock<HashMap<(String, String), UserMemories>>>,

    /// Maximum memories per user/agent pair.
    max_memories: usize,
}

impl Default for InMemoryProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryProvider {
    /// Create a new in-memory provider.
    pub fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(HashMap::new())),
            max_memories: 100,
        }
    }

    /// Set max memories per user/agent pair.
    pub fn with_max_memories(mut self, max: usize) -> Self {
        self.max_memories = max;
        self
    }

    fn storage_key(agent_id: &AgentId, user_id: &str) -> (String, String) {
        (agent_id.as_str().to_string(), user_id.to_string())
    }

    fn get_user_id(thread: &AgentThread) -> Option<String> {
        thread
            .get_metadata("user_id")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }
}

#[async_trait]
impl MemoryProvider for InMemoryProvider {
    async fn before_invoke(
        &self,
        agent_id: &AgentId,
        thread: &AgentThread,
        ctx: &mut MemoryContext,
    ) -> MemoryResult<()> {
        let Some(user_id) = Self::get_user_id(thread) else {
            return Ok(()); // No user ID, no memories to retrieve
        };

        let key = Self::storage_key(agent_id, &user_id);
        let storage = self.storage.read().map_err(|e| MemoryError::internal(e.to_string()))?;

        if let Some(user_memories) = storage.get(&key) {
            // Inject top memories into context
            let top_memories = user_memories.top_by_importance(10);
            for memory in top_memories {
                ctx.add_memory(memory.clone());
            }

            if !ctx.retrieved_memories.is_empty() {
                ctx.extra_instructions = Some(format!(
                    "User memories:\n{}",
                    user_memories.to_context_string()
                ));
            }
        }

        Ok(())
    }

    async fn after_invoke(
        &self,
        agent_id: &AgentId,
        thread: &AgentThread,
        _response: &AgentResponse,
    ) -> MemoryResult<()> {
        let Some(user_id) = Self::get_user_id(thread) else {
            return Ok(()); // No user ID, can't store memories
        };

        // In a real implementation, we would analyze the response
        // and extract memories. For now, this is a no-op.
        // The `store_memory` method can be used explicitly.

        let _key = Self::storage_key(agent_id, &user_id);

        Ok(())
    }

    async fn get_memories(
        &self,
        agent_id: &AgentId,
        user_id: &str,
    ) -> MemoryResult<Vec<Memory>> {
        let key = Self::storage_key(agent_id, user_id);
        let storage = self.storage.read().map_err(|e| MemoryError::internal(e.to_string()))?;

        Ok(storage
            .get(&key)
            .map(|um| um.memories.clone())
            .unwrap_or_default())
    }

    async fn store_memory(
        &self,
        agent_id: &AgentId,
        user_id: &str,
        memory: Memory,
    ) -> MemoryResult<()> {
        let key = Self::storage_key(agent_id, user_id);
        let mut storage = self.storage.write().map_err(|e| MemoryError::internal(e.to_string()))?;

        let user_memories = storage
            .entry(key)
            .or_insert_with(|| UserMemories::new(agent_id.clone(), user_id));

        user_memories.add(memory);

        // Trim if over max
        if user_memories.memories.len() > self.max_memories {
            // Remove oldest (lowest importance first)
            user_memories.memories.sort_by(|a, b| {
                b.importance.partial_cmp(&a.importance).unwrap()
            });
            user_memories.memories.truncate(self.max_memories);
        }

        Ok(())
    }

    async fn clear_memories(
        &self,
        agent_id: &AgentId,
        user_id: &str,
    ) -> MemoryResult<()> {
        let key = Self::storage_key(agent_id, user_id);
        let mut storage = self.storage.write().map_err(|e| MemoryError::internal(e.to_string()))?;
        storage.remove(&key);
        Ok(())
    }
}

// ============================================================================
// NO-OP PROVIDER
// ============================================================================

/// A no-op memory provider that does nothing.
///
/// Useful as a default or for agents that don't need memory.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpMemoryProvider;

#[async_trait]
impl MemoryProvider for NoOpMemoryProvider {
    async fn before_invoke(
        &self,
        _agent_id: &AgentId,
        _thread: &AgentThread,
        _ctx: &mut MemoryContext,
    ) -> MemoryResult<()> {
        Ok(())
    }

    async fn after_invoke(
        &self,
        _agent_id: &AgentId,
        _thread: &AgentThread,
        _response: &AgentResponse,
    ) -> MemoryResult<()> {
        Ok(())
    }

    async fn get_memories(
        &self,
        _agent_id: &AgentId,
        _user_id: &str,
    ) -> MemoryResult<Vec<Memory>> {
        Ok(vec![])
    }

    async fn store_memory(
        &self,
        _agent_id: &AgentId,
        _user_id: &str,
        _memory: Memory,
    ) -> MemoryResult<()> {
        Ok(())
    }

    async fn clear_memories(
        &self,
        _agent_id: &AgentId,
        _user_id: &str,
    ) -> MemoryResult<()> {
        Ok(())
    }
}

// ============================================================================
// SHARED TYPES
// ============================================================================

/// Boxed memory provider for dynamic dispatch.
pub type BoxedMemoryProvider = Box<dyn MemoryProvider>;

/// Arc-wrapped memory provider for sharing.
pub type SharedMemoryProvider = Arc<dyn MemoryProvider>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_new() {
        let memory = Memory::new("User prefers dark mode")
            .with_category(MemoryCategory::UserPreference)
            .with_importance(0.8);

        assert_eq!(memory.content, "User prefers dark mode");
        assert_eq!(memory.category, MemoryCategory::UserPreference);
        assert!((memory.importance - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_memory_importance_clamp() {
        let memory = Memory::new("test").with_importance(1.5);
        assert!((memory.importance - 1.0).abs() < f32::EPSILON);

        let memory = Memory::new("test").with_importance(-0.5);
        assert!(memory.importance.abs() < f32::EPSILON);
    }

    #[test]
    fn test_user_memories() {
        let mut memories = UserMemories::new("agent-1", "user-1");
        memories.add(Memory::new("fact 1").with_importance(0.5));
        memories.add(Memory::new("fact 2").with_importance(0.9));
        memories.add(Memory::new("fact 3").with_importance(0.2));

        let top = memories.top_by_importance(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].content, "fact 2");
        assert_eq!(top[1].content, "fact 1");
    }

    #[test]
    fn test_memory_context() {
        let mut ctx = MemoryContext::new();
        assert!(ctx.is_empty());

        ctx.extra_instructions = Some("Remember the user's name is Alice".into());
        ctx.add_memory(Memory::new("User name: Alice"));

        assert!(!ctx.is_empty());

        let context_str = ctx.to_context_string().unwrap();
        assert!(context_str.contains("Alice"));
    }

    #[tokio::test]
    async fn test_in_memory_provider() {
        let provider = InMemoryProvider::new();
        let agent_id = AgentId::new("test-agent");
        let user_id = "user-123";

        // Store a memory
        let memory = Memory::new("User likes rust")
            .with_category(MemoryCategory::UserPreference)
            .with_importance(0.9);

        provider.store_memory(&agent_id, user_id, memory).await.unwrap();

        // Retrieve memories
        let memories = provider.get_memories(&agent_id, user_id).await.unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].content, "User likes rust");

        // Clear memories
        provider.clear_memories(&agent_id, user_id).await.unwrap();
        let memories = provider.get_memories(&agent_id, user_id).await.unwrap();
        assert!(memories.is_empty());
    }

    #[tokio::test]
    async fn test_in_memory_provider_max_memories() {
        let provider = InMemoryProvider::new().with_max_memories(3);
        let agent_id = AgentId::new("test");
        let user_id = "user";

        // Add 5 memories with different importance
        for i in 0..5 {
            let importance = (i as f32) / 10.0;
            let memory = Memory::new(format!("Memory {}", i))
                .with_importance(importance);
            provider.store_memory(&agent_id, user_id, memory).await.unwrap();
        }

        // Should only have top 3 by importance
        let memories = provider.get_memories(&agent_id, user_id).await.unwrap();
        assert_eq!(memories.len(), 3);
    }

    #[tokio::test]
    async fn test_noop_provider() {
        let provider = NoOpMemoryProvider;
        let agent_id = AgentId::new("test");

        let memories = provider.get_memories(&agent_id, "user").await.unwrap();
        assert!(memories.is_empty());

        // Should not panic
        provider.store_memory(&agent_id, "user", Memory::new("test")).await.unwrap();
        provider.clear_memories(&agent_id, "user").await.unwrap();
    }

    #[tokio::test]
    async fn test_before_invoke_with_memories() {
        let provider = InMemoryProvider::new();
        let agent_id = AgentId::new("assistant");
        let user_id = "alice";

        // Store some memories
        provider.store_memory(&agent_id, user_id, Memory::new("User name: Alice")).await.unwrap();
        provider.store_memory(&agent_id, user_id, Memory::new("Likes Rust")).await.unwrap();

        // Create thread with user_id metadata
        let thread = AgentThread::new()
            .with_metadata("user_id", serde_json::json!("alice"));

        // Before invoke should populate context
        let mut ctx = MemoryContext::new();
        provider.before_invoke(&agent_id, &thread, &mut ctx).await.unwrap();

        assert!(!ctx.is_empty());
        assert_eq!(ctx.retrieved_memories.len(), 2);
        assert!(ctx.extra_instructions.is_some());
    }
}
