//! Durable FIFO work distribution using JetStream and a persistent task ledger.
//!
//! Delivery is at least once: workers must make external side effects idempotent
//! using `Task::id`. A completed ledger entry prevents completed work from being
//! run again after an ACK loss. Dropped deliveries are redistributed after the
//! server-side ACK deadline. All workers for a supervisor share one consumer.

use super::{BusError, NatsClient, Task};
use async_nats::jetstream::{
    self,
    consumer::{self, Consumer},
    kv, stream,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// Capacity and lease settings. Queue namespaces must use identical settings.
#[derive(Debug, Clone)]
pub struct DurableQueueConfig {
    pub max_pending: i64,
    pub max_queue_bytes: i64,
    pub max_result_bytes: i64,
    pub ack_wait: Duration,
    pub max_in_flight: i64,
}
impl Default for DurableQueueConfig {
    fn default() -> Self {
        Self {
            max_pending: 1000,
            max_queue_bytes: 512 * 1024 * 1024,
            max_result_bytes: 512 * 1024 * 1024,
            ack_wait: Duration::from_secs(30),
            max_in_flight: 128,
        }
    }
}

/// Persistent state of accepted work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", content = "value")]
pub enum TaskOutcome {
    Pending,
    Completed(Value),
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    task: Task,
    outcome: TaskOutcome,
}

/// A namespaced work queue. Completed outcomes remain in the bounded ledger.
#[derive(Clone)]
pub struct DurableQueue {
    context: jetstream::Context,
    stream: stream::Stream,
    ledger: kv::Store,
    namespace: String,
    config: DurableQueueConfig,
}

fn error(e: impl std::fmt::Display) -> BusError {
    BusError::PublishFailed(e.to_string())
}
fn token(s: &str) -> Result<(), BusError> {
    if s.is_empty()
        || s.len() > 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(error(
            "Queue/supervisor names must be 1..64 ASCII letters, digits, underscores or hyphens",
        ));
    }
    Ok(())
}
fn key(id: &str) -> Result<String, BusError> {
    if id.is_empty() || id.len() > 128 {
        return Err(error("Task ID must contain 1..128 bytes"));
    }
    use std::fmt::Write;
    let mut key = String::from("task_");
    for byte in id.bytes() {
        let _ = write!(key, "{byte:02x}");
    }
    Ok(key)
}

impl DurableQueue {
    /// Open a queue, creating its file-backed stream and ledger if needed.
    pub async fn new(
        nats: Arc<NatsClient>,
        namespace: &str,
        config: DurableQueueConfig,
    ) -> Result<Self, BusError> {
        token(namespace)?;
        if config.max_queue_bytes < 1
            || config.max_result_bytes < 1
            || config.max_pending < 1
            || config.max_in_flight < 1
            || config.ack_wait.is_zero()
        {
            return Err(error("Queue limits and ACK deadline must be positive"));
        }
        let context = nats
            .jetstream()
            .ok_or_else(|| error("JetStream required"))?
            .clone();
        let stream = context
            .get_or_create_stream(stream::Config {
                name: format!("DASEIN_TASKS_{namespace}"),
                subjects: vec![format!("dasein.queue.{namespace}.*")],
                retention: stream::RetentionPolicy::WorkQueue,
                storage: stream::StorageType::File,
                discard: stream::DiscardPolicy::New,
                max_messages: config.max_pending,
                max_bytes: config.max_queue_bytes,
                duplicate_window: Duration::from_secs(24 * 60 * 60),
                ..Default::default()
            })
            .await
            .map_err(error)?;
        let actual = &stream.cached_info().config;
        if actual.max_bytes != config.max_queue_bytes
            || actual.max_messages != config.max_pending
            || actual.retention != stream::RetentionPolicy::WorkQueue
            || actual.storage != stream::StorageType::File
            || actual.discard != stream::DiscardPolicy::New
            || actual.subjects != vec![format!("dasein.queue.{namespace}.*")]
        {
            return Err(error(
                "Existing queue configuration differs; migrate it explicitly",
            ));
        }
        let bucket = format!("DASEIN_RESULTS_{namespace}");
        let ledger = match context.get_key_value(&bucket).await {
            Ok(store) => store,
            Err(_) => match context
                .create_key_value(kv::Config {
                    bucket: bucket.clone(),
                    history: 1,
                    max_bytes: config.max_result_bytes,
                    storage: stream::StorageType::File,
                    ..Default::default()
                })
                .await
            {
                Ok(store) => store,
                Err(create_error) => context
                    .get_key_value(&bucket)
                    .await
                    .map_err(|_| error(create_error))?,
            },
        };
        let ledger_config = ledger.status().await.map_err(error)?.info.config;
        if ledger_config.max_bytes != config.max_result_bytes
            || ledger_config.storage != stream::StorageType::File
            || !ledger_config.max_age.is_zero()
            || ledger_config.max_messages_per_subject != 1
        {
            return Err(error(
                "Existing result ledger configuration differs; migrate it explicitly",
            ));
        }
        Ok(Self {
            context,
            stream,
            ledger,
            namespace: namespace.into(),
            config,
        })
    }

    /// Publish confirmed work. Reusing an ID with different work is an error.
    /// A transport error is ambiguous: retry the same ID and payload.
    pub async fn publish(&self, task: Task) -> Result<(), BusError> {
        token(&task.supervisor)?;
        let task_key = key(&task.id)?;
        for dependency in &task.depends_on {
            key(dependency)?;
            if dependency == &task.id {
                return Err(error("A task cannot depend on itself"));
            }
        }
        let record = Record {
            task: task.clone(),
            outcome: TaskOutcome::Pending,
        };
        if self
            .ledger
            .create(
                &task_key,
                serde_json::to_vec(&record).map_err(error)?.into(),
            )
            .await
            .is_err()
        {
            let existing = self
                .record(&task.id)
                .await?
                .ok_or_else(|| error("Could not reserve task ID"))?;
            if existing.task.payload != task.payload
                || existing.task.supervisor != task.supervisor
                || existing.task.depends_on != task.depends_on
                || existing.task.priority != task.priority
            {
                return Err(error("Task ID is already assigned to different work"));
            }
            if existing.outcome != TaskOutcome::Pending {
                return Ok(());
            }
        }
        let mut headers = async_nats::HeaderMap::new();
        headers.insert("Nats-Msg-Id", task_key);
        self.context
            .publish_with_headers(
                format!("dasein.queue.{}.{}", self.namespace, task.supervisor),
                headers,
                serde_json::to_vec(&task).map_err(error)?.into(),
            )
            .await
            .map_err(error)?
            .await
            .map_err(error)?;
        Ok(())
    }

    async fn record(&self, id: &str) -> Result<Option<Record>, BusError> {
        self.ledger
            .get(key(id)?)
            .await
            .map_err(error)?
            .map(|data| serde_json::from_slice(&data).map_err(error))
            .transpose()
    }

    /// Read a persistent result. Pending may also mean a publication needs retry.
    pub async fn outcome(&self, id: &str) -> Result<Option<TaskOutcome>, BusError> {
        Ok(self.record(id).await?.map(|r| r.outcome))
    }

    async fn finish(&self, id: &str, outcome: TaskOutcome) -> Result<(), BusError> {
        let task_key = key(id)?;
        for _ in 0..32 {
            let entry = self
                .ledger
                .entry(&task_key)
                .await
                .map_err(error)?
                .ok_or_else(|| error("Task ledger entry missing"))?;
            let mut record: Record = serde_json::from_slice(&entry.value).map_err(error)?;
            if record.outcome != TaskOutcome::Pending {
                return Ok(());
            }
            record.outcome = outcome.clone();
            match self
                .ledger
                .update(
                    &task_key,
                    serde_json::to_vec(&record).map_err(error)?.into(),
                    entry.revision,
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == kv::UpdateErrorKind::WrongLastRevision => {
                    tokio::task::yield_now().await;
                }
                Err(e) => return Err(error(e)),
            }
        }
        Err(error("Task completion contention"))
    }

    /// Open a shared durable consumer for this supervisor. Competing workers
    /// must use the same queue namespace and supervisor ID.
    pub async fn worker(&self, supervisor: &str) -> Result<DurableWorker, BusError> {
        token(supervisor)?;
        let name = format!("worker_{supervisor}");
        let consumer = self
            .stream
            .get_or_create_consumer(
                &name,
                consumer::pull::Config {
                    durable_name: Some(name.clone()),
                    filter_subject: format!("dasein.queue.{}.{supervisor}", self.namespace),
                    ack_policy: consumer::AckPolicy::Explicit,
                    ack_wait: self.config.ack_wait,
                    max_ack_pending: self.config.max_in_flight,
                    max_deliver: -1,
                    ..Default::default()
                },
            )
            .await
            .map_err(error)?;
        if consumer.cached_info().config.ack_wait != self.config.ack_wait
            || consumer.cached_info().config.max_ack_pending != self.config.max_in_flight
        {
            return Err(error("Worker ACK deadline differs from existing consumer"));
        }
        Ok(DurableWorker {
            queue: self.clone(),
            consumer,
        })
    }
}

/// Pull worker with no local task buffer; cancellation leaves work on the server.
pub struct DurableWorker {
    queue: DurableQueue,
    consumer: Consumer<consumer::pull::Config>,
}
impl DurableWorker {
    /// Claim eligible work until the deadline. Dependencies gate delivery; failed
    /// dependencies permanently fail dependent tasks. Cycles remain pending.
    pub async fn next(&self, timeout: Duration) -> Result<Option<TaskDelivery>, BusError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let mut batch = self
                .consumer
                .fetch()
                .max_messages(1)
                .expires(remaining)
                .messages()
                .await
                .map_err(error)?;
            let Some(message) = batch.next().await else {
                return Ok(None);
            };
            let message = message.map_err(error)?;
            let task: Task = serde_json::from_slice(&message.payload).map_err(error)?;
            match self.queue.outcome(&task.id).await? {
                Some(TaskOutcome::Pending) => (),
                Some(_) => {
                    message.double_ack().await.map_err(error)?;
                    continue;
                }
                None => return Err(error("Task ledger missing; delivery left unacknowledged")),
            }
            let mut blocked = false;
            let mut failed = None;
            for dependency in &task.depends_on {
                match self.queue.outcome(dependency).await? {
                    Some(TaskOutcome::Completed(_)) => (),
                    Some(TaskOutcome::Failed(_)) => {
                        failed = Some(format!("Dependency {dependency} failed"));
                        break;
                    }
                    _ => blocked = true,
                }
            }
            if let Some(reason) = failed {
                self.queue
                    .finish(&task.id, TaskOutcome::Failed(reason))
                    .await?;
                message.double_ack().await.map_err(error)?;
                continue;
            }
            if blocked {
                message
                    .ack_with(jetstream::AckKind::Nak(Some(Duration::from_millis(250))))
                    .await
                    .map_err(error)?;
                continue;
            }
            return Ok(Some(TaskDelivery {
                task_id: task.id.clone(),
                task,
                message,
                queue: self.queue.clone(),
            }));
        }
    }
}

/// An unacknowledged delivery. Dropping it causes redelivery after `ack_wait`.
pub struct TaskDelivery {
    pub task: Task,
    task_id: String,
    message: jetstream::Message,
    queue: DurableQueue,
}
impl TaskDelivery {
    /// Persist completion before confirming ACK; duplicate deliveries skip work.
    pub async fn complete(self, result: Value) -> Result<(), BusError> {
        self.queue
            .finish(&self.task_id, TaskOutcome::Completed(result))
            .await?;
        self.message.double_ack().await.map_err(error)
    }
    /// Persist a terminal failure and remove the message from the work queue.
    pub async fn fail(self, reason: impl Into<String>) -> Result<(), BusError> {
        self.queue
            .finish(&self.task_id, TaskOutcome::Failed(reason.into()))
            .await?;
        self.message.double_ack().await.map_err(error)
    }
    /// Return work for a later attempt after a transient error.
    pub async fn retry(self, delay: Duration) -> Result<(), BusError> {
        self.message
            .ack_with(jetstream::AckKind::Nak(Some(delay)))
            .await
            .map_err(error)
    }
    /// Extend the server-side lease while work is still running.
    pub async fn heartbeat(&self) -> Result<(), BusError> {
        self.message
            .ack_with(jetstream::AckKind::Progress)
            .await
            .map_err(error)
    }
}
