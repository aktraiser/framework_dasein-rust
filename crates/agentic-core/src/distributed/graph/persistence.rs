//! Graph Persistence - Durable checkpoint storage for fault-tolerant workflows.
//!
//! Enables workflows to survive crashes and resume from the last checkpoint.
//!
//! # Use Case: Large-Scale Migration
//!
//! ```text
//! Friday 18:00: Start migration of 500 services
//!     → Checkpoint saved at superstep 50
//! Saturday 02:00: Server crash (power failure)
//! Monday 09:00: Restart workflow
//!     → Loads checkpoint from superstep 50
//!     → Resumes exactly where it left off
//!     → No wasted LLM tokens, no lost progress
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::persistence::RedisCheckpointBackend;
//!
//! // Connect to Redis
//! let backend = RedisCheckpointBackend::connect("redis://localhost:6379").await?;
//!
//! // Create workflow with durable checkpoints
//! let workflow = Workflow::with_config(definition, registry, config)
//!     .with_checkpoint_backend(Arc::new(backend));
//!
//! // Run - checkpoints are saved to Redis
//! let result = workflow.run(input).await?;
//!
//! // Or resume from a previous checkpoint
//! let result = workflow.run_with_resume(Some("checkpoint-id")).await?;
//! ```

use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::RwLock;

use super::superstep::Checkpoint;
#[cfg(feature = "redis-persistence")]
use super::superstep::CheckpointBackend;
use super::types::{ExecutorError, TaskId, WorkflowId};
#[cfg(feature = "redis-persistence")]
use super::types::ExecutorId;

// ============================================================================
// ENHANCED CHECKPOINT WITH SHARED STATE
// ============================================================================

/// Extended checkpoint that captures shared state for full recovery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistentCheckpoint {
    /// Base checkpoint data.
    pub checkpoint: Checkpoint,
    /// Shared state snapshot (key-value pairs).
    pub shared_state: HashMap<String, serde_json::Value>,
    /// Schema version for migration support.
    pub schema_version: u32,
    /// Workflow definition hash (to detect incompatible changes).
    pub workflow_hash: Option<String>,
}

impl PersistentCheckpoint {
    /// Create from a base checkpoint.
    pub fn from_checkpoint(checkpoint: Checkpoint) -> Self {
        Self {
            checkpoint,
            shared_state: HashMap::new(),
            schema_version: 1,
            workflow_hash: None,
        }
    }

    /// Add shared state snapshot.
    pub fn with_shared_state(mut self, state: HashMap<String, serde_json::Value>) -> Self {
        self.shared_state = state;
        self
    }

    /// Add workflow hash for compatibility checking.
    pub fn with_workflow_hash(mut self, hash: String) -> Self {
        self.workflow_hash = Some(hash);
        self
    }

    /// Get the checkpoint ID.
    pub fn id(&self) -> &str {
        &self.checkpoint.id
    }

    /// Get the superstep number.
    pub fn superstep(&self) -> u32 {
        self.checkpoint.superstep
    }
}

// ============================================================================
// CHECKPOINT METADATA
// ============================================================================

/// Lightweight metadata for listing checkpoints without loading full data.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointMetadata {
    /// Checkpoint ID.
    pub id: String,
    /// Workflow ID.
    pub workflow_id: String,
    /// Task ID.
    pub task_id: String,
    /// Superstep number.
    pub superstep: u32,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Size in bytes.
    pub size_bytes: usize,
    /// Number of pending messages.
    pub pending_message_count: usize,
    /// Number of outputs.
    pub output_count: usize,
}

impl From<&PersistentCheckpoint> for CheckpointMetadata {
    fn from(pc: &PersistentCheckpoint) -> Self {
        Self {
            id: pc.checkpoint.id.clone(),
            workflow_id: pc.checkpoint.workflow_id.as_str().to_string(),
            task_id: pc.checkpoint.task_id.as_str().to_string(),
            superstep: pc.checkpoint.superstep,
            created_at: pc.checkpoint.created_at.to_rfc3339(),
            size_bytes: 0, // Calculated on storage
            pending_message_count: pc.checkpoint.state.pending_message_count(),
            output_count: pc.checkpoint.outputs.len(),
        }
    }
}

// ============================================================================
// PERSISTENT CHECKPOINT BACKEND TRAIT
// ============================================================================

/// Extended checkpoint backend with shared state support.
#[async_trait]
pub trait PersistentCheckpointBackend: Send + Sync {
    /// Save a persistent checkpoint.
    async fn save_persistent(&self, checkpoint: &PersistentCheckpoint) -> Result<(), ExecutorError>;

    /// Load the latest persistent checkpoint.
    async fn load_persistent(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Option<PersistentCheckpoint>, ExecutorError>;

    /// Load a specific checkpoint by ID.
    async fn load_persistent_by_id(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<PersistentCheckpoint>, ExecutorError>;

    /// List checkpoint metadata without loading full data.
    async fn list_metadata(
        &self,
        workflow_id: &WorkflowId,
        task_id: Option<&TaskId>,
    ) -> Result<Vec<CheckpointMetadata>, ExecutorError>;

    /// Delete old checkpoints, keeping only the last N.
    async fn cleanup_keep_last(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        keep_count: usize,
    ) -> Result<usize, ExecutorError>;

    /// Delete all checkpoints for a workflow/task.
    async fn delete_all(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<usize, ExecutorError>;
}

// ============================================================================
// IN-MEMORY PERSISTENT BACKEND (for testing)
// ============================================================================

/// In-memory implementation for testing.
#[derive(Debug, Default)]
pub struct InMemoryPersistentBackend {
    checkpoints: RwLock<HashMap<String, PersistentCheckpoint>>,
}

impl InMemoryPersistentBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PersistentCheckpointBackend for InMemoryPersistentBackend {
    async fn save_persistent(&self, checkpoint: &PersistentCheckpoint) -> Result<(), ExecutorError> {
        let mut store = self.checkpoints.write().await;
        store.insert(checkpoint.id().to_string(), checkpoint.clone());
        Ok(())
    }

    async fn load_persistent(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Option<PersistentCheckpoint>, ExecutorError> {
        let store = self.checkpoints.read().await;
        let prefix = format!("{}-{}", workflow_id.as_str(), task_id.as_str());

        let latest = store
            .values()
            .filter(|c| c.id().starts_with(&prefix))
            .max_by_key(|c| c.superstep());

        Ok(latest.cloned())
    }

    async fn load_persistent_by_id(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<PersistentCheckpoint>, ExecutorError> {
        let store = self.checkpoints.read().await;
        Ok(store.get(checkpoint_id).cloned())
    }

    async fn list_metadata(
        &self,
        workflow_id: &WorkflowId,
        task_id: Option<&TaskId>,
    ) -> Result<Vec<CheckpointMetadata>, ExecutorError> {
        let store = self.checkpoints.read().await;

        let mut result: Vec<_> = store
            .values()
            .filter(|c| {
                c.checkpoint.workflow_id.as_str() == workflow_id.as_str()
                    && task_id
                        .map(|t| c.checkpoint.task_id.as_str() == t.as_str())
                        .unwrap_or(true)
            })
            .map(CheckpointMetadata::from)
            .collect();

        result.sort_by(|a, b| b.superstep.cmp(&a.superstep));
        Ok(result)
    }

    async fn cleanup_keep_last(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        keep_count: usize,
    ) -> Result<usize, ExecutorError> {
        let mut store = self.checkpoints.write().await;
        let prefix = format!("{}-{}", workflow_id.as_str(), task_id.as_str());

        // Get all matching checkpoints sorted by superstep descending
        let mut matching: Vec<_> = store
            .iter()
            .filter(|(id, _)| id.starts_with(&prefix))
            .map(|(id, c)| (id.clone(), c.superstep()))
            .collect();

        matching.sort_by(|a, b| b.1.cmp(&a.1));

        // Remove all but the last `keep_count`
        let to_remove: Vec<_> = matching.into_iter().skip(keep_count).map(|(id, _)| id).collect();

        let count = to_remove.len();
        for id in to_remove {
            store.remove(&id);
        }

        Ok(count)
    }

    async fn delete_all(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<usize, ExecutorError> {
        let mut store = self.checkpoints.write().await;
        let prefix = format!("{}-{}", workflow_id.as_str(), task_id.as_str());

        let to_remove: Vec<_> = store
            .keys()
            .filter(|id| id.starts_with(&prefix))
            .cloned()
            .collect();

        let count = to_remove.len();
        for id in to_remove {
            store.remove(&id);
        }

        Ok(count)
    }
}

// ============================================================================
// REDIS CHECKPOINT BACKEND
// ============================================================================

/// Redis-backed checkpoint storage for production use.
///
/// Stores checkpoints with the key pattern:
/// `checkpoint:{workflow_id}:{task_id}:{superstep}`
///
/// Also maintains a metadata index for fast listing:
/// `checkpoint_index:{workflow_id}:{task_id}` → sorted set of checkpoint IDs
///
/// Requires the `redis-persistence` feature to be enabled.
#[cfg(feature = "redis-persistence")]
pub struct RedisCheckpointBackend {
    client: redis::aio::ConnectionManager,
    /// TTL for checkpoints in seconds (None = no expiry)
    ttl_seconds: Option<u64>,
}

#[cfg(feature = "redis-persistence")]
impl RedisCheckpointBackend {
    /// Connect to Redis.
    pub async fn connect(url: &str) -> Result<Self, ExecutorError> {
        let client = redis::Client::open(url)
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis connection failed: {}", e)))?;

        let conn = redis::aio::ConnectionManager::new(client)
            .await
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis connection failed: {}", e)))?;

        Ok(Self {
            client: conn,
            ttl_seconds: None,
        })
    }

    /// Set TTL for checkpoints.
    pub fn with_ttl_seconds(mut self, ttl: u64) -> Self {
        self.ttl_seconds = Some(ttl);
        self
    }

    /// Set TTL for checkpoints in days.
    pub fn with_ttl_days(self, days: u64) -> Self {
        self.with_ttl_seconds(days * 24 * 60 * 60)
    }

    fn checkpoint_key(checkpoint_id: &str) -> String {
        format!("checkpoint:{}", checkpoint_id)
    }

    fn index_key(workflow_id: &str, task_id: &str) -> String {
        format!("checkpoint_index:{}:{}", workflow_id, task_id)
    }

    fn workflow_index_key(workflow_id: &str) -> String {
        format!("checkpoint_workflows:{}", workflow_id)
    }
}

#[cfg(feature = "redis-persistence")]
#[async_trait]
impl PersistentCheckpointBackend for RedisCheckpointBackend {
    async fn save_persistent(&self, checkpoint: &PersistentCheckpoint) -> Result<(), ExecutorError> {
        use redis::AsyncCommands;

        let mut conn = self.client.clone();
        let key = Self::checkpoint_key(checkpoint.id());
        let index_key = Self::index_key(
            checkpoint.checkpoint.workflow_id.as_str(),
            checkpoint.checkpoint.task_id.as_str(),
        );
        let workflow_index = Self::workflow_index_key(checkpoint.checkpoint.workflow_id.as_str());

        // Serialize checkpoint
        let data = serde_json::to_string(checkpoint)
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Serialization failed: {}", e)))?;

        // Store checkpoint
        if let Some(ttl) = self.ttl_seconds {
            conn.set_ex::<_, _, ()>(&key, &data, ttl)
                .await
                .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis set failed: {}", e)))?;
        } else {
            conn.set::<_, _, ()>(&key, &data)
                .await
                .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis set failed: {}", e)))?;
        }

        // Add to index (sorted set with superstep as score)
        conn.zadd::<_, _, _, ()>(&index_key, checkpoint.id(), checkpoint.superstep() as f64)
            .await
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis zadd failed: {}", e)))?;

        // Add task_id to workflow index
        conn.sadd::<_, _, ()>(&workflow_index, checkpoint.checkpoint.task_id.as_str())
            .await
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis sadd failed: {}", e)))?;

        tracing::debug!(
            checkpoint_id = %checkpoint.id(),
            superstep = checkpoint.superstep(),
            "Checkpoint saved to Redis"
        );

        Ok(())
    }

    async fn load_persistent(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Option<PersistentCheckpoint>, ExecutorError> {
        use redis::AsyncCommands;

        let mut conn = self.client.clone();
        let index_key = Self::index_key(workflow_id.as_str(), task_id.as_str());

        // Get the checkpoint ID with highest score (latest superstep)
        let result: Vec<String> = conn
            .zrevrange(&index_key, 0, 0)
            .await
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis zrevrange failed: {}", e)))?;

        let Some(checkpoint_id) = result.first() else {
            return Ok(None);
        };

        self.load_persistent_by_id(checkpoint_id).await
    }

    async fn load_persistent_by_id(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<PersistentCheckpoint>, ExecutorError> {
        use redis::AsyncCommands;

        let mut conn = self.client.clone();
        let key = Self::checkpoint_key(checkpoint_id);

        let data: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis get failed: {}", e)))?;

        let Some(data) = data else {
            return Ok(None);
        };

        let checkpoint: PersistentCheckpoint = serde_json::from_str(&data)
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Deserialization failed: {}", e)))?;

        Ok(Some(checkpoint))
    }

    async fn list_metadata(
        &self,
        workflow_id: &WorkflowId,
        task_id: Option<&TaskId>,
    ) -> Result<Vec<CheckpointMetadata>, ExecutorError> {
        use redis::AsyncCommands;

        let mut conn = self.client.clone();
        let mut all_metadata = Vec::new();

        // Get all task IDs for this workflow
        let task_ids: Vec<String> = if let Some(tid) = task_id {
            vec![tid.as_str().to_string()]
        } else {
            let workflow_index = Self::workflow_index_key(workflow_id.as_str());
            conn.smembers(&workflow_index)
                .await
                .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis smembers failed: {}", e)))?
        };

        for tid in task_ids {
            let index_key = Self::index_key(workflow_id.as_str(), &tid);

            // Get all checkpoint IDs (sorted by superstep descending)
            let checkpoint_ids: Vec<String> = conn
                .zrevrange(&index_key, 0, -1)
                .await
                .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis zrevrange failed: {}", e)))?;

            for id in checkpoint_ids {
                if let Some(checkpoint) = self.load_persistent_by_id(&id).await? {
                    all_metadata.push(CheckpointMetadata::from(&checkpoint));
                }
            }
        }

        Ok(all_metadata)
    }

    async fn cleanup_keep_last(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        keep_count: usize,
    ) -> Result<usize, ExecutorError> {
        use redis::AsyncCommands;

        let mut conn = self.client.clone();
        let index_key = Self::index_key(workflow_id.as_str(), task_id.as_str());

        // Get all checkpoint IDs except the last `keep_count`
        let to_remove: Vec<String> = conn
            .zrange(&index_key, 0, -(keep_count as isize + 1))
            .await
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis zrange failed: {}", e)))?;

        let count = to_remove.len();

        for id in &to_remove {
            let key = Self::checkpoint_key(id);
            conn.del::<_, ()>(&key)
                .await
                .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis del failed: {}", e)))?;

            conn.zrem::<_, _, ()>(&index_key, id)
                .await
                .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis zrem failed: {}", e)))?;
        }

        tracing::info!(
            workflow_id = %workflow_id.as_str(),
            task_id = %task_id.as_str(),
            removed = count,
            kept = keep_count,
            "Cleaned up old checkpoints"
        );

        Ok(count)
    }

    async fn delete_all(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<usize, ExecutorError> {
        use redis::AsyncCommands;

        let mut conn = self.client.clone();
        let index_key = Self::index_key(workflow_id.as_str(), task_id.as_str());

        // Get all checkpoint IDs
        let checkpoint_ids: Vec<String> = conn
            .zrange(&index_key, 0, -1)
            .await
            .map_err(|e| ExecutorError::new(ExecutorId::new("checkpoint"), format!("Redis zrange failed: {}", e)))?;

        let count = checkpoint_ids.len();

        // Delete all checkpoints
        for id in &checkpoint_ids {
            let key = Self::checkpoint_key(id);
            let _: Option<()> = conn.del(&key).await.ok();
        }

        // Delete the index
        let _: Option<()> = conn.del(&index_key).await.ok();

        // Remove from workflow index
        let workflow_index = Self::workflow_index_key(workflow_id.as_str());
        let _: Option<()> = conn.srem(&workflow_index, task_id.as_str()).await.ok();

        Ok(count)
    }
}

// Also implement the base CheckpointBackend trait for compatibility
#[cfg(feature = "redis-persistence")]
#[async_trait]
impl CheckpointBackend for RedisCheckpointBackend {
    async fn save(&self, checkpoint: &Checkpoint) -> Result<(), ExecutorError> {
        let persistent = PersistentCheckpoint::from_checkpoint(checkpoint.clone());
        self.save_persistent(&persistent).await
    }

    async fn load(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Option<Checkpoint>, ExecutorError> {
        let persistent = self.load_persistent(workflow_id, task_id).await?;
        Ok(persistent.map(|p| p.checkpoint))
    }

    async fn load_by_id(&self, checkpoint_id: &str) -> Result<Option<Checkpoint>, ExecutorError> {
        let persistent = self.load_persistent_by_id(checkpoint_id).await?;
        Ok(persistent.map(|p| p.checkpoint))
    }

    async fn cleanup(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        _keep_after_superstep: u32,
    ) -> Result<usize, ExecutorError> {
        // Convert to keep_count based approach
        // This is approximate - we'd need to load all to count
        self.cleanup_keep_last(workflow_id, task_id, 10).await
    }

    async fn list(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Vec<String>, ExecutorError> {
        let metadata = self.list_metadata(workflow_id, Some(task_id)).await?;
        Ok(metadata.into_iter().map(|m| m.id).collect())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::graph::superstep::SuperstepState;

    #[tokio::test]
    async fn test_inmemory_persistent_backend() {
        let backend = InMemoryPersistentBackend::new();

        // Create checkpoint
        let checkpoint = Checkpoint::new(
            WorkflowId::new("wf-1"),
            TaskId::new("task-1"),
            5,
            SuperstepState::new(),
        );
        let persistent = PersistentCheckpoint::from_checkpoint(checkpoint)
            .with_shared_state([("key".to_string(), serde_json::json!("value"))].into());

        // Save
        backend.save_persistent(&persistent).await.unwrap();

        // Load
        let loaded = backend
            .load_persistent(&WorkflowId::new("wf-1"), &TaskId::new("task-1"))
            .await
            .unwrap();

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.superstep(), 5);
        assert_eq!(loaded.shared_state.get("key"), Some(&serde_json::json!("value")));
    }

    #[tokio::test]
    async fn test_cleanup_keep_last() {
        let backend = InMemoryPersistentBackend::new();

        // Create 10 checkpoints
        for i in 0..10 {
            let checkpoint = Checkpoint::new(
                WorkflowId::new("wf-1"),
                TaskId::new("task-1"),
                i,
                SuperstepState::new(),
            );
            let persistent = PersistentCheckpoint::from_checkpoint(checkpoint);
            backend.save_persistent(&persistent).await.unwrap();
        }

        // Keep only last 3
        let removed = backend
            .cleanup_keep_last(&WorkflowId::new("wf-1"), &TaskId::new("task-1"), 3)
            .await
            .unwrap();

        assert_eq!(removed, 7);

        // List remaining
        let metadata = backend
            .list_metadata(&WorkflowId::new("wf-1"), Some(&TaskId::new("task-1")))
            .await
            .unwrap();

        assert_eq!(metadata.len(), 3);
        // Should be supersteps 9, 8, 7 (highest)
        assert_eq!(metadata[0].superstep, 9);
        assert_eq!(metadata[1].superstep, 8);
        assert_eq!(metadata[2].superstep, 7);
    }
}
