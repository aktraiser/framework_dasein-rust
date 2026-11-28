//! Workflow shared state kept in NATS, for workflows whose steps run in more
//! than one process.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::consumer::{push, DeliverPolicy, ReplayPolicy};
use async_nats::jetstream::{kv, stream};
use async_trait::async_trait;
use futures::StreamExt;

use super::context::{split_namespace, state_namespace, SharedStateBackend};
use super::types::{ExecutorError, TaskId, WorkflowId};
use crate::distributed::bus::{BusError, NatsClient};

/// Shared state stored in a NATS JetStream key-value bucket.
///
/// Give the same bucket name to the workflow and to the
/// [`RemoteWorker`](super::RemoteWorker)s that run its remote steps: they
/// then read and write the same state, wherever they run.
///
/// The state of a run stays in the bucket until [`forget`](Self::forget) is
/// called for it. Values are written last-writer-wins. `restore` replaces a namespace key
/// by key, not atomically: do not run steps of a task while it is being
/// restored from a checkpoint.
#[derive(Clone)]
pub struct NatsStateBackend {
    store: kv::Store,
    /// How long one read or write goes on trying while NATS is away.
    patience: Duration,
}

fn failed(error: impl std::fmt::Display) -> ExecutorError {
    ExecutorError::new("shared-state", format!("NATS shared state: {error}"))
}

/// Lower-case hexadecimal of a text: bucket keys accept few characters.
fn hex(text: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(text.len() * 2);
    for byte in text.bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn unhex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect()
}

/// Bucket key of a state key: `n_{namespace}.k_{rest}`, both in hexadecimal.
///
/// The namespace of a workflow run is a token of its own so that NATS can
/// list the keys of one run without reading those of the others. Keys
/// outside any run namespace have an empty first token.
fn encode(key: &str) -> String {
    let (namespace, rest) = split_namespace(key).unwrap_or(("", key));
    format!("n_{}.k_{}", hex(namespace), hex(rest))
}

fn decode(encoded: &str) -> Option<String> {
    let (namespace, rest) = encoded.split_once('.')?;
    let mut bytes = unhex(namespace.strip_prefix("n_")?)?;
    bytes.extend(unhex(rest.strip_prefix("k_")?)?);
    String::from_utf8(bytes).ok()
}

/// How long a listing waits for its next entry before giving up.
const LISTING_STALL: Duration = Duration::from_secs(30);

impl NatsStateBackend {
    /// Open the bucket `DASEIN_STATE_{name}`, creating it if needed. `name`
    /// is 1 to 64 ASCII letters, digits, underscores or hyphens.
    ///
    /// # Errors
    /// Fails if the name is invalid, JetStream is not enabled, or the bucket
    /// cannot be opened.
    pub async fn open(nats: Arc<NatsClient>, name: &str) -> Result<Self, BusError> {
        let error = |e: &dyn std::fmt::Display| BusError::PublishFailed(e.to_string());
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(error(
                &"State bucket names must be 1..64 ASCII letters, digits, underscores or hyphens",
            ));
        }
        let context = nats
            .jetstream()
            .ok_or_else(|| error(&"JetStream required"))?
            .clone();
        let bucket = format!("DASEIN_STATE_{name}");
        let store = match context.get_key_value(&bucket).await {
            Ok(store) => store,
            Err(_) => match context
                .create_key_value(kv::Config {
                    bucket: bucket.clone(),
                    history: 1,
                    storage: stream::StorageType::File,
                    ..Default::default()
                })
                .await
            {
                Ok(store) => store,
                // Another process created it at the same moment.
                Err(create_error) => context
                    .get_key_value(&bucket)
                    .await
                    .map_err(|_| error(&create_error))?,
            },
        };
        Ok(Self {
            store,
            patience: Duration::from_secs(30),
        })
    }

    /// How long a read or a write of one key goes on trying while NATS
    /// cannot be reached, before it fails: 30 seconds unless set here.
    ///
    /// A step that reads or writes shared state then rides out a server
    /// restart or a short network cut instead of failing and using up its
    /// retries. Keep it under the time the step is given
    /// (`executor_timeout_ms`), which still applies.
    ///
    /// A write that is tried again may land after a newer write of the same
    /// key by another step: values are last-writer-wins, and waiting makes
    /// "last" span the outage. Do not let two steps write one key if the
    /// order matters.
    #[must_use]
    pub const fn with_patience(mut self, patience: Duration) -> Self {
        self.patience = patience;
        self
    }

    /// Remove the shared state of one workflow run. State is otherwise kept
    /// for as long as the bucket exists: call this when a run is finished and
    /// will not be resumed.
    ///
    /// The entries are removed from the bucket in one request, without
    /// leaving deletion markers behind.
    ///
    /// # Errors
    /// Fails if the state cannot be removed.
    pub async fn forget(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
    ) -> Result<(), ExecutorError> {
        let namespace = state_namespace(workflow_id, task_id);
        let (namespace, _) = split_namespace(&namespace)
            .ok_or_else(|| failed("not the state namespace of a run"))?;
        let subject = format!("{}n_{}.>", self.store.prefix, hex(namespace));
        let patience = crate::distributed::bus::patience::Patience::new(self.patience);
        loop {
            match self.store.stream.purge().filter(&subject).await {
                Ok(_) => return Ok(()),
                Err(error) => match patience.pause_for(&error) {
                    Some(pause) => tokio::time::sleep(pause).await,
                    None => return Err(failed(error)),
                },
            }
        }
    }

    /// Live entries whose key starts with `prefix`.
    ///
    /// When `prefix` is the namespace of a workflow run, only that run's
    /// entries are read from the server; otherwise the whole bucket is.
    async fn entries(
        &self,
        prefix: &str,
    ) -> Result<HashMap<String, serde_json::Value>, ExecutorError> {
        // A listing reads; it can be started again while the server is away.
        let patience = crate::distributed::bus::patience::Patience::new(self.patience);
        loop {
            match self.list(prefix).await {
                Ok(values) => return Ok(values),
                Err(error) => match patience.pause_for(&error) {
                    Some(pause) => tokio::time::sleep(pause).await,
                    None => return Err(error),
                },
            }
        }
    }

    async fn list(
        &self,
        prefix: &str,
    ) -> Result<HashMap<String, serde_json::Value>, ExecutorError> {
        let filter = match split_namespace(prefix) {
            Some((namespace, "")) => format!("n_{}.>", hex(namespace)),
            _ => ">".to_owned(),
        };
        let consumer = self
            .store
            .stream
            .create_consumer(push::OrderedConfig {
                deliver_subject: format!("_INBOX.{}", uuid::Uuid::new_v4().simple()),
                description: Some("dasein shared state listing".to_owned()),
                filter_subject: format!("{}{filter}", self.store.prefix),
                replay_policy: ReplayPolicy::Instant,
                // The current value of each key, not its history.
                deliver_policy: DeliverPolicy::LastPerSubject,
                ..Default::default()
            })
            .await
            .map_err(failed)?;
        // The server would keep the consumer for a while after this call:
        // remove it, so that many listings in a row do not pile them up.
        let name = consumer.cached_info().name.clone();
        let values = Self::read_listing(&self.store.prefix, prefix, consumer).await;
        if let Err(error) = self.store.stream.delete_consumer(&name).await {
            tracing::debug!(%error, "Could not remove a state listing consumer");
        }
        values
    }

    /// Read what a listing consumer delivers, up to the last entry it had
    /// when it was created.
    async fn read_listing(
        bucket_prefix: &str,
        prefix: &str,
        consumer: async_nats::jetstream::consumer::Consumer<push::OrderedConfig>,
    ) -> Result<HashMap<String, serde_json::Value>, ExecutorError> {
        let mut values = HashMap::new();
        if consumer.cached_info().num_pending == 0 {
            return Ok(values);
        }
        let mut messages = consumer.messages().await.map_err(failed)?;
        loop {
            let message = tokio::time::timeout(LISTING_STALL, messages.next())
                .await
                .map_err(|_| failed("listing stalled"))?
                .ok_or_else(|| failed("listing ended early"))?
                .map_err(failed)?;
            let left = message.info().map_err(failed)?.pending;
            // Deletions stay in the bucket as entries carrying a marker.
            let deleted = message.headers.as_ref().is_some_and(|headers| {
                headers.get("KV-Operation").is_some() || headers.get("Nats-Marker-Reason").is_some()
            });
            let key = message.subject.strip_prefix(bucket_prefix).and_then(decode);
            if let (false, Some(key)) = (deleted, key) {
                if key.starts_with(prefix) {
                    values.insert(
                        key,
                        serde_json::from_slice(&message.payload).map_err(failed)?,
                    );
                }
            }
            if left == 0 {
                return Ok(values);
            }
        }
    }
}

#[async_trait]
impl SharedStateBackend for NatsStateBackend {
    async fn set(&self, key: &str, value: serde_json::Value) -> Result<(), ExecutorError> {
        let bytes = serde_json::to_vec(&value).map_err(failed)?;
        let key = encode(key);
        // Writing the same value again is harmless: it can be repeated
        // until the server is back.
        let patience = crate::distributed::bus::patience::Patience::new(self.patience);
        loop {
            match self.store.put(&key, bytes.clone().into()).await {
                Ok(_) => return Ok(()),
                Err(error) => match patience.pause_for(&error) {
                    Some(pause) => tokio::time::sleep(pause).await,
                    None => return Err(failed(error)),
                },
            }
        }
    }

    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>, ExecutorError> {
        let key = encode(key);
        let patience = crate::distributed::bus::patience::Patience::new(self.patience);
        let found = loop {
            match self.store.get(&key).await {
                Ok(found) => break found,
                Err(error) => match patience.pause_for(&error) {
                    Some(pause) => tokio::time::sleep(pause).await,
                    None => return Err(failed(error)),
                },
            }
        };
        found
            .map(|bytes| serde_json::from_slice(&bytes).map_err(failed))
            .transpose()
    }

    async fn snapshot(
        &self,
        prefix: &str,
    ) -> Result<HashMap<String, serde_json::Value>, ExecutorError> {
        self.entries(prefix).await
    }

    async fn restore(
        &self,
        prefix: &str,
        values: &HashMap<String, serde_json::Value>,
    ) -> Result<(), ExecutorError> {
        for key in self.entries(prefix).await?.into_keys() {
            if !values.contains_key(&key) {
                self.delete(&key).await?;
            }
        }
        for (key, value) in values {
            self.set(key, value.clone()).await?;
        }
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ExecutorError> {
        let key = encode(key);
        let patience = crate::distributed::bus::patience::Patience::new(self.patience);
        loop {
            match self.store.delete(&key).await {
                Ok(()) => return Ok(()),
                Err(error) => match patience.pause_for(&error) {
                    Some(pause) => tokio::time::sleep(pause).await,
                    None => return Err(failed(error)),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_through_the_bucket_encoding() {
        for key in [
            "",
            "8:workflow4:task:group_chat",
            "8:workflow4:task:",
            "clé à accents/ * > .",
            "a\u{1f}b",
        ] {
            let encoded = encode(key);
            assert!(
                encoded
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.'),
                "{encoded}"
            );
            assert!(!encoded.starts_with('.') && !encoded.ends_with('.'));
            assert_eq!(decode(&encoded).as_deref(), Some(key));
        }
        assert_eq!(decode("not-ours"), None);
        assert_eq!(decode("n_.k_zz"), None);
    }

    #[test]
    fn keys_of_one_run_share_a_first_token() {
        let namespace = state_namespace(&WorkflowId::new("wf:1"), &TaskId::new("run 7"));
        let first = encode(&format!("{namespace}a"));
        let second = encode(&format!("{namespace}b:c"));
        let other = encode(&format!(
            "{}a",
            state_namespace(&WorkflowId::new("wf:1"), &TaskId::new("run 8"))
        ));
        let token = |key: &str| key.split_once('.').unwrap().0.to_owned();
        assert_eq!(token(&first), token(&second));
        assert_eq!(token(&first), format!("n_{}", hex(&namespace)));
        assert_ne!(token(&first), token(&other));
        // A key outside any run has the empty namespace.
        assert_eq!(token(&encode("loose")), "n_");
    }
}
