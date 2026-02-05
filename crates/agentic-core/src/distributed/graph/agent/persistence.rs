//! Thread Persistence - NATS KV storage for AgentThread.
//!
//! Enables conversation threads to persist across restarts and be shared
//! between multiple agent instances.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    THREAD PERSISTENCE                       │
//! │                                                             │
//! │   AgentThread ──serialize──▶ NATS KV (thread.{id})          │
//! │                                                             │
//! │   Operations:                                               │
//! │     save(thread) → store in KV                              │
//! │     load(id) → retrieve from KV                             │
//! │     delete(id) → remove from KV                             │
//! │     list(agent_id) → get all threads for agent              │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::agent::{
//!     ThreadStore, NatsThreadStore, AgentThread,
//! };
//!
//! let store = NatsThreadStore::new(nats_client).await?;
//!
//! // Save a thread
//! let thread = AgentThread::new();
//! store.save(&thread).await?;
//!
//! // Load it back
//! let loaded = store.load(&thread.id).await?;
//!
//! // List all threads for an agent
//! let summaries = store.list_by_agent(&agent_id).await?;
//! ```

use async_nats::jetstream::kv;
use async_trait::async_trait;
use futures::StreamExt;
use std::sync::Arc;
use tracing::{debug, info};

use crate::distributed::bus::{BusError, NatsClient};

use super::thread::{AgentThread, ThreadSummary};
use super::types::{AgentId, ThreadId};

// ============================================================================
// CONSTANTS
// ============================================================================

/// KV bucket name for threads.
pub const BUCKET_THREADS: &str = "AGENTIC_THREADS";

/// KV bucket name for thread index (for listing by agent).
pub const BUCKET_THREAD_INDEX: &str = "AGENTIC_THREAD_INDEX";

// ============================================================================
// THREAD STORE ERROR
// ============================================================================

/// Errors that can occur during thread persistence operations.
#[derive(Debug, thiserror::Error)]
pub enum ThreadStoreError {
    /// Storage backend error.
    #[error("Storage error: {0}")]
    StorageError(String),

    /// Thread not found.
    #[error("Thread not found: {0}")]
    NotFound(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Connection error.
    #[error("Connection error: {0}")]
    ConnectionError(String),
}

impl From<BusError> for ThreadStoreError {
    fn from(err: BusError) -> Self {
        Self::StorageError(err.to_string())
    }
}

impl From<serde_json::Error> for ThreadStoreError {
    fn from(err: serde_json::Error) -> Self {
        Self::SerializationError(err.to_string())
    }
}

/// Result type for thread store operations.
pub type ThreadStoreResult<T> = Result<T, ThreadStoreError>;

// ============================================================================
// THREAD STORE TRAIT
// ============================================================================

/// Trait for thread persistence backends.
///
/// Implementations can use various backends: in-memory, NATS KV, Redis, etc.
#[async_trait]
pub trait ThreadStore: Send + Sync {
    /// Save a thread.
    async fn save(&self, thread: &AgentThread) -> ThreadStoreResult<()>;

    /// Load a thread by ID.
    async fn load(&self, thread_id: &ThreadId) -> ThreadStoreResult<Option<AgentThread>>;

    /// Delete a thread.
    async fn delete(&self, thread_id: &ThreadId) -> ThreadStoreResult<()>;

    /// Check if a thread exists.
    async fn exists(&self, thread_id: &ThreadId) -> ThreadStoreResult<bool>;

    /// List all thread summaries for an agent.
    async fn list_by_agent(&self, agent_id: &AgentId) -> ThreadStoreResult<Vec<ThreadSummary>>;

    /// List all thread summaries (all agents).
    async fn list_all(&self) -> ThreadStoreResult<Vec<ThreadSummary>>;

    /// Delete all threads for an agent.
    async fn delete_by_agent(&self, agent_id: &AgentId) -> ThreadStoreResult<usize>;
}

// ============================================================================
// NATS THREAD STORE
// ============================================================================

/// NATS KV-based thread store implementation.
///
/// Uses two KV buckets:
/// - `AGENTIC_THREADS` - Stores full thread data (key: thread_id)
/// - `AGENTIC_THREAD_INDEX` - Index for listing by agent (key: agent_id, value: list of thread_ids)
pub struct NatsThreadStore {
    nats: Arc<NatsClient>,
}

impl NatsThreadStore {
    /// Create a new NATS thread store.
    pub async fn new(nats: Arc<NatsClient>) -> ThreadStoreResult<Self> {
        let store = Self { nats };
        store.ensure_buckets().await?;
        Ok(store)
    }

    /// Ensure KV buckets exist.
    async fn ensure_buckets(&self) -> ThreadStoreResult<()> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| ThreadStoreError::ConnectionError("JetStream not enabled".to_string()))?;

        for bucket_name in [BUCKET_THREADS, BUCKET_THREAD_INDEX] {
            match js.get_key_value(bucket_name).await {
                Ok(_) => {
                    debug!("Using existing KV bucket: {}", bucket_name);
                }
                Err(_) => {
                    info!("Creating KV bucket: {}", bucket_name);
                    js.create_key_value(kv::Config {
                        bucket: bucket_name.to_string(),
                        history: 5, // Keep 5 versions
                        ..Default::default()
                    })
                    .await
                    .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;
                }
            }
        }

        Ok(())
    }

    /// Get the threads KV store.
    async fn get_threads_store(
        &self,
    ) -> ThreadStoreResult<async_nats::jetstream::kv::Store> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| ThreadStoreError::ConnectionError("JetStream not enabled".to_string()))?;

        js.get_key_value(BUCKET_THREADS)
            .await
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))
    }

    /// Get the index KV store.
    async fn get_index_store(
        &self,
    ) -> ThreadStoreResult<async_nats::jetstream::kv::Store> {
        let js = self
            .nats
            .jetstream()
            .ok_or_else(|| ThreadStoreError::ConnectionError("JetStream not enabled".to_string()))?;

        js.get_key_value(BUCKET_THREAD_INDEX)
            .await
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))
    }

    /// Add a thread ID to the agent's index.
    async fn add_to_index(
        &self,
        agent_id: &AgentId,
        thread_id: &ThreadId,
    ) -> ThreadStoreResult<()> {
        let store = self.get_index_store().await?;
        let key = agent_id.as_str();

        // Get existing index
        let mut thread_ids: Vec<String> = match store.get(key).await {
            Ok(Some(entry)) => {
                serde_json::from_slice(&entry).unwrap_or_default()
            }
            _ => vec![],
        };

        // Add if not already present
        let thread_id_str = thread_id.as_str().to_string();
        if !thread_ids.contains(&thread_id_str) {
            thread_ids.push(thread_id_str);

            let value = serde_json::to_vec(&thread_ids)?;
            store
                .put(key, value.into())
                .await
                .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;
        }

        Ok(())
    }

    /// Remove a thread ID from the agent's index.
    async fn remove_from_index(
        &self,
        agent_id: &AgentId,
        thread_id: &ThreadId,
    ) -> ThreadStoreResult<()> {
        let store = self.get_index_store().await?;
        let key = agent_id.as_str();

        // Get existing index
        let mut thread_ids: Vec<String> = match store.get(key).await {
            Ok(Some(entry)) => {
                serde_json::from_slice(&entry).unwrap_or_default()
            }
            _ => return Ok(()),
        };

        // Remove if present
        let thread_id_str = thread_id.as_str();
        thread_ids.retain(|id| id != thread_id_str);

        if thread_ids.is_empty() {
            store
                .delete(key)
                .await
                .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;
        } else {
            let value = serde_json::to_vec(&thread_ids)?;
            store
                .put(key, value.into())
                .await
                .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;
        }

        Ok(())
    }
}

#[async_trait]
impl ThreadStore for NatsThreadStore {
    async fn save(&self, thread: &AgentThread) -> ThreadStoreResult<()> {
        let store = self.get_threads_store().await?;

        let key = thread.id.as_str();
        let value = serde_json::to_vec(thread)?;

        store
            .put(key, value.into())
            .await
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;

        // Update index if thread has a creator
        if let Some(ref agent_id) = thread.created_by {
            self.add_to_index(agent_id, &thread.id).await?;
        }

        debug!("Saved thread: {}", key);
        Ok(())
    }

    async fn load(&self, thread_id: &ThreadId) -> ThreadStoreResult<Option<AgentThread>> {
        let store = self.get_threads_store().await?;
        let key = thread_id.as_str();

        match store.get(key).await {
            Ok(Some(entry)) => {
                let thread: AgentThread = serde_json::from_slice(&entry)?;
                Ok(Some(thread))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(ThreadStoreError::StorageError(e.to_string())),
        }
    }

    async fn delete(&self, thread_id: &ThreadId) -> ThreadStoreResult<()> {
        // First load to get agent_id for index cleanup
        if let Some(thread) = self.load(thread_id).await? {
            if let Some(ref agent_id) = thread.created_by {
                self.remove_from_index(agent_id, thread_id).await?;
            }
        }

        let store = self.get_threads_store().await?;
        let key = thread_id.as_str();

        store
            .delete(key)
            .await
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;

        debug!("Deleted thread: {}", key);
        Ok(())
    }

    async fn exists(&self, thread_id: &ThreadId) -> ThreadStoreResult<bool> {
        let store = self.get_threads_store().await?;
        let key = thread_id.as_str();

        match store.get(key).await {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(ThreadStoreError::StorageError(e.to_string())),
        }
    }

    async fn list_by_agent(&self, agent_id: &AgentId) -> ThreadStoreResult<Vec<ThreadSummary>> {
        let index_store = self.get_index_store().await?;
        let threads_store = self.get_threads_store().await?;

        let key = agent_id.as_str();

        let thread_ids: Vec<String> = match index_store.get(key).await {
            Ok(Some(entry)) => {
                serde_json::from_slice(&entry).unwrap_or_default()
            }
            _ => return Ok(vec![]),
        };

        let mut summaries = Vec::with_capacity(thread_ids.len());
        for thread_id in thread_ids {
            if let Ok(Some(entry)) = threads_store.get(&thread_id).await {
                if let Ok(thread) = serde_json::from_slice::<AgentThread>(&entry) {
                    summaries.push(ThreadSummary::from(&thread));
                }
            }
        }

        // Sort by updated_at descending
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        Ok(summaries)
    }

    async fn list_all(&self) -> ThreadStoreResult<Vec<ThreadSummary>> {
        let store = self.get_threads_store().await?;

        let mut summaries = Vec::new();
        let mut keys = store
            .keys()
            .await
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;

        while let Some(Ok(key)) = keys.next().await {
            if let Ok(Some(entry)) = store.get(&key).await {
                if let Ok(thread) = serde_json::from_slice::<AgentThread>(&entry) {
                    summaries.push(ThreadSummary::from(&thread));
                }
            }
        }

        // Sort by updated_at descending
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        Ok(summaries)
    }

    async fn delete_by_agent(&self, agent_id: &AgentId) -> ThreadStoreResult<usize> {
        let index_store = self.get_index_store().await?;
        let threads_store = self.get_threads_store().await?;

        let key = agent_id.as_str();

        let thread_ids: Vec<String> = match index_store.get(key).await {
            Ok(Some(entry)) => {
                serde_json::from_slice(&entry).unwrap_or_default()
            }
            _ => return Ok(0),
        };

        let count = thread_ids.len();

        // Delete all threads
        for thread_id in &thread_ids {
            let _ = threads_store.delete(thread_id).await;
        }

        // Delete index entry
        let _ = index_store.delete(key).await;

        info!("Deleted {} threads for agent: {}", count, agent_id);
        Ok(count)
    }
}

// ============================================================================
// IN-MEMORY THREAD STORE
// ============================================================================

/// In-memory thread store for testing.
pub struct InMemoryThreadStore {
    threads: std::sync::RwLock<std::collections::HashMap<String, AgentThread>>,
}

impl Default for InMemoryThreadStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryThreadStore {
    /// Create a new in-memory thread store.
    pub fn new() -> Self {
        Self {
            threads: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl ThreadStore for InMemoryThreadStore {
    async fn save(&self, thread: &AgentThread) -> ThreadStoreResult<()> {
        let mut threads = self
            .threads
            .write()
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;
        threads.insert(thread.id.as_str().to_string(), thread.clone());
        Ok(())
    }

    async fn load(&self, thread_id: &ThreadId) -> ThreadStoreResult<Option<AgentThread>> {
        let threads = self
            .threads
            .read()
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;
        Ok(threads.get(thread_id.as_str()).cloned())
    }

    async fn delete(&self, thread_id: &ThreadId) -> ThreadStoreResult<()> {
        let mut threads = self
            .threads
            .write()
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;
        threads.remove(thread_id.as_str());
        Ok(())
    }

    async fn exists(&self, thread_id: &ThreadId) -> ThreadStoreResult<bool> {
        let threads = self
            .threads
            .read()
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;
        Ok(threads.contains_key(thread_id.as_str()))
    }

    async fn list_by_agent(&self, agent_id: &AgentId) -> ThreadStoreResult<Vec<ThreadSummary>> {
        let threads = self
            .threads
            .read()
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;

        let mut summaries: Vec<_> = threads
            .values()
            .filter(|t| t.created_by.as_ref() == Some(agent_id))
            .map(ThreadSummary::from)
            .collect();

        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    async fn list_all(&self) -> ThreadStoreResult<Vec<ThreadSummary>> {
        let threads = self
            .threads
            .read()
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;

        let mut summaries: Vec<_> = threads.values().map(ThreadSummary::from).collect();
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    async fn delete_by_agent(&self, agent_id: &AgentId) -> ThreadStoreResult<usize> {
        let mut threads = self
            .threads
            .write()
            .map_err(|e| ThreadStoreError::StorageError(e.to_string()))?;

        let to_delete: Vec<_> = threads
            .iter()
            .filter(|(_, t)| t.created_by.as_ref() == Some(agent_id))
            .map(|(k, _)| k.clone())
            .collect();

        let count = to_delete.len();
        for key in to_delete {
            threads.remove(&key);
        }

        Ok(count)
    }
}

// ============================================================================
// SHARED TYPES
// ============================================================================

/// Boxed thread store for dynamic dispatch.
pub type BoxedThreadStore = Box<dyn ThreadStore>;

/// Arc-wrapped thread store for sharing.
pub type SharedThreadStore = Arc<dyn ThreadStore>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::graph::agent::types::ChatMessage;

    #[tokio::test]
    async fn test_in_memory_store_save_load() {
        let store = InMemoryThreadStore::new();

        let mut thread = AgentThread::for_agent("test-agent");
        thread.add_message(ChatMessage::user("Hello"));
        thread.add_message(ChatMessage::assistant("Hi there!"));

        // Save
        store.save(&thread).await.unwrap();

        // Load
        let loaded = store.load(&thread.id).await.unwrap().unwrap();
        assert_eq!(loaded.id, thread.id);
        assert_eq!(loaded.message_count(), 2);
    }

    #[tokio::test]
    async fn test_in_memory_store_delete() {
        let store = InMemoryThreadStore::new();

        let thread = AgentThread::new();
        store.save(&thread).await.unwrap();

        assert!(store.exists(&thread.id).await.unwrap());

        store.delete(&thread.id).await.unwrap();

        assert!(!store.exists(&thread.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_in_memory_store_list_by_agent() {
        let store = InMemoryThreadStore::new();
        let agent_id = AgentId::new("my-agent");

        // Create 3 threads for the agent
        for i in 0..3 {
            let mut thread = AgentThread::for_agent(agent_id.clone());
            thread.add_message(ChatMessage::user(format!("Message {}", i)));
            store.save(&thread).await.unwrap();
        }

        // Create 1 thread for another agent
        let other_thread = AgentThread::for_agent("other-agent");
        store.save(&other_thread).await.unwrap();

        // List by agent
        let summaries = store.list_by_agent(&agent_id).await.unwrap();
        assert_eq!(summaries.len(), 3);
    }

    #[tokio::test]
    async fn test_in_memory_store_delete_by_agent() {
        let store = InMemoryThreadStore::new();
        let agent_id = AgentId::new("my-agent");

        // Create threads
        for _ in 0..3 {
            let thread = AgentThread::for_agent(agent_id.clone());
            store.save(&thread).await.unwrap();
        }

        // Delete all
        let count = store.delete_by_agent(&agent_id).await.unwrap();
        assert_eq!(count, 3);

        // Verify empty
        let summaries = store.list_by_agent(&agent_id).await.unwrap();
        assert!(summaries.is_empty());
    }
}
