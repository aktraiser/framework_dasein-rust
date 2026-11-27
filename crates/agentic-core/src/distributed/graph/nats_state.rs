//! Workflow shared state kept in NATS, for workflows whose steps run in more
//! than one process.

use std::collections::HashMap;
use std::sync::Arc;

use async_nats::jetstream::{kv, stream};
use async_trait::async_trait;
use futures::StreamExt;

use super::context::SharedStateBackend;
use super::types::ExecutorError;
use crate::distributed::bus::{BusError, NatsClient};

/// Shared state stored in a NATS JetStream key-value bucket.
///
/// Give the same bucket name to the workflow and to the
/// [`RemoteWorker`](super::RemoteWorker)s that run its remote steps: they
/// then read and write the same state, wherever they run.
///
/// Values are written last-writer-wins. `restore` replaces a namespace key
/// by key, not atomically: do not run steps of a task while it is being
/// restored from a checkpoint.
#[derive(Clone)]
pub struct NatsStateBackend {
    store: kv::Store,
}

fn failed(error: impl std::fmt::Display) -> ExecutorError {
    ExecutorError::new("shared-state", format!("NATS shared state: {error}"))
}

/// Keys may contain any character; bucket keys may not. Hex keeps the
/// mapping exact and reversible.
fn encode(key: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(2 + key.len() * 2);
    encoded.push_str("k_");
    for byte in key.bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn decode(encoded: &str) -> Option<String> {
    let hex = encoded.strip_prefix("k_")?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

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
        Ok(Self { store })
    }

    /// Every live key of the bucket, decoded.
    async fn keys(&self) -> Result<Vec<String>, ExecutorError> {
        let mut encoded = self.store.keys().await.map_err(failed)?;
        let mut keys = Vec::new();
        while let Some(key) = encoded.next().await {
            if let Some(key) = decode(&key.map_err(failed)?) {
                keys.push(key);
            }
        }
        Ok(keys)
    }
}

#[async_trait]
impl SharedStateBackend for NatsStateBackend {
    async fn set(&self, key: &str, value: serde_json::Value) -> Result<(), ExecutorError> {
        let bytes = serde_json::to_vec(&value).map_err(failed)?;
        self.store
            .put(encode(key), bytes.into())
            .await
            .map_err(failed)?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>, ExecutorError> {
        self.store
            .get(encode(key))
            .await
            .map_err(failed)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(failed))
            .transpose()
    }

    async fn snapshot(
        &self,
        prefix: &str,
    ) -> Result<HashMap<String, serde_json::Value>, ExecutorError> {
        let mut values = HashMap::new();
        for key in self.keys().await? {
            if key.starts_with(prefix) {
                // A key deleted since it was listed is simply absent.
                if let Some(value) = self.get(&key).await? {
                    values.insert(key, value);
                }
            }
        }
        Ok(values)
    }

    async fn restore(
        &self,
        prefix: &str,
        values: &HashMap<String, serde_json::Value>,
    ) -> Result<(), ExecutorError> {
        for key in self.keys().await? {
            if key.starts_with(prefix) && !values.contains_key(&key) {
                self.delete(&key).await?;
            }
        }
        for (key, value) in values {
            self.set(key, value.clone()).await?;
        }
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ExecutorError> {
        self.store.delete(encode(key)).await.map_err(failed)
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
            "clé à accents/ * > .",
            "a\u{1f}b",
        ] {
            let encoded = encode(key);
            assert!(
                encoded
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "{encoded}"
            );
            assert_eq!(decode(&encoded).as_deref(), Some(key));
        }
        assert_eq!(decode("not-ours"), None);
        assert_eq!(decode("k_zz"), None);
    }
}
