//! WorkflowContext - Concrete implementation of ExecutorContext.
//!
//! Provides executors with the ability to:
//! - Send messages to connected executors
//! - Yield outputs to the workflow caller
//! - Emit events for observability
//! - Access shared state
//! - Track execution history
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::{WorkflowContext, ExecutorContext};
//!
//! async fn example(ctx: &mut WorkflowContext<String, String>) {
//!     // Send to connected executors
//!     ctx.send_message("Hello".into()).await?;
//!
//!     // Yield output to caller
//!     ctx.yield_output("Result".into()).await?;
//!
//!     // Shared state
//!     ctx.set_shared_state("key", "value").await?;
//!     let val: Option<String> = ctx.get_shared_state("key").await?;
//! }
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::executor::ExecutorContext;
use super::types::{ExecutorError, ExecutorId, TaskId, WorkflowId};

// ============================================================================
// WORKFLOW EVENT
// ============================================================================

/// Events emitted during workflow execution.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct WorkflowEvent {
    /// Event type (e.g., "log", "progress", "custom")
    pub event_type: String,
    /// Executor that emitted the event
    pub executor_id: ExecutorId,
    /// Event data
    pub data: serde_json::Value,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

impl WorkflowEvent {
    /// Create a new workflow event.
    pub fn new(
        event_type: impl Into<String>,
        executor_id: ExecutorId,
        data: serde_json::Value,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            executor_id,
            data,
            timestamp: Utc::now(),
        }
    }
}

// ============================================================================
// SHARED STATE BACKEND
// ============================================================================

/// Backend for shared state storage.
///
/// This trait allows different implementations:
/// - `InMemoryStateBackend` for testing
/// - `NatsStateBackend` for production (future)
#[async_trait]
pub trait SharedStateBackend: Send + Sync {
    /// Set a value in shared state.
    async fn set(&self, key: &str, value: serde_json::Value) -> Result<(), ExecutorError>;

    /// Get a value from shared state.
    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>, ExecutorError>;

    /// Delete a value from shared state.
    async fn delete(&self, key: &str) -> Result<(), ExecutorError>;
}

/// In-memory shared state for testing.
#[derive(Debug, Default)]
pub struct InMemoryStateBackend {
    state: RwLock<HashMap<String, serde_json::Value>>,
}

impl InMemoryStateBackend {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl SharedStateBackend for InMemoryStateBackend {
    async fn set(&self, key: &str, value: serde_json::Value) -> Result<(), ExecutorError> {
        self.state.write().await.insert(key.to_string(), value);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>, ExecutorError> {
        Ok(self.state.read().await.get(key).cloned())
    }

    async fn delete(&self, key: &str) -> Result<(), ExecutorError> {
        self.state.write().await.remove(key);
        Ok(())
    }
}

// ============================================================================
// MESSAGE SINK
// ============================================================================

/// Sink for messages sent by executors.
///
/// Messages are collected and can be retrieved for routing to edges.
#[derive(Debug, Default)]
pub struct MessageSink<T> {
    messages: Vec<T>,
}

impl<T> MessageSink<T> {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn push(&mut self, message: T) {
        self.messages.push(message);
    }

    pub fn drain(&mut self) -> Vec<T> {
        std::mem::take(&mut self.messages)
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }
}

// ============================================================================
// OUTPUT SINK
// ============================================================================

/// Sink for outputs yielded by executors.
///
/// Outputs are visible to the workflow caller.
#[derive(Debug, Default)]
pub struct OutputSink<T> {
    outputs: Vec<T>,
}

impl<T> OutputSink<T> {
    pub fn new() -> Self {
        Self {
            outputs: Vec::new(),
        }
    }

    pub fn push(&mut self, output: T) {
        self.outputs.push(output);
    }

    pub fn drain(&mut self) -> Vec<T> {
        std::mem::take(&mut self.outputs)
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.outputs.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.outputs.len()
    }
}

// ============================================================================
// WORKFLOW CONTEXT
// ============================================================================

/// Concrete implementation of ExecutorContext.
///
/// Provides executors with:
/// - Message sending (to connected executors)
/// - Output yielding (to workflow caller)
/// - Event emission (observability)
/// - Shared state access
/// - Error history
pub struct WorkflowContext<TMessage, TOutput>
where
    TMessage: Send,
    TOutput: Send,
{
    /// Workflow this context belongs to
    workflow_id: WorkflowId,
    /// Current task being processed
    task_id: TaskId,
    /// Executor using this context
    executor_id: ExecutorId,
    /// Current superstep number
    superstep: u32,
    /// Messages sent by the executor
    messages: MessageSink<TMessage>,
    /// Outputs yielded by the executor
    outputs: OutputSink<TOutput>,
    /// Events emitted by the executor
    events: Vec<WorkflowEvent>,
    /// Errors from previous attempts
    previous_errors: Vec<ExecutorError>,
    /// Shared state backend
    state_backend: Arc<dyn SharedStateBackend>,
}

impl<TMessage, TOutput> WorkflowContext<TMessage, TOutput>
where
    TMessage: Send,
    TOutput: Send,
{
    /// Create a new workflow context.
    pub fn new(
        workflow_id: WorkflowId,
        task_id: TaskId,
        executor_id: ExecutorId,
        state_backend: Arc<dyn SharedStateBackend>,
    ) -> Self {
        Self {
            workflow_id,
            task_id,
            executor_id,
            superstep: 0,
            messages: MessageSink::new(),
            outputs: OutputSink::new(),
            events: Vec::new(),
            previous_errors: Vec::new(),
            state_backend,
        }
    }

    /// Create a context with in-memory state (for testing).
    pub fn in_memory(workflow_id: WorkflowId, task_id: TaskId, executor_id: ExecutorId) -> Self {
        Self::new(
            workflow_id,
            task_id,
            executor_id,
            Arc::new(InMemoryStateBackend::new()),
        )
    }

    /// Set the current superstep number.
    pub fn set_superstep(&mut self, superstep: u32) {
        self.superstep = superstep;
    }

    /// Get the current superstep number.
    pub fn superstep(&self) -> u32 {
        self.superstep
    }

    /// Get the workflow ID.
    pub fn workflow_id(&self) -> &WorkflowId {
        &self.workflow_id
    }

    /// Get the task ID.
    pub fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Get the executor ID.
    pub fn executor_id(&self) -> &ExecutorId {
        &self.executor_id
    }

    /// Add errors from previous execution attempts.
    pub fn add_previous_errors(&mut self, errors: impl IntoIterator<Item = ExecutorError>) {
        self.previous_errors.extend(errors);
    }

    /// Drain all messages (for routing to edges).
    pub fn drain_messages(&mut self) -> Vec<TMessage> {
        self.messages.drain()
    }

    /// Drain all outputs (for returning to caller).
    pub fn drain_outputs(&mut self) -> Vec<TOutput> {
        self.outputs.drain()
    }

    /// Drain all events (for publishing).
    pub fn drain_events(&mut self) -> Vec<WorkflowEvent> {
        std::mem::take(&mut self.events)
    }

    /// Check if any messages were sent.
    pub fn has_messages(&self) -> bool {
        !self.messages.is_empty()
    }

    /// Check if any outputs were yielded.
    pub fn has_outputs(&self) -> bool {
        !self.outputs.is_empty()
    }

    // ========================================================================
    // SHARED STATE METHODS
    // ========================================================================

    /// Set a value in shared state.
    ///
    /// Shared state is visible to all executors in the workflow.
    pub async fn set_shared_state<T: Serialize>(
        &self,
        key: &str,
        value: T,
    ) -> Result<(), ExecutorError> {
        let json = serde_json::to_value(value).map_err(|e| {
            ExecutorError::internal(format!("Failed to serialize shared state: {e}"))
        })?;
        let full_key = format!("{}:{}", self.workflow_id, key);
        self.state_backend.set(&full_key, json).await
    }

    /// Get a value from shared state.
    pub async fn get_shared_state<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, ExecutorError> {
        let full_key = format!("{}:{}", self.workflow_id, key);
        match self.state_backend.get(&full_key).await? {
            Some(json) => {
                let value = serde_json::from_value(json).map_err(|e| {
                    ExecutorError::internal(format!("Failed to deserialize shared state: {e}"))
                })?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Delete a value from shared state.
    pub async fn delete_shared_state(&self, key: &str) -> Result<(), ExecutorError> {
        let full_key = format!("{}:{}", self.workflow_id, key);
        self.state_backend.delete(&full_key).await
    }
}

// ============================================================================
// EXECUTOR CONTEXT IMPLEMENTATION
// ============================================================================

#[async_trait]
impl<TMessage, TOutput> ExecutorContext<TMessage, TOutput> for WorkflowContext<TMessage, TOutput>
where
    TMessage: Serialize + Send + Sync,
    TOutput: Serialize + Send + Sync,
{
    async fn send_message(&mut self, message: TMessage) -> Result<(), ExecutorError> {
        self.messages.push(message);
        Ok(())
    }

    async fn yield_output(&mut self, output: TOutput) -> Result<(), ExecutorError> {
        self.outputs.push(output);
        Ok(())
    }

    async fn add_event(
        &mut self,
        event_type: &str,
        data: serde_json::Value,
    ) -> Result<(), ExecutorError> {
        let event = WorkflowEvent::new(event_type, self.executor_id.clone(), data);
        self.events.push(event);
        Ok(())
    }

    fn previous_errors(&self) -> &[ExecutorError] {
        &self.previous_errors
    }
}

// ============================================================================
// CONTEXT BUILDER
// ============================================================================

/// Builder for WorkflowContext.
pub struct WorkflowContextBuilder<TMessage, TOutput>
where
    TMessage: Send,
    TOutput: Send,
{
    workflow_id: WorkflowId,
    task_id: TaskId,
    executor_id: ExecutorId,
    superstep: u32,
    previous_errors: Vec<ExecutorError>,
    state_backend: Option<Arc<dyn SharedStateBackend>>,
    _phantom: std::marker::PhantomData<(TMessage, TOutput)>,
}

impl<TMessage, TOutput> WorkflowContextBuilder<TMessage, TOutput>
where
    TMessage: Send,
    TOutput: Send,
{
    /// Create a new context builder.
    pub fn new(workflow_id: WorkflowId, task_id: TaskId, executor_id: ExecutorId) -> Self {
        Self {
            workflow_id,
            task_id,
            executor_id,
            superstep: 0,
            previous_errors: Vec::new(),
            state_backend: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Set the superstep number.
    pub fn superstep(mut self, superstep: u32) -> Self {
        self.superstep = superstep;
        self
    }

    /// Add previous errors.
    pub fn previous_errors(mut self, errors: Vec<ExecutorError>) -> Self {
        self.previous_errors = errors;
        self
    }

    /// Set the shared state backend.
    pub fn state_backend(mut self, backend: Arc<dyn SharedStateBackend>) -> Self {
        self.state_backend = Some(backend);
        self
    }

    /// Build the context.
    pub fn build(self) -> WorkflowContext<TMessage, TOutput> {
        let state_backend = self
            .state_backend
            .unwrap_or_else(|| Arc::new(InMemoryStateBackend::new()));

        let mut ctx = WorkflowContext::new(
            self.workflow_id,
            self.task_id,
            self.executor_id,
            state_backend,
        );
        ctx.superstep = self.superstep;
        ctx.previous_errors = self.previous_errors;
        ctx
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::graph::LogLevel;

    fn test_ids() -> (WorkflowId, TaskId, ExecutorId) {
        (
            WorkflowId::new("wf-test"),
            TaskId::new("task-test"),
            ExecutorId::new("exe-test"),
        )
    }

    #[tokio::test]
    async fn test_send_message() {
        let (wf_id, task_id, exe_id) = test_ids();
        let mut ctx: WorkflowContext<String, ()> =
            WorkflowContext::in_memory(wf_id, task_id, exe_id);

        ctx.send_message("Hello".to_string()).await.unwrap();
        ctx.send_message("World".to_string()).await.unwrap();

        assert!(ctx.has_messages());
        let messages = ctx.drain_messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], "Hello");
        assert_eq!(messages[1], "World");
    }

    #[tokio::test]
    async fn test_yield_output() {
        let (wf_id, task_id, exe_id) = test_ids();
        let mut ctx: WorkflowContext<(), String> =
            WorkflowContext::in_memory(wf_id, task_id, exe_id);

        ctx.yield_output("Result 1".to_string()).await.unwrap();
        ctx.yield_output("Result 2".to_string()).await.unwrap();

        assert!(ctx.has_outputs());
        let outputs = ctx.drain_outputs();
        assert_eq!(outputs.len(), 2);
    }

    #[tokio::test]
    async fn test_add_event() {
        let (wf_id, task_id, exe_id) = test_ids();
        let mut ctx: WorkflowContext<(), ()> = WorkflowContext::in_memory(wf_id, task_id, exe_id);

        ctx.add_event("progress", serde_json::json!({"percent": 50}))
            .await
            .unwrap();

        let events = ctx.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "progress");
    }

    #[tokio::test]
    async fn test_log() {
        let (wf_id, task_id, exe_id) = test_ids();
        let mut ctx: WorkflowContext<(), ()> = WorkflowContext::in_memory(wf_id, task_id, exe_id);

        ctx.log(LogLevel::Info, "Test message").await.unwrap();

        let events = ctx.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "log");
    }

    #[tokio::test]
    async fn test_shared_state() {
        let (wf_id, task_id, exe_id) = test_ids();
        let ctx: WorkflowContext<(), ()> = WorkflowContext::in_memory(wf_id, task_id, exe_id);

        // Set and get
        ctx.set_shared_state("counter", 42i32).await.unwrap();
        let value: Option<i32> = ctx.get_shared_state("counter").await.unwrap();
        assert_eq!(value, Some(42));

        // Complex types
        ctx.set_shared_state("config", serde_json::json!({"enabled": true}))
            .await
            .unwrap();
        let config: Option<serde_json::Value> = ctx.get_shared_state("config").await.unwrap();
        assert!(config.is_some());

        // Non-existent key
        let missing: Option<String> = ctx.get_shared_state("missing").await.unwrap();
        assert!(missing.is_none());

        // Delete
        ctx.delete_shared_state("counter").await.unwrap();
        let deleted: Option<i32> = ctx.get_shared_state("counter").await.unwrap();
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_previous_errors() {
        let (wf_id, task_id, exe_id) = test_ids();
        let mut ctx: WorkflowContext<(), ()> = WorkflowContext::in_memory(wf_id, task_id, exe_id);

        assert!(ctx.previous_errors().is_empty());

        ctx.add_previous_errors(vec![
            ExecutorError::validation("First error"),
            ExecutorError::validation("Second error"),
        ]);

        assert_eq!(ctx.previous_errors().len(), 2);
    }

    #[tokio::test]
    async fn test_context_builder() {
        let (wf_id, task_id, exe_id) = test_ids();

        let ctx: WorkflowContext<String, String> =
            WorkflowContextBuilder::new(wf_id, task_id, exe_id)
                .superstep(5)
                .previous_errors(vec![ExecutorError::timeout()])
                .build();

        assert_eq!(ctx.superstep(), 5);
        assert_eq!(ctx.previous_errors().len(), 1);
    }
}
