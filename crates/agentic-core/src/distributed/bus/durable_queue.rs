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
    /// How long the record of a task, with its result, is kept, counted
    /// from its last change (submission, then completion). After that the
    /// task is forgotten: submitted again, it runs again. A task that waits
    /// and runs for longer still completes, but may be accepted a second
    /// time meanwhile: keep this above the time your tasks take.
    ///
    /// The ledger must hold every task of that period within
    /// `max_result_bytes`. When it is full, tasks can neither be submitted
    /// nor completed until records expire.
    pub result_ttl: Duration,
}
impl Default for DurableQueueConfig {
    fn default() -> Self {
        Self {
            max_pending: 1000,
            max_queue_bytes: 512 * 1024 * 1024,
            max_result_bytes: 512 * 1024 * 1024,
            ack_wait: Duration::from_secs(30),
            max_in_flight: 128,
            result_ttl: Duration::from_secs(60 * 60),
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

/// A namespaced work queue. The record of a task, with its outcome, stays
/// in the ledger for `result_ttl`.
#[derive(Clone)]
pub struct DurableQueue {
    context: jetstream::Context,
    stream: stream::Stream,
    ledger: kv::Store,
    namespace: String,
    config: DurableQueueConfig,
    hub: Arc<Hub>,
}

/// How long a worker goes on trying to record a result while the server is
/// away.
const REPORT_PATIENCE: Duration = Duration::from_secs(30);

/// How often a wait reads the record itself, whatever the subscription says.
const RECHECK: Duration = Duration::from_secs(2);

type Waiting = std::collections::HashMap<String, Vec<tokio::sync::oneshot::Sender<TaskOutcome>>>;

/// Where the waits of a queue meet the results the server pushes.
struct Hub {
    /// Waits by ledger key.
    waiting: std::sync::Mutex<Waiting>,
    /// The subscription task, once started.
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Whether the subscription is currently established.
    watching: tokio::sync::watch::Sender<bool>,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            waiting: std::sync::Mutex::default(),
            task: std::sync::Mutex::default(),
            watching: tokio::sync::watch::Sender::new(false),
        }
    }
}

impl Hub {
    fn waiting(&self) -> std::sync::MutexGuard<'_, Waiting> {
        self.waiting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Hand a recorded result to the waits for its task, if any.
    fn deliver(&self, entry: &kv::Entry) {
        if entry.operation != kv::Operation::Put || !self.waiting().contains_key(&entry.key) {
            return;
        }
        let Ok(record) = serde_json::from_slice::<Record>(&entry.value) else {
            return;
        };
        if record.outcome == TaskOutcome::Pending {
            return;
        }
        if let Some(waits) = self.waiting().remove(&entry.key) {
            for wait in waits {
                let _ = wait.send(record.outcome.clone());
            }
        }
    }
}

/// Removes the ended waits of a task from the hub.
struct Unregister<'a> {
    hub: &'a Hub,
    key: &'a str,
}

impl Drop for Unregister<'_> {
    fn drop(&mut self) {
        let mut waiting = self.hub.waiting();
        if let Some(waits) = waiting.get_mut(self.key) {
            waits.retain(|wait| !wait.is_closed());
            if waits.is_empty() {
                waiting.remove(self.key);
            }
        }
    }
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
            || config.result_ttl.is_zero()
        {
            return Err(error(
                "Queue limits, ACK deadline and result lifetime must be positive",
            ));
        }
        let context = nats
            .jetstream()
            .ok_or_else(|| error("JetStream required"))?
            .clone();
        // A task ID is refused as a duplicate for as long as its record is
        // kept, not longer: a forgotten task must be able to run again.
        let duplicate_window = config.result_ttl.min(Duration::from_secs(24 * 60 * 60));
        let stream = context
            .get_or_create_stream(stream::Config {
                name: format!("DASEIN_TASKS_{namespace}"),
                subjects: vec![format!("dasein.queue.{namespace}.*")],
                retention: stream::RetentionPolicy::WorkQueue,
                storage: stream::StorageType::File,
                discard: stream::DiscardPolicy::New,
                max_messages: config.max_pending,
                max_bytes: config.max_queue_bytes,
                duplicate_window,
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
        let ledger_settings = kv::Config {
            bucket: bucket.clone(),
            history: 1,
            max_bytes: config.max_result_bytes,
            max_age: config.result_ttl,
            storage: stream::StorageType::File,
            ..Default::default()
        };
        let ledger = match context.get_key_value(&bucket).await {
            Ok(store) => store,
            Err(_) => match context.create_key_value(ledger_settings.clone()).await {
                Ok(store) => store,
                Err(create_error) => context
                    .get_key_value(&bucket)
                    .await
                    .map_err(|_| error(create_error))?,
            },
        };
        let mut ledger_config = ledger.status().await.map_err(error)?.info.config;
        // A ledger made by an earlier version keeps its records for ever,
        // and fills up: give it the lifetime asked for.
        // Only the lifetime changes: whatever else was set on the bucket,
        // such as its replicas, stays.
        let ledger = if ledger_config.max_age.is_zero()
            && ledger_config.max_bytes == config.max_result_bytes
            && ledger_config.storage == stream::StorageType::File
            && ledger_config.max_messages_per_subject == 1
        {
            let mut wanted = ledger_config.clone();
            wanted.max_age = config.result_ttl;
            // The server wants this window no longer than the lifetime.
            wanted.duplicate_window = wanted.duplicate_window.min(config.result_ttl);
            context.update_stream(wanted).await.map_err(error)?;
            let ledger = context.get_key_value(&bucket).await.map_err(error)?;
            ledger_config = ledger.status().await.map_err(error)?.info.config;
            ledger
        } else {
            ledger
        };
        if ledger_config.max_bytes != config.max_result_bytes
            || ledger_config.storage != stream::StorageType::File
            || ledger_config.max_age != config.result_ttl
            || ledger_config.max_messages_per_subject != 1
        {
            return Err(error(
                "Existing result ledger configuration differs; migrate it explicitly",
            ));
        }
        // Last, once this process is known to agree with the queue: a queue
        // made by an earlier version refuses a known task ID for a day,
        // longer than its record is now kept.
        let stream = if stream.cached_info().config.duplicate_window == duplicate_window {
            stream
        } else {
            let mut wanted = stream.cached_info().config.clone();
            wanted.duplicate_window = duplicate_window;
            context.update_stream(wanted).await.map_err(error)?;
            context
                .get_stream(format!("DASEIN_TASKS_{namespace}"))
                .await
                .map_err(error)?
        };
        Ok(Self {
            context,
            stream,
            ledger,
            namespace: namespace.into(),
            config,
            hub: Arc::default(),
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
        if let Err(refusal) = self
            .ledger
            .create(
                &task_key,
                serde_json::to_vec(&record).map_err(error)?.into(),
            )
            .await
        {
            // Refused and not there: the ledger itself said no, as it does
            // when it is full.
            let existing = self.record(&task.id).await?.ok_or_else(|| {
                error(format!(
                    "Could not record the task ({refusal}); a full result ledger takes new \
                     tasks again as records expire"
                ))
            })?;
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

    /// Wait up to `timeout` for a task to finish, and return its state then:
    /// its result or failure, or `Pending` if it is still not done.
    ///
    /// Results are pushed by the server as they are recorded, over one
    /// subscription shared by every wait of this queue and its clones. The
    /// record is also read again every few seconds, so that a result still
    /// arrives if that subscription is being re-established.
    pub async fn wait_outcome(
        &self,
        id: &str,
        timeout: Duration,
    ) -> Result<Option<TaskOutcome>, BusError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let task_key = key(id)?;
        self.watch_results();
        // Dropped after the receiver: forgets this wait in the hub.
        let _forget = Unregister {
            hub: &self.hub,
            key: &task_key,
        };
        let (found, mut pushed) = tokio::sync::oneshot::channel();
        self.hub
            .waiting()
            .entry(task_key.clone())
            .or_default()
            .push(found);
        if !*self.hub.watching.borrow() {
            // No subscription yet, or it is being re-established. A task
            // that is already done needs none.
            if let Ok(outcome) = self.outcome(id).await {
                if !matches!(outcome, Some(TaskOutcome::Pending) | None) {
                    return Ok(outcome);
                }
            }
            // A result recorded before the subscription exists would not
            // be pushed: give it a moment to start.
            let mut watching = self.hub.watching.subscribe();
            let _ = tokio::time::timeout(
                // Never longer than the caller is ready to wait at all.
                Duration::from_secs(1).min(timeout),
                watching.wait_for(|watching| *watching),
            )
            .await;
        }
        let mut pushed_open = true;
        loop {
            // A record that cannot be read (the server is restarting, the
            // network is down) is not a reason to give up before the time
            // is over: the result may well be there at the next read.
            let read = self.outcome(id).await;
            if let Ok(outcome) = &read {
                if !matches!(outcome, Some(TaskOutcome::Pending) | None) {
                    return read;
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return read;
            }
            let recheck = tokio::time::sleep_until(deadline.min(now + RECHECK));
            tokio::select! {
                outcome = &mut pushed, if pushed_open => match outcome {
                    Ok(outcome) => return Ok(Some(outcome)),
                    Err(_) => pushed_open = false,
                },
                () = recheck => {}
            }
        }
    }

    /// Start the task that receives recorded results and hands them to the
    /// waits, unless it is running. It ends when the queue and all its clones
    /// are dropped, or with the runtime it was started on: a later wait then
    /// starts it again.
    fn watch_results(&self) {
        let mut task = self
            .hub
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if task.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        self.hub.watching.send_replace(false);
        let hub = Arc::downgrade(&self.hub);
        let ledger = self.ledger.clone();
        *task = Some(tokio::spawn(async move {
            let mut pause = Duration::from_millis(250);
            loop {
                match ledger.watch_all().await {
                    Ok(mut changes) => {
                        pause = Duration::from_millis(250);
                        let Some(alive) = hub.upgrade() else { return };
                        alive.watching.send_replace(true);
                        drop(alive);
                        loop {
                            // Looking up now and then whether anyone is left.
                            let next =
                                tokio::time::timeout(Duration::from_secs(5), changes.next()).await;
                            let Some(hub) = hub.upgrade() else { return };
                            match next {
                                Err(_) => {}
                                Ok(Some(Ok(entry))) => hub.deliver(&entry),
                                Ok(Some(Err(error))) => {
                                    tracing::warn!(%error, "Result subscription failed");
                                    hub.watching.send_replace(false);
                                    break;
                                }
                                Ok(None) => {
                                    hub.watching.send_replace(false);
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, retry_in = ?pause, "Could not subscribe to results");
                    }
                }
                if hub.strong_count() == 0 {
                    return;
                }
                tokio::time::sleep(pause).await;
                pause = (pause * 2).min(Duration::from_secs(30));
            }
        }));
    }

    async fn finish(&self, id: &str, task: &Task, outcome: TaskOutcome) -> Result<(), BusError> {
        let task_key = key(id)?;
        for _ in 0..32 {
            let Some(entry) = self.ledger.entry(&task_key).await.map_err(error)? else {
                // The record expired while the task waited or ran, longer
                // than `result_ttl`: write it anew with the outcome, so the
                // work is not lost. Someone else doing so first is fine.
                let record = Record {
                    task: task.clone(),
                    outcome: outcome.clone(),
                };
                if self
                    .ledger
                    .create(
                        &task_key,
                        serde_json::to_vec(&record).map_err(error)?.into(),
                    )
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                tokio::task::yield_now().await;
                continue;
            };
            // A deleted record reads as an entry without a value.
            if entry.operation != kv::Operation::Put {
                self.ledger
                    .put(
                        &task_key,
                        serde_json::to_vec(&Record {
                            task: task.clone(),
                            outcome: outcome.clone(),
                        })
                        .map_err(error)?
                        .into(),
                    )
                    .await
                    .map_err(error)?;
                return Ok(());
            }
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
                    .finish(&task.id, &task, TaskOutcome::Failed(reason))
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
    ///
    /// If the server is away, this goes on trying for 30 seconds:
    /// work that is done should not be done again because its result could
    /// not be written at that moment. A task whose acknowledgement deadline
    /// passes meanwhile may still be delivered again; it is skipped if this
    /// record lands first: keep `ack_wait` above the outages you want
    /// finished work to survive without being done twice.
    pub async fn complete(self, result: Value) -> Result<(), BusError> {
        self.report(TaskOutcome::Completed(result)).await
    }
    /// Persist a terminal failure and remove the message from the work queue.
    /// Tried again like [`complete`](Self::complete) while the server is away.
    pub async fn fail(self, reason: impl Into<String>) -> Result<(), BusError> {
        self.report(TaskOutcome::Failed(reason.into())).await
    }
    async fn report(self, outcome: TaskOutcome) -> Result<(), BusError> {
        let patience = super::patience::Patience::new(REPORT_PATIENCE);
        // Both steps can be repeated: a record already final is left alone,
        // and so is a message already acknowledged.
        let mut tried = false;
        loop {
            if tried {
                // The server may be back: say first that the task is still
                // ours, so that it is not handed to another worker while its
                // result is being written. If its deadline passed during the
                // outage it may have been handed out already; delivery is at
                // least once, as the module says.
                let _ = self.message.ack_with(jetstream::AckKind::Progress).await;
            }
            tried = true;
            match self
                .queue
                .finish(&self.task_id, &self.task, outcome.clone())
                .await
            {
                Ok(()) => break,
                Err(e) => match patience.pause_for(&e) {
                    Some(pause) => tokio::time::sleep(pause).await,
                    None => return Err(e),
                },
            }
        }
        loop {
            match self.message.double_ack().await {
                Ok(()) => return Ok(()),
                Err(e) => match patience.pause_for(&e) {
                    Some(pause) => tokio::time::sleep(pause).await,
                    None => return Err(error(e)),
                },
            }
        }
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
