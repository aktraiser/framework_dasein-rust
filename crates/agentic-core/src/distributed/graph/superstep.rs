//! Superstep - State management and checkpointing for workflow execution.
//!
//! The superstep model (BSP/Pregel) executes workflows in discrete steps:
//! 1. All active executors process their messages in parallel
//! 2. Messages are collected and routed through edges
//! 3. State is checkpointed at configurable intervals
//! 4. On failure, execution can resume from the last checkpoint
//!
//! # Checkpointing
//!
//! Checkpoints capture:
//! - Current superstep number
//! - Pending messages for each executor
//! - Completed executors in the current superstep
//! - Accumulated outputs
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::superstep::{
//!     Checkpoint, CheckpointBackend, InMemoryCheckpointBackend,
//! };
//!
//! let backend = InMemoryCheckpointBackend::new();
//!
//! // Save checkpoint
//! let checkpoint = Checkpoint::new(workflow_id, task_id, superstep, state);
//! backend.save(&checkpoint).await?;
//!
//! // Restore from checkpoint
//! let restored = backend.load(&workflow_id, &task_id).await?;
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

use super::types::{ExecutorError, ExecutorId, TaskId, WorkflowId};

// ============================================================================
// SUPERSTEP STATE
// ============================================================================

/// State for a single superstep execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SuperstepState {
    /// Messages pending for each executor.
    pub pending_messages: HashMap<ExecutorId, Vec<serde_json::Value>>,
    /// Executors that have completed in the current superstep.
    pub completed_executors: Vec<ExecutorId>,
    /// Executors that failed in the current superstep.
    pub failed_executors: HashMap<ExecutorId, String>,
}

impl SuperstepState {
    /// Create a new empty superstep state.
    pub fn new() -> Self {
        Self {
            pending_messages: HashMap::new(),
            completed_executors: Vec::new(),
            failed_executors: HashMap::new(),
        }
    }

    /// Enqueue a message for an executor.
    pub fn enqueue_message(&mut self, executor_id: ExecutorId, message: serde_json::Value) {
        self.pending_messages
            .entry(executor_id)
            .or_default()
            .push(message);
    }

    /// Mark an executor as completed.
    pub fn mark_completed(&mut self, executor_id: ExecutorId) {
        if !self.completed_executors.contains(&executor_id) {
            self.completed_executors.push(executor_id);
        }
    }

    /// Mark an executor as failed.
    pub fn mark_failed(&mut self, executor_id: ExecutorId, error: String) {
        self.failed_executors.insert(executor_id, error);
    }

    /// Check if there are pending messages.
    pub fn has_pending_messages(&self) -> bool {
        !self.pending_messages.is_empty()
    }

    /// Get the count of pending messages.
    pub fn pending_message_count(&self) -> usize {
        self.pending_messages.values().map(std::vec::Vec::len).sum()
    }

    /// Get all executor IDs with pending messages.
    pub fn active_executors(&self) -> Vec<&ExecutorId> {
        self.pending_messages.keys().collect()
    }

    /// Clear state for the next superstep.
    pub fn clear_for_next_superstep(&mut self) {
        self.completed_executors.clear();
        self.failed_executors.clear();
        // pending_messages are consumed during execution
    }

    /// Check if any executors failed.
    pub fn has_failures(&self) -> bool {
        !self.failed_executors.is_empty()
    }
}

// ============================================================================
// CHECKPOINT
// ============================================================================

/// A checkpoint capturing workflow state at a superstep boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique checkpoint ID.
    pub id: String,
    /// Workflow ID.
    pub workflow_id: WorkflowId,
    /// Task ID.
    pub task_id: TaskId,
    /// Superstep number at checkpoint.
    pub superstep: u32,
    /// State at checkpoint.
    pub state: SuperstepState,
    /// Accumulated outputs (serialized).
    pub outputs: Vec<serde_json::Value>,
    /// Timestamp when checkpoint was created.
    pub created_at: DateTime<Utc>,
    /// Retry counts per executor.
    pub retry_counts: HashMap<ExecutorId, u32>,
}

impl Checkpoint {
    /// Create a new checkpoint.
    pub fn new(
        workflow_id: WorkflowId,
        task_id: TaskId,
        superstep: u32,
        state: SuperstepState,
    ) -> Self {
        Self {
            id: format!("{}-{}-{}", workflow_id.as_str(), task_id.as_str(), superstep),
            workflow_id,
            task_id,
            superstep,
            state,
            outputs: Vec::new(),
            created_at: Utc::now(),
            retry_counts: HashMap::new(),
        }
    }

    /// Add outputs to the checkpoint.
    pub fn with_outputs(mut self, outputs: Vec<serde_json::Value>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Add retry counts to the checkpoint.
    pub fn with_retry_counts(mut self, counts: HashMap<ExecutorId, u32>) -> Self {
        self.retry_counts = counts;
        self
    }

    /// Get the checkpoint ID.
    pub fn checkpoint_id(&self) -> &str {
        &self.id
    }
}

// ============================================================================
// CHECKPOINT BACKEND
// ============================================================================

/// Backend for storing and retrieving checkpoints.
#[async_trait]
pub trait CheckpointBackend: Send + Sync {
    /// Save a checkpoint.
    async fn save(&self, checkpoint: &Checkpoint) -> Result<(), ExecutorError>;

    /// Load the latest checkpoint for a workflow/task.
    async fn load(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Option<Checkpoint>, ExecutorError>;

    /// Load a specific checkpoint by ID.
    async fn load_by_id(&self, checkpoint_id: &str) -> Result<Option<Checkpoint>, ExecutorError>;

    /// Delete checkpoints older than a given superstep.
    async fn cleanup(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        keep_after_superstep: u32,
    ) -> Result<usize, ExecutorError>;

    /// List all checkpoint IDs for a workflow/task.
    async fn list(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Vec<String>, ExecutorError>;
}

/// In-memory checkpoint backend for testing.
#[derive(Debug, Default)]
pub struct InMemoryCheckpointBackend {
    checkpoints: RwLock<HashMap<String, Checkpoint>>,
}

impl InMemoryCheckpointBackend {
    pub fn new() -> Self {
        Self {
            checkpoints: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl CheckpointBackend for InMemoryCheckpointBackend {
    async fn save(&self, checkpoint: &Checkpoint) -> Result<(), ExecutorError> {
        let mut checkpoints = self.checkpoints.write().await;
        checkpoints.insert(checkpoint.id.clone(), checkpoint.clone());
        Ok(())
    }

    async fn load(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Option<Checkpoint>, ExecutorError> {
        let checkpoints = self.checkpoints.read().await;
        let prefix = format!("{}-{}", workflow_id.as_str(), task_id.as_str());

        // Find the checkpoint with the highest superstep
        let latest = checkpoints
            .values()
            .filter(|c| c.id.starts_with(&prefix))
            .max_by_key(|c| c.superstep);

        Ok(latest.cloned())
    }

    async fn load_by_id(&self, checkpoint_id: &str) -> Result<Option<Checkpoint>, ExecutorError> {
        let checkpoints = self.checkpoints.read().await;
        Ok(checkpoints.get(checkpoint_id).cloned())
    }

    async fn cleanup(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        keep_after_superstep: u32,
    ) -> Result<usize, ExecutorError> {
        let mut checkpoints = self.checkpoints.write().await;
        let prefix = format!("{}-{}", workflow_id.as_str(), task_id.as_str());

        let to_remove: Vec<_> = checkpoints
            .iter()
            .filter(|(id, c)| id.starts_with(&prefix) && c.superstep < keep_after_superstep)
            .map(|(id, _)| id.clone())
            .collect();

        let count = to_remove.len();
        for id in to_remove {
            checkpoints.remove(&id);
        }

        Ok(count)
    }

    async fn list(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<Vec<String>, ExecutorError> {
        let checkpoints = self.checkpoints.read().await;
        let prefix = format!("{}-{}", workflow_id.as_str(), task_id.as_str());

        Ok(checkpoints
            .keys()
            .filter(|id| id.starts_with(&prefix))
            .cloned()
            .collect())
    }
}

// ============================================================================
// SUPERSTEP METRICS
// ============================================================================

/// Metrics for a single superstep execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SuperstepMetrics {
    /// Superstep number.
    pub superstep: u32,
    /// Number of executors that ran.
    pub executor_count: usize,
    /// Number of messages processed.
    pub messages_processed: usize,
    /// Number of messages produced.
    pub messages_produced: usize,
    /// Number of outputs yielded.
    pub outputs_yielded: usize,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Number of failures.
    pub failure_count: usize,
}

impl SuperstepMetrics {
    pub fn new(superstep: u32) -> Self {
        Self {
            superstep,
            ..Default::default()
        }
    }

    pub fn record_execution(
        &mut self,
        messages_processed: usize,
        messages_produced: usize,
        outputs_yielded: usize,
    ) {
        self.executor_count += 1;
        self.messages_processed += messages_processed;
        self.messages_produced += messages_produced;
        self.outputs_yielded += outputs_yielded;
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
    }

    pub fn set_duration(&mut self, duration_ms: u64) {
        self.duration_ms = duration_ms;
    }
}

// ============================================================================
// SUPERSTEP EXECUTOR (Helper for parallel execution)
// ============================================================================

/// Result of executing a single executor in a superstep.
#[derive(Debug)]
pub struct ExecutorSuperstepResult {
    /// Executor that was run.
    pub executor_id: ExecutorId,
    /// Whether execution succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// Messages produced.
    pub messages: Vec<serde_json::Value>,
    /// Outputs yielded.
    pub outputs: Vec<serde_json::Value>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

impl ExecutorSuperstepResult {
    pub fn success(
        executor_id: ExecutorId,
        messages: Vec<serde_json::Value>,
        outputs: Vec<serde_json::Value>,
        duration_ms: u64,
    ) -> Self {
        Self {
            executor_id,
            success: true,
            error: None,
            messages,
            outputs,
            duration_ms,
        }
    }

    pub fn failure(executor_id: ExecutorId, error: String, duration_ms: u64) -> Self {
        Self {
            executor_id,
            success: false,
            error: Some(error),
            messages: vec![],
            outputs: vec![],
            duration_ms,
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
    fn test_superstep_state_new() {
        let state = SuperstepState::new();
        assert!(!state.has_pending_messages());
        assert_eq!(state.pending_message_count(), 0);
        assert!(state.active_executors().is_empty());
    }

    #[test]
    fn test_superstep_state_enqueue_message() {
        let mut state = SuperstepState::new();

        state.enqueue_message(
            ExecutorId::new("executor-1"),
            serde_json::json!({"data": "test1"}),
        );
        state.enqueue_message(
            ExecutorId::new("executor-1"),
            serde_json::json!({"data": "test2"}),
        );
        state.enqueue_message(
            ExecutorId::new("executor-2"),
            serde_json::json!({"data": "test3"}),
        );

        assert!(state.has_pending_messages());
        assert_eq!(state.pending_message_count(), 3);
        assert_eq!(state.active_executors().len(), 2);
    }

    #[test]
    fn test_superstep_state_completion() {
        let mut state = SuperstepState::new();

        state.mark_completed(ExecutorId::new("exec-1"));
        state.mark_completed(ExecutorId::new("exec-2"));
        state.mark_completed(ExecutorId::new("exec-1")); // Duplicate

        assert_eq!(state.completed_executors.len(), 2);
    }

    #[test]
    fn test_superstep_state_failures() {
        let mut state = SuperstepState::new();

        state.mark_failed(ExecutorId::new("exec-1"), "Error 1".into());
        state.mark_failed(ExecutorId::new("exec-2"), "Error 2".into());

        assert!(state.has_failures());
        assert_eq!(state.failed_executors.len(), 2);
    }

    #[test]
    fn test_superstep_state_clear() {
        let mut state = SuperstepState::new();

        state.enqueue_message(ExecutorId::new("exec-1"), serde_json::json!({"x": 1}));
        state.mark_completed(ExecutorId::new("exec-1"));
        state.mark_failed(ExecutorId::new("exec-2"), "Error".into());

        state.clear_for_next_superstep();

        assert!(state.completed_executors.is_empty());
        assert!(state.failed_executors.is_empty());
        // Messages should remain (they're consumed during execution)
        assert!(state.has_pending_messages());
    }

    #[test]
    fn test_checkpoint_creation() {
        let checkpoint = Checkpoint::new(
            WorkflowId::new("wf-1"),
            TaskId::new("task-1"),
            5,
            SuperstepState::new(),
        );

        assert_eq!(checkpoint.workflow_id.as_str(), "wf-1");
        assert_eq!(checkpoint.task_id.as_str(), "task-1");
        assert_eq!(checkpoint.superstep, 5);
        assert!(checkpoint.outputs.is_empty());
    }

    #[test]
    fn test_checkpoint_with_outputs() {
        let checkpoint = Checkpoint::new(
            WorkflowId::new("wf-1"),
            TaskId::new("task-1"),
            5,
            SuperstepState::new(),
        )
        .with_outputs(vec![
            serde_json::json!({"output": 1}),
            serde_json::json!({"output": 2}),
        ]);

        assert_eq!(checkpoint.outputs.len(), 2);
    }

    #[tokio::test]
    async fn test_inmemory_checkpoint_backend_save_load() {
        let backend = InMemoryCheckpointBackend::new();

        let checkpoint = Checkpoint::new(
            WorkflowId::new("wf-1"),
            TaskId::new("task-1"),
            5,
            SuperstepState::new(),
        );

        backend.save(&checkpoint).await.unwrap();

        let loaded = backend
            .load(&WorkflowId::new("wf-1"), &TaskId::new("task-1"))
            .await
            .unwrap();

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.superstep, 5);
    }

    #[tokio::test]
    async fn test_inmemory_checkpoint_backend_load_latest() {
        let backend = InMemoryCheckpointBackend::new();

        // Save multiple checkpoints
        for i in 0..5 {
            let checkpoint = Checkpoint::new(
                WorkflowId::new("wf-1"),
                TaskId::new("task-1"),
                i,
                SuperstepState::new(),
            );
            backend.save(&checkpoint).await.unwrap();
        }

        // Load should return the latest (highest superstep)
        let loaded = backend
            .load(&WorkflowId::new("wf-1"), &TaskId::new("task-1"))
            .await
            .unwrap();

        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().superstep, 4);
    }

    #[tokio::test]
    async fn test_inmemory_checkpoint_backend_cleanup() {
        let backend = InMemoryCheckpointBackend::new();

        // Save checkpoints for supersteps 0-9
        for i in 0..10 {
            let checkpoint = Checkpoint::new(
                WorkflowId::new("wf-1"),
                TaskId::new("task-1"),
                i,
                SuperstepState::new(),
            );
            backend.save(&checkpoint).await.unwrap();
        }

        // Cleanup checkpoints before superstep 5
        let removed = backend
            .cleanup(&WorkflowId::new("wf-1"), &TaskId::new("task-1"), 5)
            .await
            .unwrap();

        assert_eq!(removed, 5); // Supersteps 0-4 removed

        // List remaining
        let remaining = backend
            .list(&WorkflowId::new("wf-1"), &TaskId::new("task-1"))
            .await
            .unwrap();

        assert_eq!(remaining.len(), 5); // Supersteps 5-9 remain
    }

    #[tokio::test]
    async fn test_inmemory_checkpoint_backend_load_by_id() {
        let backend = InMemoryCheckpointBackend::new();

        let checkpoint = Checkpoint::new(
            WorkflowId::new("wf-1"),
            TaskId::new("task-1"),
            5,
            SuperstepState::new(),
        );

        let id = checkpoint.id.clone();
        backend.save(&checkpoint).await.unwrap();

        let loaded = backend.load_by_id(&id).await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().superstep, 5);

        // Non-existent ID
        let not_found = backend.load_by_id("non-existent").await.unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_superstep_metrics() {
        let mut metrics = SuperstepMetrics::new(3);

        metrics.record_execution(10, 5, 2);
        metrics.record_execution(8, 3, 1);
        metrics.record_failure();
        metrics.set_duration(150);

        assert_eq!(metrics.superstep, 3);
        assert_eq!(metrics.executor_count, 2);
        assert_eq!(metrics.messages_processed, 18);
        assert_eq!(metrics.messages_produced, 8);
        assert_eq!(metrics.outputs_yielded, 3);
        assert_eq!(metrics.failure_count, 1);
        assert_eq!(metrics.duration_ms, 150);
    }

    #[test]
    fn test_executor_superstep_result_success() {
        let result = ExecutorSuperstepResult::success(
            ExecutorId::new("exec-1"),
            vec![serde_json::json!({"msg": 1})],
            vec![serde_json::json!({"out": 1})],
            50,
        );

        assert!(result.success);
        assert!(result.error.is_none());
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.outputs.len(), 1);
        assert_eq!(result.duration_ms, 50);
    }

    #[test]
    fn test_executor_superstep_result_failure() {
        let result = ExecutorSuperstepResult::failure(
            ExecutorId::new("exec-1"),
            "Test error".into(),
            25,
        );

        assert!(!result.success);
        assert_eq!(result.error, Some("Test error".into()));
        assert!(result.messages.is_empty());
        assert!(result.outputs.is_empty());
    }
}
