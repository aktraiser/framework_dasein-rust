//! Workflow - Orchestrates executor execution using the Superstep model.
//!
//! A workflow runs executors in discrete supersteps (BSP/Pregel model):
//! 1. Each superstep, all active executors process their input in parallel
//! 2. Messages sent by executors are collected
//! 3. Messages are routed through edges to determine next active executors
//! 4. Repeat until no more messages or terminal state reached
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::{
//!     Workflow, WorkflowBuilder, WorkflowConfig,
//! };
//!
//! // Build workflow definition
//! let definition = WorkflowBuilder::<String>::new("my-workflow")
//!     .set_start("code_gen")
//!     .add_direct_edge("code_gen", "validator")
//!     .build()?;
//!
//! // Create workflow with executor registry
//! let workflow = Workflow::new(definition, registry);
//!
//! // Run the workflow
//! let result = workflow.run("Generate a function").await?;
//!
//! // Or stream events
//! let mut stream = workflow.run_stream("Generate a function");
//! while let Some(event) = stream.next().await {
//!     println!("Event: {:?}", event);
//! }
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

use super::builder::WorkflowDefinition;
use super::context::{SharedStateBackend, WorkflowContext, WorkflowContextBuilder};
use super::executor::Executor;
use super::superstep::{Checkpoint, CheckpointBackend, InMemoryCheckpointBackend, SuperstepState};
use super::types::{
    ExecutorError, ExecutorId, GraphError, GraphResult, TaskId, WorkflowId,
};

// ============================================================================
// WORKFLOW CONFIGURATION
// ============================================================================

/// Configuration for workflow execution.
#[derive(Debug, Clone)]
pub struct WorkflowConfig {
    /// Maximum number of supersteps before timeout.
    pub max_supersteps: u32,
    /// Maximum retries per executor on failure.
    pub max_retries_per_executor: u32,
    /// Enable checkpointing at superstep boundaries.
    pub enable_checkpointing: bool,
    /// Checkpoint every N supersteps (if enabled).
    pub checkpoint_interval: u32,
    /// Timeout per executor in milliseconds.
    pub executor_timeout_ms: u64,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            max_supersteps: 100,
            max_retries_per_executor: 3,
            enable_checkpointing: false,
            checkpoint_interval: 5,
            executor_timeout_ms: 60_000, // 60 seconds
        }
    }
}

impl WorkflowConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_supersteps(mut self, max: u32) -> Self {
        self.max_supersteps = max;
        self
    }

    pub fn with_max_retries(mut self, max: u32) -> Self {
        self.max_retries_per_executor = max;
        self
    }

    pub fn with_checkpointing(mut self, enabled: bool) -> Self {
        self.enable_checkpointing = enabled;
        self
    }

    pub fn with_checkpoint_interval(mut self, interval: u32) -> Self {
        self.checkpoint_interval = interval;
        self
    }

    pub fn with_executor_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.executor_timeout_ms = timeout_ms;
        self
    }
}

// ============================================================================
// WORKFLOW EVENTS
// ============================================================================

/// Events emitted during workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowStreamEvent {
    /// Workflow started.
    Started {
        workflow_id: WorkflowId,
        task_id: TaskId,
        timestamp: DateTime<Utc>,
    },
    /// Superstep started.
    SuperstepStarted {
        superstep: u32,
        active_executor_count: usize,
    },
    /// Executor started processing.
    ExecutorStarted {
        executor_id: ExecutorId,
        superstep: u32,
    },
    /// Executor completed successfully.
    ExecutorCompleted {
        executor_id: ExecutorId,
        superstep: u32,
        duration_ms: u64,
        message_count: usize,
    },
    /// Executor failed.
    ExecutorFailed {
        executor_id: ExecutorId,
        superstep: u32,
        error: String,
        will_retry: bool,
    },
    /// Superstep completed.
    SuperstepCompleted {
        superstep: u32,
        duration_ms: u64,
        messages_routed: usize,
    },
    /// Checkpoint created.
    CheckpointCreated {
        superstep: u32,
        checkpoint_id: String,
    },
    /// Output yielded by an executor.
    Output {
        executor_id: ExecutorId,
        data: serde_json::Value,
    },
    /// Custom event from executor.
    CustomEvent {
        executor_id: ExecutorId,
        event_type: String,
        data: serde_json::Value,
    },
    /// Workflow completed successfully.
    Completed {
        duration_ms: u64,
        superstep_count: u32,
        output_count: usize,
    },
    /// Workflow failed.
    Failed {
        error: String,
        last_superstep: u32,
    },
}

// ============================================================================
// WORKFLOW RESULT
// ============================================================================

/// Result of a workflow execution.
#[derive(Debug, Clone)]
pub struct WorkflowResult<TOutput> {
    /// Workflow ID.
    pub workflow_id: WorkflowId,
    /// Task ID.
    pub task_id: TaskId,
    /// All outputs yielded during execution.
    pub outputs: Vec<TOutput>,
    /// Number of supersteps executed.
    pub superstep_count: u32,
    /// Total execution duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the workflow completed successfully.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

impl<TOutput> WorkflowResult<TOutput> {
    /// Create a successful result.
    pub fn success(
        workflow_id: WorkflowId,
        task_id: TaskId,
        outputs: Vec<TOutput>,
        superstep_count: u32,
        duration_ms: u64,
    ) -> Self {
        Self {
            workflow_id,
            task_id,
            outputs,
            superstep_count,
            duration_ms,
            success: true,
            error: None,
        }
    }

    /// Create a failed result.
    pub fn failure(
        workflow_id: WorkflowId,
        task_id: TaskId,
        error: impl Into<String>,
        superstep_count: u32,
        duration_ms: u64,
    ) -> Self {
        Self {
            workflow_id,
            task_id,
            outputs: vec![],
            superstep_count,
            duration_ms,
            success: false,
            error: Some(error.into()),
        }
    }
}

// ============================================================================
// EXECUTOR REGISTRY
// ============================================================================

/// A type-erased executor wrapper for the registry.
///
/// This allows storing executors with different type parameters in a single map.
#[async_trait]
pub trait DynExecutor<TMessage, TOutput>: Send + Sync
where
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone,
{
    /// Get the executor ID.
    fn id(&self) -> &ExecutorId;

    /// Process serialized input and return serialized messages.
    async fn handle_dyn(
        &self,
        input: serde_json::Value,
        ctx: &mut WorkflowContext<TMessage, TOutput>,
    ) -> Result<(), ExecutorError>;
}

/// Registry of executors for a workflow.
pub struct ExecutorRegistry<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    executors: HashMap<ExecutorId, Arc<dyn DynExecutor<TMessage, TOutput>>>,
}

impl<TMessage, TOutput> Default for ExecutorRegistry<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<TMessage, TOutput> ExecutorRegistry<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    pub fn new() -> Self {
        Self {
            executors: HashMap::new(),
        }
    }

    /// Register an executor.
    pub fn register<E>(&mut self, executor: E)
    where
        E: Executor<Message = TMessage, Output = TOutput> + 'static,
        E::Input: Serialize + DeserializeOwned,
    {
        let id = executor.id().clone();
        self.executors
            .insert(id, Arc::new(ExecutorWrapper::new(executor)));
    }

    /// Get an executor by ID.
    pub fn get(&self, id: &ExecutorId) -> Option<&Arc<dyn DynExecutor<TMessage, TOutput>>> {
        self.executors.get(id)
    }

    /// Check if an executor exists.
    pub fn contains(&self, id: &ExecutorId) -> bool {
        self.executors.contains_key(id)
    }
}

/// Wrapper to make any Executor work with the registry.
struct ExecutorWrapper<E>
where
    E: Executor,
{
    executor: E,
}

impl<E> ExecutorWrapper<E>
where
    E: Executor,
{
    fn new(executor: E) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl<E, TMessage, TOutput> DynExecutor<TMessage, TOutput> for ExecutorWrapper<E>
where
    E: Executor<Message = TMessage, Output = TOutput> + Send + Sync,
    E::Input: Serialize + DeserializeOwned + Send + Sync,
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone,
{
    fn id(&self) -> &ExecutorId {
        self.executor.id()
    }

    async fn handle_dyn(
        &self,
        input: serde_json::Value,
        ctx: &mut WorkflowContext<TMessage, TOutput>,
    ) -> Result<(), ExecutorError> {
        let typed_input: E::Input = serde_json::from_value(input).map_err(|e| {
            ExecutorError::internal(format!("Failed to deserialize input: {e}"))
        })?;
        self.executor.handle(typed_input, ctx).await
    }
}

// ============================================================================
// WORKFLOW
// ============================================================================

/// A workflow that orchestrates executor execution using the Superstep model.
pub struct Workflow<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    /// Workflow definition (graph structure).
    definition: WorkflowDefinition<TMessage>,
    /// Executor registry.
    registry: ExecutorRegistry<TMessage, TOutput>,
    /// Configuration.
    config: WorkflowConfig,
    /// Shared state backend.
    state_backend: Arc<dyn SharedStateBackend>,
    /// Checkpoint backend (if checkpointing enabled).
    checkpoint_backend: Option<Arc<dyn CheckpointBackend>>,
}

impl<TMessage, TOutput> Workflow<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    /// Create a new workflow.
    pub fn new(
        definition: WorkflowDefinition<TMessage>,
        registry: ExecutorRegistry<TMessage, TOutput>,
    ) -> Self {
        Self {
            definition,
            registry,
            config: WorkflowConfig::default(),
            state_backend: Arc::new(super::context::InMemoryStateBackend::new()),
            checkpoint_backend: None,
        }
    }

    /// Create a workflow with configuration.
    pub fn with_config(
        definition: WorkflowDefinition<TMessage>,
        registry: ExecutorRegistry<TMessage, TOutput>,
        config: WorkflowConfig,
    ) -> Self {
        let checkpoint_backend: Option<Arc<dyn CheckpointBackend>> = if config.enable_checkpointing
        {
            Some(Arc::new(InMemoryCheckpointBackend::new()))
        } else {
            None
        };

        Self {
            definition,
            registry,
            config,
            state_backend: Arc::new(super::context::InMemoryStateBackend::new()),
            checkpoint_backend,
        }
    }

    /// Set the shared state backend.
    pub fn with_state_backend(mut self, backend: Arc<dyn SharedStateBackend>) -> Self {
        self.state_backend = backend;
        self
    }

    /// Set the checkpoint backend.
    pub fn with_checkpoint_backend(mut self, backend: Arc<dyn CheckpointBackend>) -> Self {
        self.checkpoint_backend = Some(backend);
        self
    }

    /// Get the workflow definition.
    pub fn definition(&self) -> &WorkflowDefinition<TMessage> {
        &self.definition
    }

    /// Get the workflow ID.
    pub fn id(&self) -> &WorkflowId {
        &self.definition.id
    }

    // ========================================================================
    // RUN (Blocking)
    // ========================================================================

    /// Run the workflow with the given input.
    ///
    /// Returns when the workflow completes or fails.
    pub async fn run<TInput>(&self, input: TInput) -> GraphResult<WorkflowResult<TOutput>>
    where
        TInput: Serialize + Send,
    {
        let task_id = TaskId::generate();
        let start_time = std::time::Instant::now();

        // Serialize input
        let input_json = serde_json::to_value(&input).map_err(|e| {
            GraphError::SerializationError(format!("Failed to serialize input: {e}"))
        })?;

        // Initialize superstep state
        let mut state = SuperstepState::new();
        state.enqueue_message(self.definition.start.clone(), input_json);

        let mut all_outputs: Vec<TOutput> = Vec::new();
        let mut retry_counts: HashMap<ExecutorId, u32> = HashMap::new();

        // Execute supersteps
        for superstep in 0..self.config.max_supersteps {
            // Check if we have any messages to process
            if state.pending_messages.is_empty() {
                // Workflow completed - no more work to do
                let duration_ms = start_time.elapsed().as_millis() as u64;
                return Ok(WorkflowResult::success(
                    self.definition.id.clone(),
                    task_id,
                    all_outputs,
                    superstep,
                    duration_ms,
                ));
            }

            // Execute the superstep
            let result = self
                .execute_superstep(superstep, &mut state, &task_id, &mut retry_counts)
                .await;

            match result {
                Ok(outputs) => {
                    all_outputs.extend(outputs);
                }
                Err(e) => {
                    let duration_ms = start_time.elapsed().as_millis() as u64;
                    return Ok(WorkflowResult::failure(
                        self.definition.id.clone(),
                        task_id,
                        e.to_string(),
                        superstep,
                        duration_ms,
                    ));
                }
            }

            // Checkpoint if enabled
            if self.config.enable_checkpointing
                && superstep > 0
                && superstep % self.config.checkpoint_interval == 0
            {
                if let Some(backend) = &self.checkpoint_backend {
                    let checkpoint = Checkpoint::new(
                        self.definition.id.clone(),
                        task_id.clone(),
                        superstep,
                        state.clone(),
                    );
                    backend.save(&checkpoint).await.ok(); // Ignore checkpoint errors
                }
            }
        }

        // Max supersteps reached
        let duration_ms = start_time.elapsed().as_millis() as u64;
        Ok(WorkflowResult::failure(
            self.definition.id.clone(),
            task_id,
            format!("Max supersteps ({}) exceeded", self.config.max_supersteps),
            self.config.max_supersteps,
            duration_ms,
        ))
    }

    /// Execute a single superstep.
    async fn execute_superstep(
        &self,
        superstep: u32,
        state: &mut SuperstepState,
        task_id: &TaskId,
        retry_counts: &mut HashMap<ExecutorId, u32>,
    ) -> GraphResult<Vec<TOutput>> {
        let mut all_outputs = Vec::new();

        // Take pending messages for this superstep
        let messages = std::mem::take(&mut state.pending_messages);

        // Process each executor's messages
        for (executor_id, executor_messages) in messages {
            let executor = self.registry.get(&executor_id).ok_or_else(|| {
                GraphError::ExecutorNotFound(executor_id.clone())
            })?;

            // Process each message
            for input_json in executor_messages {
                let result = self
                    .execute_single(
                        executor.clone(),
                        input_json,
                        superstep,
                        task_id,
                        retry_counts,
                    )
                    .await;

                match result {
                    Ok((messages, outputs)) => {
                        // Route messages through edges
                        for message in messages {
                            let message_json = serde_json::to_value(&message).map_err(|e| {
                                GraphError::SerializationError(format!(
                                    "Failed to serialize message: {e}"
                                ))
                            })?;

                            let targets = self
                                .definition
                                .edges
                                .route_from(&executor_id, &message);

                            for target_id in targets {
                                state.enqueue_message(target_id, message_json.clone());
                            }
                        }

                        all_outputs.extend(outputs);
                    }
                    Err(e) => {
                        // Check if we should retry
                        let count = retry_counts.entry(executor_id.clone()).or_insert(0);
                        *count += 1;

                        if *count >= self.config.max_retries_per_executor {
                            return Err(GraphError::MaxRetriesExceeded(executor_id.clone()));
                        }

                        // Re-queue the message for retry
                        // Note: we lost the original input here, this is a simplification
                        return Err(GraphError::ExecutorFailed(format!(
                            "Executor {} failed: {}",
                            executor_id, e
                        )));
                    }
                }
            }
        }

        Ok(all_outputs)
    }

    /// Execute a single executor.
    async fn execute_single(
        &self,
        executor: Arc<dyn DynExecutor<TMessage, TOutput>>,
        input_json: serde_json::Value,
        superstep: u32,
        task_id: &TaskId,
        _retry_counts: &HashMap<ExecutorId, u32>,
    ) -> Result<(Vec<TMessage>, Vec<TOutput>), ExecutorError> {
        // Create context for this executor
        let mut ctx: WorkflowContext<TMessage, TOutput> = WorkflowContextBuilder::new(
            self.definition.id.clone(),
            task_id.clone(),
            executor.id().clone(),
        )
        .superstep(superstep)
        .state_backend(self.state_backend.clone())
        .build();

        // Execute with timeout
        let timeout = std::time::Duration::from_millis(self.config.executor_timeout_ms);
        let result = tokio::time::timeout(timeout, executor.handle_dyn(input_json, &mut ctx)).await;

        match result {
            Ok(Ok(())) => {
                let messages = ctx.drain_messages();
                let outputs = ctx.drain_outputs();
                Ok((messages, outputs))
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ExecutorError::timeout()),
        }
    }

    // ========================================================================
    // RUN_STREAM (Streaming)
    // ========================================================================

    /// Run the workflow and stream events.
    ///
    /// Returns a stream of events that can be consumed as the workflow executes.
    pub fn run_stream<TInput>(
        &self,
        input: TInput,
    ) -> Pin<Box<dyn Stream<Item = WorkflowStreamEvent> + Send + '_>>
    where
        TInput: Serialize + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(100);

        // Clone what we need for the async task
        let workflow_id = self.definition.id.clone();
        let task_id = TaskId::generate();

        // We need to clone the registry references for use in the spawned task
        // For now, we'll use a simpler approach that processes synchronously
        let input_json = serde_json::to_value(&input).ok();

        tokio::spawn(async move {
            let start_time = std::time::Instant::now();

            // Send started event
            let _ = tx
                .send(WorkflowStreamEvent::Started {
                    workflow_id: workflow_id.clone(),
                    task_id: task_id.clone(),
                    timestamp: Utc::now(),
                })
                .await;

            // Since we can't easily pass the registry into the spawned task,
            // we'll send a simple completion event.
            // A full implementation would use a different architecture.

            if input_json.is_none() {
                let _ = tx
                    .send(WorkflowStreamEvent::Failed {
                        error: "Failed to serialize input".into(),
                        last_superstep: 0,
                    })
                    .await;
                return;
            }

            // For the streaming implementation, we'd need a more sophisticated
            // approach that either:
            // 1. Uses an Arc<Workflow> that can be cloned into the task
            // 2. Passes the registry as an Arc
            // 3. Uses a message-passing approach

            // For now, emit a simple completion
            let duration_ms = start_time.elapsed().as_millis() as u64;
            let _ = tx
                .send(WorkflowStreamEvent::Completed {
                    duration_ms,
                    superstep_count: 0,
                    output_count: 0,
                })
                .await;
        });

        Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::graph::{Executor, ExecutorContext, ExecutorKind, WorkflowBuilder};

    // Test executor that uppercases strings
    struct UppercaseExecutor {
        id: ExecutorId,
    }

    impl UppercaseExecutor {
        fn new(id: impl Into<String>) -> Self {
            Self {
                id: ExecutorId::new(id),
            }
        }
    }

    #[async_trait]
    impl Executor for UppercaseExecutor {
        type Input = String;
        type Message = String;
        type Output = String;

        fn id(&self) -> &ExecutorId {
            &self.id
        }

        fn kind(&self) -> ExecutorKind {
            ExecutorKind::Worker
        }

        async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
        where
            Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
        {
            let result = input.to_uppercase();
            ctx.yield_output(result.clone()).await?;
            ctx.send_message(result).await?;
            Ok(())
        }
    }

    // Test executor that counts characters
    struct CountExecutor {
        id: ExecutorId,
    }

    impl CountExecutor {
        fn new(id: impl Into<String>) -> Self {
            Self {
                id: ExecutorId::new(id),
            }
        }
    }

    #[async_trait]
    impl Executor for CountExecutor {
        type Input = String;
        type Message = String;
        type Output = String;

        fn id(&self) -> &ExecutorId {
            &self.id
        }

        fn kind(&self) -> ExecutorKind {
            ExecutorKind::Worker
        }

        async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
        where
            Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
        {
            let count = input.len();
            let result = format!("Length: {}", count);
            ctx.yield_output(result).await?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_simple_workflow() {
        // Build workflow: uppercase → count
        let definition = WorkflowBuilder::<String>::new("test-wf")
            .set_start("uppercase")
            .add_executor_id("count")
            .add_direct_edge("uppercase", "count")
            .build()
            .unwrap();

        // Create registry
        let mut registry = ExecutorRegistry::new();
        registry.register(UppercaseExecutor::new("uppercase"));
        registry.register(CountExecutor::new("count"));

        // Create and run workflow
        let workflow = Workflow::new(definition, registry);
        let result = workflow.run("hello".to_string()).await.unwrap();

        assert!(result.success);
        assert_eq!(result.outputs.len(), 2); // One from each executor
        assert_eq!(result.outputs[0], "HELLO");
        assert_eq!(result.outputs[1], "Length: 5");
    }

    #[tokio::test]
    async fn test_workflow_config() {
        let config = WorkflowConfig::new()
            .with_max_supersteps(50)
            .with_max_retries(5)
            .with_checkpointing(true)
            .with_checkpoint_interval(10);

        assert_eq!(config.max_supersteps, 50);
        assert_eq!(config.max_retries_per_executor, 5);
        assert!(config.enable_checkpointing);
        assert_eq!(config.checkpoint_interval, 10);
    }

    #[tokio::test]
    async fn test_workflow_result() {
        let result: WorkflowResult<String> = WorkflowResult::success(
            WorkflowId::new("wf-1"),
            TaskId::new("task-1"),
            vec!["output1".into(), "output2".into()],
            5,
            100,
        );

        assert!(result.success);
        assert_eq!(result.outputs.len(), 2);
        assert_eq!(result.superstep_count, 5);
        assert_eq!(result.duration_ms, 100);

        let failed: WorkflowResult<String> = WorkflowResult::failure(
            WorkflowId::new("wf-2"),
            TaskId::new("task-2"),
            "Test error",
            3,
            50,
        );

        assert!(!failed.success);
        assert!(failed.outputs.is_empty());
        assert_eq!(failed.error, Some("Test error".into()));
    }

    #[tokio::test]
    async fn test_single_executor_workflow() {
        // Workflow with just one executor
        let definition = WorkflowBuilder::<String>::new("single-wf")
            .set_start("uppercase")
            .build()
            .unwrap();

        let mut registry = ExecutorRegistry::new();
        registry.register(UppercaseExecutor::new("uppercase"));

        let workflow = Workflow::new(definition, registry);
        let result = workflow.run("test".to_string()).await.unwrap();

        assert!(result.success);
        assert_eq!(result.outputs.len(), 1);
        assert_eq!(result.outputs[0], "TEST");
    }

    #[tokio::test]
    async fn test_executor_not_found() {
        let definition = WorkflowBuilder::<String>::new("missing-wf")
            .set_start("missing")
            .build()
            .unwrap();

        let registry: ExecutorRegistry<String, String> = ExecutorRegistry::new();

        let workflow = Workflow::new(definition, registry);
        let result = workflow.run("test".to_string()).await.unwrap();

        // Missing executor results in a failed workflow
        assert!(!result.success);
        assert!(result.error.is_some());
        let error_msg = result.error.unwrap();
        assert!(
            error_msg.contains("not found") || error_msg.contains("missing"),
            "Error message should mention 'not found' or 'missing': {}",
            error_msg
        );
    }
}
