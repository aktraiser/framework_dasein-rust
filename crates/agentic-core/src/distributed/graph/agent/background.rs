//! Background Responses - Continuation tokens for long-running tasks.
//!
//! When an agent task takes too long or needs to be paused/resumed,
//! background responses allow the work to continue asynchronously.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                  BACKGROUND RESPONSES                       │
//! │                                                             │
//! │   User Request ──▶ Agent ──▶ Returns ContinuationToken      │
//! │                                                             │
//! │   Later...                                                  │
//! │   Poll(token) ──▶ Check status ──▶ Complete/InProgress      │
//! │                                                             │
//! │   States:                                                   │
//! │     Pending → InProgress → Complete/Failed                  │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::agent::{
//!     BackgroundTask, TaskStatus, BackgroundResponse,
//! };
//!
//! // Start a long-running task
//! let task = BackgroundTask::new("Generate report");
//!
//! // Return token to client
//! return BackgroundResponse::InProgress {
//!     task_id: task.id.clone(),
//!     continuation_token: task.to_token(),
//!     estimated_completion: None,
//! };
//!
//! // Later, poll for completion
//! let status = task_store.get_status(&task.id).await?;
//! match status {
//!     TaskStatus::Complete(result) => { /* use result */ }
//!     TaskStatus::InProgress => { /* keep polling */ }
//!     TaskStatus::Failed(err) => { /* handle error */ }
//! }
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ============================================================================
// TASK ID
// ============================================================================

/// Unique identifier for a background task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BackgroundTaskId(String);

impl BackgroundTaskId {
    /// Create a new task ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a new random task ID.
    pub fn generate() -> Self {
        Self(format!("task-{}", uuid::Uuid::new_v4()))
    }

    /// Get the ID as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BackgroundTaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for BackgroundTaskId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

// ============================================================================
// CONTINUATION TOKEN
// ============================================================================

/// A token that can be used to resume or check on a background task.
///
/// Contains serialized state that allows the task to be resumed
/// from where it left off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationToken {
    /// Task identifier.
    pub task_id: BackgroundTaskId,

    /// Serialized checkpoint state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoint: Vec<u8>,

    /// When the token was created.
    pub created_at: DateTime<Utc>,

    /// When the token expires (if applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,

    /// Optional metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl ContinuationToken {
    /// Create a new continuation token for a task.
    pub fn new(task_id: BackgroundTaskId) -> Self {
        Self {
            task_id,
            checkpoint: vec![],
            created_at: Utc::now(),
            expires_at: None,
            metadata: HashMap::new(),
        }
    }

    /// Create with checkpoint data.
    pub fn with_checkpoint(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.checkpoint = data.into();
        self
    }

    /// Set expiration time.
    pub fn with_expiry(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Set expiration duration from now.
    pub fn expires_in(mut self, duration: std::time::Duration) -> Self {
        self.expires_at = Some(Utc::now() + chrono::Duration::from_std(duration).unwrap());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Check if the token has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at.map(|exp| Utc::now() > exp).unwrap_or(false)
    }

    /// Serialize to JSON string for transport.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize from JSON string.
    pub fn from_json(s: &str) -> Result<Self, ContinuationError> {
        serde_json::from_str(s).map_err(|e| ContinuationError::InvalidToken(e.to_string()))
    }
}

// ============================================================================
// TASK STATUS
// ============================================================================

/// Status of a background task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task is waiting to start.
    Pending,

    /// Task is currently running.
    InProgress {
        /// Progress percentage (0-100).
        progress: Option<u8>,
        /// Status message.
        message: Option<String>,
        /// When the task started.
        started_at: DateTime<Utc>,
    },

    /// Task completed successfully.
    Complete {
        /// The result data.
        result: serde_json::Value,
        /// When the task completed.
        completed_at: DateTime<Utc>,
    },

    /// Task failed.
    Failed {
        /// Error message.
        error: String,
        /// When the task failed.
        failed_at: DateTime<Utc>,
        /// Whether the task can be retried.
        retriable: bool,
    },

    /// Task was cancelled.
    Cancelled {
        /// Why it was cancelled.
        reason: Option<String>,
        /// When it was cancelled.
        cancelled_at: DateTime<Utc>,
    },
}

impl TaskStatus {
    /// Check if the task is complete (successfully or not).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Complete { .. } | TaskStatus::Failed { .. } | TaskStatus::Cancelled { .. }
        )
    }

    /// Check if the task is still running.
    pub fn is_running(&self) -> bool {
        matches!(self, TaskStatus::Pending | TaskStatus::InProgress { .. })
    }

    /// Check if the task succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self, TaskStatus::Complete { .. })
    }
}

// ============================================================================
// BACKGROUND TASK
// ============================================================================

/// A background task that can be tracked and resumed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundTask {
    /// Unique task identifier.
    pub id: BackgroundTaskId,

    /// Human-readable description.
    pub description: String,

    /// Current status.
    pub status: TaskStatus,

    /// When the task was created.
    pub created_at: DateTime<Utc>,

    /// When the task was last updated.
    pub updated_at: DateTime<Utc>,

    /// Optional agent ID that owns this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// Optional user ID that requested this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,

    /// Continuation token for resuming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<ContinuationToken>,

    /// Metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl BackgroundTask {
    /// Create a new background task.
    pub fn new(description: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: BackgroundTaskId::generate(),
            description: description.into(),
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
            agent_id: None,
            user_id: None,
            continuation_token: None,
            metadata: HashMap::new(),
        }
    }

    /// Create with a specific ID.
    pub fn with_id(mut self, id: BackgroundTaskId) -> Self {
        self.id = id;
        self
    }

    /// Set the agent ID.
    pub fn for_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    /// Set the user ID.
    pub fn for_user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Mark as in progress.
    pub fn start(&mut self) {
        self.status = TaskStatus::InProgress {
            progress: Some(0),
            message: None,
            started_at: Utc::now(),
        };
        self.updated_at = Utc::now();
    }

    /// Update progress.
    pub fn update_progress(&mut self, progress: u8, message: Option<String>) {
        if let TaskStatus::InProgress { started_at, .. } = &self.status {
            self.status = TaskStatus::InProgress {
                progress: Some(progress.min(100)),
                message,
                started_at: *started_at,
            };
            self.updated_at = Utc::now();
        }
    }

    /// Mark as complete.
    pub fn complete(&mut self, result: serde_json::Value) {
        self.status = TaskStatus::Complete {
            result,
            completed_at: Utc::now(),
        };
        self.updated_at = Utc::now();
    }

    /// Mark as failed.
    pub fn fail(&mut self, error: impl Into<String>, retriable: bool) {
        self.status = TaskStatus::Failed {
            error: error.into(),
            failed_at: Utc::now(),
            retriable,
        };
        self.updated_at = Utc::now();
    }

    /// Mark as cancelled.
    pub fn cancel(&mut self, reason: Option<String>) {
        self.status = TaskStatus::Cancelled {
            reason,
            cancelled_at: Utc::now(),
        };
        self.updated_at = Utc::now();
    }

    /// Generate a continuation token for this task.
    pub fn to_token(&self) -> ContinuationToken {
        ContinuationToken::new(self.id.clone())
    }

    /// Generate a continuation token with checkpoint data.
    pub fn to_token_with_checkpoint(&self, checkpoint: Vec<u8>) -> ContinuationToken {
        ContinuationToken::new(self.id.clone()).with_checkpoint(checkpoint)
    }
}

// ============================================================================
// BACKGROUND RESPONSE
// ============================================================================

/// Response type that can indicate completion or continuation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackgroundResponse<T> {
    /// Task completed immediately.
    Complete(T),

    /// Task is in progress, use token to check status.
    InProgress {
        /// Task ID for status checks.
        task_id: BackgroundTaskId,
        /// Token for resuming/polling.
        continuation_token: ContinuationToken,
        /// Estimated completion time.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        estimated_completion: Option<DateTime<Utc>>,
        /// Current progress.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<u8>,
    },
}

impl<T> BackgroundResponse<T> {
    /// Create a complete response.
    pub fn complete(result: T) -> Self {
        Self::Complete(result)
    }

    /// Create an in-progress response.
    pub fn in_progress(task: &BackgroundTask) -> Self {
        Self::InProgress {
            task_id: task.id.clone(),
            continuation_token: task.to_token(),
            estimated_completion: None,
            progress: match &task.status {
                TaskStatus::InProgress { progress, .. } => *progress,
                _ => None,
            },
        }
    }

    /// Check if complete.
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete(_))
    }

    /// Get the result if complete.
    pub fn result(self) -> Option<T> {
        match self {
            Self::Complete(r) => Some(r),
            _ => None,
        }
    }

    /// Get the continuation token if in progress.
    pub fn token(&self) -> Option<&ContinuationToken> {
        match self {
            Self::InProgress {
                continuation_token, ..
            } => Some(continuation_token),
            _ => None,
        }
    }
}

// ============================================================================
// ERROR TYPES
// ============================================================================

/// Errors that can occur with continuation tokens.
#[derive(Debug, thiserror::Error)]
pub enum ContinuationError {
    /// Token is invalid or corrupted.
    #[error("Invalid continuation token: {0}")]
    InvalidToken(String),

    /// Token has expired.
    #[error("Continuation token has expired")]
    Expired,

    /// Task not found.
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    /// Task already complete.
    #[error("Task already complete")]
    AlreadyComplete,
}

// ============================================================================
// TASK STORE (IN-MEMORY)
// ============================================================================

/// In-memory store for background tasks.
pub struct InMemoryTaskStore {
    tasks: RwLock<HashMap<String, BackgroundTask>>,
}

impl Default for InMemoryTaskStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryTaskStore {
    /// Create a new task store.
    pub fn new() -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
        }
    }

    /// Store a task.
    pub async fn store(&self, task: BackgroundTask) {
        let mut tasks = self.tasks.write().await;
        tasks.insert(task.id.as_str().to_string(), task);
    }

    /// Get a task by ID.
    pub async fn get(&self, task_id: &BackgroundTaskId) -> Option<BackgroundTask> {
        let tasks = self.tasks.read().await;
        tasks.get(task_id.as_str()).cloned()
    }

    /// Get task status.
    pub async fn get_status(&self, task_id: &BackgroundTaskId) -> Option<TaskStatus> {
        self.get(task_id).await.map(|t| t.status)
    }

    /// Update a task.
    pub async fn update(&self, task: BackgroundTask) {
        let mut tasks = self.tasks.write().await;
        tasks.insert(task.id.as_str().to_string(), task);
    }

    /// Delete a task.
    pub async fn delete(&self, task_id: &BackgroundTaskId) {
        let mut tasks = self.tasks.write().await;
        tasks.remove(task_id.as_str());
    }

    /// List all tasks.
    pub async fn list_all(&self) -> Vec<BackgroundTask> {
        let tasks = self.tasks.read().await;
        tasks.values().cloned().collect()
    }

    /// List tasks by status.
    pub async fn list_by_status(&self, running: bool) -> Vec<BackgroundTask> {
        let tasks = self.tasks.read().await;
        tasks
            .values()
            .filter(|t| t.status.is_running() == running)
            .cloned()
            .collect()
    }

    /// Clean up completed tasks older than a duration.
    pub async fn cleanup_old(&self, older_than: std::time::Duration) -> usize {
        let cutoff = Utc::now() - chrono::Duration::from_std(older_than).unwrap();
        let mut tasks = self.tasks.write().await;

        let to_remove: Vec<_> = tasks
            .iter()
            .filter(|(_, t)| t.status.is_terminal() && t.updated_at < cutoff)
            .map(|(k, _)| k.clone())
            .collect();

        let count = to_remove.len();
        for key in to_remove {
            tasks.remove(&key);
        }

        count
    }
}

/// Shared task store.
pub type SharedTaskStore = Arc<InMemoryTaskStore>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_background_task_lifecycle() {
        let mut task = BackgroundTask::new("Test task");
        assert!(matches!(task.status, TaskStatus::Pending));

        task.start();
        assert!(task.status.is_running());

        task.update_progress(50, Some("Halfway done".into()));
        if let TaskStatus::InProgress {
            progress, message, ..
        } = &task.status
        {
            assert_eq!(*progress, Some(50));
            assert_eq!(message.as_deref(), Some("Halfway done"));
        }

        task.complete(serde_json::json!({"result": "done"}));
        assert!(task.status.is_success());
        assert!(task.status.is_terminal());
    }

    #[test]
    fn test_background_task_failure() {
        let mut task = BackgroundTask::new("Failing task");
        task.start();
        task.fail("Something went wrong", true);

        assert!(task.status.is_terminal());
        assert!(!task.status.is_success());

        if let TaskStatus::Failed { retriable, .. } = &task.status {
            assert!(*retriable);
        }
    }

    #[test]
    fn test_continuation_token() {
        let task = BackgroundTask::new("Token test");
        let token = task.to_token_with_checkpoint(b"checkpoint data".to_vec());

        assert_eq!(token.task_id, task.id);
        assert_eq!(token.checkpoint, b"checkpoint data");
        assert!(!token.is_expired());
    }

    #[test]
    fn test_continuation_token_expiry() {
        let token = ContinuationToken::new(BackgroundTaskId::generate())
            .expires_in(std::time::Duration::from_secs(0));

        // Should be expired immediately (or very soon)
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(token.is_expired());
    }

    #[test]
    fn test_continuation_token_json() {
        let original = ContinuationToken::new(BackgroundTaskId::new("test-123"))
            .with_checkpoint(b"data".to_vec())
            .with_metadata("key", serde_json::json!("value"));

        let encoded = original.to_json().unwrap();
        let decoded = ContinuationToken::from_json(&encoded).unwrap();

        assert_eq!(decoded.task_id.as_str(), "test-123");
        assert_eq!(decoded.checkpoint, b"data");
    }

    #[test]
    fn test_background_response() {
        let response: BackgroundResponse<String> = BackgroundResponse::complete("Done!".into());
        assert!(response.is_complete());
        assert_eq!(response.result(), Some("Done!".into()));

        let task = BackgroundTask::new("Long task");
        let response: BackgroundResponse<String> = BackgroundResponse::in_progress(&task);
        assert!(!response.is_complete());
        assert!(response.token().is_some());
    }

    #[tokio::test]
    async fn test_in_memory_task_store() {
        let store = InMemoryTaskStore::new();

        let mut task = BackgroundTask::new("Store test");
        let task_id = task.id.clone();

        store.store(task.clone()).await;

        // Get
        let loaded = store.get(&task_id).await.unwrap();
        assert_eq!(loaded.description, "Store test");

        // Update
        task.start();
        store.update(task).await;

        let status = store.get_status(&task_id).await.unwrap();
        assert!(status.is_running());

        // Delete
        store.delete(&task_id).await;
        assert!(store.get(&task_id).await.is_none());
    }

    #[tokio::test]
    async fn test_task_store_list_by_status() {
        let store = InMemoryTaskStore::new();

        // Add running task
        let mut running = BackgroundTask::new("Running");
        running.start();
        store.store(running).await;

        // Add complete task
        let mut complete = BackgroundTask::new("Complete");
        complete.complete(serde_json::json!(null));
        store.store(complete).await;

        // List running
        let running_tasks = store.list_by_status(true).await;
        assert_eq!(running_tasks.len(), 1);
        assert_eq!(running_tasks[0].description, "Running");

        // List completed
        let completed_tasks = store.list_by_status(false).await;
        assert_eq!(completed_tasks.len(), 1);
        assert_eq!(completed_tasks[0].description, "Complete");
    }
}
