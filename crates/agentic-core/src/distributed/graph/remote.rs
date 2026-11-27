//! Running workflow steps in other processes.
//!
//! A [`RemoteExecutor`] is an ordinary workflow step that does not do the
//! work itself: it puts its input on a task queue and waits for the result.
//! A [`RemoteWorker`], in any process on any machine that can reach the
//! queue, hosts the real executors, takes the tasks and publishes what they
//! produced. The workflow engine does not know the difference.
//!
//! ```text
//!  process A (workflow)                         process B, C... (workers)
//!  ┌────────────────────┐      task queue      ┌─────────────────────┐
//!  │ RemoteExecutor "x" │ ── input ─────────▶  │ RemoteWorker        │
//!  │                    │ ◀─ messages/outputs  │   real executor "x" │
//!  └────────────────────┘                      └─────────────────────┘
//! ```
//!
//! With [`DurableQueue`](crate::distributed::bus::DurableQueue) the queue is
//! NATS JetStream: tasks survive a restart, a worker that dies has its task
//! handed to another one, and adding a machine is starting one more worker.
//! [`InMemoryTaskQueue`] gives the same behaviour inside one process, for
//! development and tests.
//!
//! # What to know before relying on it
//!
//! - **At least once.** A task whose worker died mid-way is run again. A
//!   remote executor with side effects must make them idempotent; its
//!   invocation is identified by workflow, task, superstep, executor and
//!   input, and the same invocation always gets the same queue task ID.
//! - **Results are remembered.** A retried or resumed invocation whose task
//!   already finished gets the recorded result instead of running again.
//!   That includes a recorded failure: transient errors are retried by the
//!   worker, and what it finally reports is final for that invocation.
//! - **Shared state needs a shared backend.** A remote executor only sees
//!   the workflow's shared state if both sides use a backend they can both
//!   reach, such as [`NatsStateBackend`](super::NatsStateBackend).
//! - **Events stay local.** Events a remote executor emits with `add_event`
//!   are not sent back to the workflow.
//! - Workers of one group must host the same executors: any of them may
//!   receive any task of the group.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use super::context::{InMemoryStateBackend, SharedStateBackend, WorkflowContextBuilder};
use super::executor::{Executor, ExecutorContext, ExecutorKind};
use super::types::{ExecutorError, ExecutorId, TaskId, WorkflowId};
use super::workflow::ExecutorRegistry;
use crate::distributed::bus::{
    BusError, DurableQueue, DurableWorker, Task, TaskDelivery, TaskOutcome,
};

// ============================================================================
// QUEUE ABSTRACTION
// ============================================================================

/// Where a [`RemoteExecutor`] sends work and reads results.
#[async_trait]
pub trait TaskQueue: Send + Sync {
    /// Accept a task. Submitting the same ID and payload again is a no-op.
    async fn submit(&self, task: Task) -> Result<(), BusError>;

    /// The recorded state of a task, or `None` if it is unknown.
    async fn outcome(&self, id: &str) -> Result<Option<TaskOutcome>, BusError>;
}

/// Where a [`RemoteWorker`] takes its work.
#[async_trait]
pub trait TaskSource: Send + Sync {
    /// A task taken from the queue and not yet reported on.
    type Claim: TaskClaim;

    /// Wait up to `timeout` for a task.
    async fn next(&self, timeout: Duration) -> Result<Option<Self::Claim>, BusError>;
}

/// A task a worker has taken. It must end with `complete` or `fail`;
/// dropping it hands the task to another worker.
#[async_trait]
pub trait TaskClaim: Send {
    /// The task to run.
    fn task(&self) -> &Task;

    /// Record the result and remove the task from the queue.
    async fn complete(self, result: Value) -> Result<(), BusError>;

    /// Record a final failure and remove the task from the queue.
    async fn fail(self, reason: String) -> Result<(), BusError>;

    /// Tell the queue the task is still being worked on.
    async fn heartbeat(&self) -> Result<(), BusError>;
}

#[async_trait]
impl TaskQueue for DurableQueue {
    async fn submit(&self, task: Task) -> Result<(), BusError> {
        self.publish(task).await
    }

    async fn outcome(&self, id: &str) -> Result<Option<TaskOutcome>, BusError> {
        DurableQueue::outcome(self, id).await
    }
}

#[async_trait]
impl TaskSource for DurableWorker {
    type Claim = TaskDelivery;

    async fn next(&self, timeout: Duration) -> Result<Option<TaskDelivery>, BusError> {
        DurableWorker::next(self, timeout).await
    }
}

#[async_trait]
impl TaskClaim for TaskDelivery {
    fn task(&self) -> &Task {
        &self.task
    }

    async fn complete(self, result: Value) -> Result<(), BusError> {
        TaskDelivery::complete(self, result).await
    }

    async fn fail(self, reason: String) -> Result<(), BusError> {
        TaskDelivery::fail(self, reason).await
    }

    async fn heartbeat(&self) -> Result<(), BusError> {
        TaskDelivery::heartbeat(self).await
    }
}

// ============================================================================
// IN-MEMORY QUEUE
// ============================================================================

#[derive(Default)]
struct MemoryQueueState {
    /// Tasks waiting for a worker, oldest first.
    waiting: std::collections::VecDeque<Task>,
    /// Every accepted task and what became of it.
    records: std::collections::HashMap<String, (Task, TaskOutcome)>,
}

/// A task queue living in one process, with the same contract as the durable
/// one except durability: results are remembered, a dropped claim puts the
/// task back, and a known task ID is not run twice.
///
/// It is both the [`TaskQueue`] and the [`TaskSource`]; all tasks form one
/// group. Use it to develop and test remote steps without NATS.
#[derive(Clone, Default)]
pub struct InMemoryTaskQueue {
    state: Arc<std::sync::Mutex<MemoryQueueState>>,
    arrived: Arc<tokio::sync::Notify>,
}

impl InMemoryTaskQueue {
    /// Create an empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, MemoryQueueState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl TaskQueue for InMemoryTaskQueue {
    async fn submit(&self, task: Task) -> Result<(), BusError> {
        {
            let mut state = self.state();
            if let Some((known, _)) = state.records.get(&task.id) {
                if known.payload != task.payload {
                    return Err(BusError::PublishFailed(
                        "Task ID is already assigned to different work".into(),
                    ));
                }
                return Ok(());
            }
            state
                .records
                .insert(task.id.clone(), (task.clone(), TaskOutcome::Pending));
            state.waiting.push_back(task);
        }
        self.arrived.notify_one();
        Ok(())
    }

    async fn outcome(&self, id: &str) -> Result<Option<TaskOutcome>, BusError> {
        Ok(self
            .state()
            .records
            .get(id)
            .map(|(_, outcome)| outcome.clone()))
    }
}

#[async_trait]
impl TaskSource for InMemoryTaskQueue {
    type Claim = InMemoryClaim;

    async fn next(&self, timeout: Duration) -> Result<Option<InMemoryClaim>, BusError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // Register for the wake-up before looking, so a task submitted
            // in between is not missed.
            let arrived = self.arrived.notified();
            if let Some(task) = self.state().waiting.pop_front() {
                return Ok(Some(InMemoryClaim {
                    queue: self.clone(),
                    task: Some(task),
                }));
            }
            if tokio::time::timeout_at(deadline, arrived).await.is_err() {
                return Ok(None);
            }
        }
    }
}

/// A task taken from an [`InMemoryTaskQueue`].
pub struct InMemoryClaim {
    queue: InMemoryTaskQueue,
    /// `None` once the task was reported on.
    task: Option<Task>,
}

impl InMemoryClaim {
    fn finish(mut self, outcome: TaskOutcome) {
        if let Some(task) = self.task.take() {
            if let Some(record) = self.queue.state().records.get_mut(&task.id) {
                record.1 = outcome;
            }
        }
    }
}

impl Drop for InMemoryClaim {
    fn drop(&mut self) {
        // Not reported on: the worker gave up or died. Hand the task back.
        if let Some(task) = self.task.take() {
            self.queue.state().waiting.push_front(task);
            self.queue.arrived.notify_one();
        }
    }
}

#[async_trait]
impl TaskClaim for InMemoryClaim {
    fn task(&self) -> &Task {
        self.task
            .as_ref()
            .unwrap_or_else(|| unreachable!("a claim holds its task until it is consumed"))
    }

    async fn complete(self, result: Value) -> Result<(), BusError> {
        self.finish(TaskOutcome::Completed(result));
        Ok(())
    }

    async fn fail(self, reason: String) -> Result<(), BusError> {
        self.finish(TaskOutcome::Failed(reason));
        Ok(())
    }

    async fn heartbeat(&self) -> Result<(), BusError> {
        Ok(())
    }
}

// ============================================================================
// WIRE FORMAT
// ============================================================================

/// What a remote executor asks a worker to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteCall {
    executor: String,
    workflow_id: String,
    task_id: String,
    superstep: u32,
    input: Value,
}

/// What the worker's executor produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RemoteReply {
    #[serde(default)]
    messages: Vec<Value>,
    #[serde(default)]
    outputs: Vec<Value>,
}

// ============================================================================
// REMOTE EXECUTOR
// ============================================================================

/// A workflow step whose work is done by a [`RemoteWorker`].
///
/// Register it in the workflow under the ID of the executor the workers
/// host. `I`, `M` and `O` are that executor's input, message and output
/// types.
pub struct RemoteExecutor<I, M, O, Q> {
    id: ExecutorId,
    queue: Q,
    group: String,
    wait: Duration,
    types: PhantomData<fn(I) -> (M, O)>,
}

impl<I, M, O, Q: TaskQueue> RemoteExecutor<I, M, O, Q> {
    /// Create a remote step for executor `id`, served by the workers of
    /// `group` on `queue`. A group name is 1 to 64 ASCII letters, digits,
    /// underscores or hyphens.
    pub fn new(id: impl Into<ExecutorId>, queue: Q, group: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            queue,
            group: group.into(),
            wait: Duration::from_secs(50),
            types: PhantomData,
        }
    }

    /// Set how long one invocation waits for its result (default 50 s).
    ///
    /// Keep it below the workflow's executor timeout (60 s by default), or
    /// raise both: when the wait runs out the step fails as a timeout, and a
    /// retry resumes waiting for the same task rather than starting another.
    #[must_use]
    pub const fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }
}

/// Queue task ID of an invocation: the same step with the same input always
/// maps to the same task, so a retry or a resumed workflow finds the work
/// already submitted, or already done, instead of starting it again.
fn task_id_for(call: &RemoteCall) -> String {
    let identity = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        call.workflow_id, call.task_id, call.superstep, call.executor, call.input
    );
    format!("{:x}", md5::compute(identity.as_bytes()))
}

#[async_trait]
impl<I, M, O, Q> Executor for RemoteExecutor<I, M, O, Q>
where
    I: Serialize + DeserializeOwned + Send + Sync,
    M: Serialize + DeserializeOwned + Send + Sync,
    O: Serialize + DeserializeOwned + Send + Sync,
    Q: TaskQueue,
{
    type Input = I;
    type Message = M;
    type Output = O;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: I, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<M, O> + Send,
    {
        let retriable = |message: String| ExecutorError::new(self.id.clone(), message);
        let fatal = |message: String| {
            let mut error = ExecutorError::new(self.id.clone(), message);
            error.retriable = false;
            error
        };

        let invocation = ctx.invocation();
        let call = RemoteCall {
            executor: self.id.as_str().to_owned(),
            // Outside a workflow there is no invocation to key on: every
            // call is then a task of its own.
            workflow_id: invocation.as_ref().map_or_else(
                || "detached".to_owned(),
                |invocation| invocation.workflow_id.as_str().to_owned(),
            ),
            task_id: invocation.as_ref().map_or_else(
                || uuid::Uuid::new_v4().to_string(),
                |invocation| invocation.task_id.as_str().to_owned(),
            ),
            superstep: invocation
                .as_ref()
                .map_or(0, |invocation| invocation.superstep),
            input: serde_json::to_value(&input).map_err(|e| fatal(e.to_string()))?,
        };
        let id = task_id_for(&call);
        let payload = serde_json::to_value(&call).map_err(|e| fatal(e.to_string()))?;
        self.queue
            .submit(Task::new(id.clone(), self.group.clone(), payload))
            .await
            .map_err(|e| retriable(format!("could not submit remote task: {e}")))?;

        let deadline = tokio::time::Instant::now() + self.wait;
        let mut pause = Duration::from_millis(20);
        let reply = loop {
            match self.queue.outcome(&id).await {
                Ok(Some(TaskOutcome::Completed(value))) => break value,
                Ok(Some(TaskOutcome::Failed(reason))) => {
                    return Err(fatal(format!("remote executor failed: {reason}")));
                }
                Ok(Some(TaskOutcome::Pending) | None) => {}
                Err(e) => return Err(retriable(format!("could not read remote result: {e}"))),
            }
            if tokio::time::Instant::now() + pause > deadline {
                return Err(ExecutorError::timeout());
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 2).min(Duration::from_millis(500));
        };

        let reply: RemoteReply = serde_json::from_value(reply)
            .map_err(|e| fatal(format!("unreadable remote result: {e}")))?;
        for message in reply.messages {
            let message = serde_json::from_value(message)
                .map_err(|e| fatal(format!("remote message: {e}")))?;
            ctx.send_message(message).await?;
        }
        for output in reply.outputs {
            let output =
                serde_json::from_value(output).map_err(|e| fatal(format!("remote output: {e}")))?;
            ctx.yield_output(output).await?;
        }
        Ok(())
    }
}

// ============================================================================
// REMOTE WORKER
// ============================================================================

/// Hosts executors and runs the tasks remote steps send to them.
pub struct RemoteWorker<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    registry: ExecutorRegistry<TMessage, TOutput>,
    state: Arc<dyn SharedStateBackend>,
    max_attempts: u32,
    executor_timeout: Duration,
    heartbeat: Duration,
}

impl<TMessage, TOutput> RemoteWorker<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
    TOutput: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    /// Create a worker hosting the executors of `registry`.
    #[must_use]
    pub fn new(registry: ExecutorRegistry<TMessage, TOutput>) -> Self {
        Self {
            registry,
            state: Arc::new(InMemoryStateBackend::new()),
            max_attempts: 3,
            executor_timeout: Duration::from_secs(60),
            heartbeat: Duration::from_secs(5),
        }
    }

    /// Use the shared state backend the workflow uses, so executors here read
    /// and write the same state. Without it they get a private, empty one.
    #[must_use]
    pub fn with_state_backend(mut self, state: Arc<dyn SharedStateBackend>) -> Self {
        self.state = state;
        self
    }

    /// Set how many times an executor is tried when it reports a transient
    /// error (default 3).
    #[must_use]
    pub const fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts;
        self
    }

    /// Set the time allowed to one attempt (default 60 s).
    #[must_use]
    pub const fn with_executor_timeout(mut self, timeout: Duration) -> Self {
        self.executor_timeout = timeout;
        self
    }

    /// Set how often the queue is told a task is still running (default 5 s).
    /// Keep it well below the queue's acknowledgement deadline.
    #[must_use]
    pub const fn with_heartbeat(mut self, every: Duration) -> Self {
        self.heartbeat = every;
        self
    }

    /// Take and run tasks until `shutdown` completes. A task in progress is
    /// finished first.
    ///
    /// Tasks are run one at a time; call `serve` from several tasks, or start
    /// several workers, to run more in parallel.
    ///
    /// # Errors
    /// Returns an error if the queue can no longer be read.
    pub async fn serve<S: TaskSource>(
        &self,
        source: &S,
        shutdown: impl std::future::Future<Output = ()> + Send,
    ) -> Result<(), BusError> {
        tokio::pin!(shutdown);
        loop {
            let claim = tokio::select! {
                () = &mut shutdown => return Ok(()),
                claim = source.next(Duration::from_secs(1)) => claim?,
            };
            if let Some(claim) = claim {
                self.run(claim).await;
            }
        }
    }

    /// Run one claimed task and report its result.
    pub async fn run<C: TaskClaim>(&self, claim: C) {
        let task_id = claim.task().id.clone();
        let reported = match self.execute(&claim).await {
            Ok(reply) => claim.complete(reply).await,
            Err(reason) => claim.fail(reason).await,
        };
        if let Err(error) = reported {
            // The task stays claimed and will be delivered again.
            tracing::warn!(task = %task_id, %error, "Could not report a remote task result");
        }
    }

    /// Run the executor a task names; `Err` is the final failure reason.
    async fn execute<C: TaskClaim>(&self, claim: &C) -> Result<Value, String> {
        let call: RemoteCall = serde_json::from_value(claim.task().payload.clone())
            .map_err(|e| format!("not a remote executor call: {e}"))?;
        let executor = self
            .registry
            .get(&ExecutorId::new(call.executor.clone()))
            .ok_or_else(|| format!("executor `{}` is not hosted by this worker", call.executor))?
            .clone();

        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut ctx = WorkflowContextBuilder::new(
                WorkflowId::new(call.workflow_id.clone()),
                TaskId::new(call.task_id.clone()),
                executor.id().clone(),
            )
            .superstep(call.superstep)
            .state_backend(self.state.clone())
            .build();

            let result = {
                let work = tokio::time::timeout(
                    self.executor_timeout,
                    executor.handle_dyn(call.input.clone(), &mut ctx),
                );
                tokio::pin!(work);
                let mut beat = tokio::time::interval(self.heartbeat);
                beat.tick().await; // the first tick is immediate
                loop {
                    tokio::select! {
                        result = &mut work => break result,
                        _ = beat.tick() => {
                            if let Err(error) = claim.heartbeat().await {
                                tracing::debug!(%error, "Remote task heartbeat failed");
                            }
                        }
                    }
                }
            };

            let error = match result {
                Ok(Ok(())) => {
                    let messages = ctx
                        .drain_messages()
                        .iter()
                        .map(serde_json::to_value)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| format!("unserializable message: {e}"))?;
                    let outputs = ctx
                        .drain_outputs()
                        .iter()
                        .map(serde_json::to_value)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| format!("unserializable output: {e}"))?;
                    let reply = RemoteReply { messages, outputs };
                    return serde_json::to_value(reply).map_err(|e| e.to_string());
                }
                Ok(Err(error)) => error,
                Err(_) => ExecutorError::timeout(),
            };
            if !error.retriable || attempt >= self.max_attempts.max(1) {
                return Err(format!("{error} (after {attempt} attempt(s))"));
            }
            tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
        }
    }
}
