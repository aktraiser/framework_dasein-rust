//! Redis state store implementation.

use async_trait::async_trait;
use redis::{aio::ConnectionManager, AsyncCommands, Client};
use serde::{de::DeserializeOwned, Serialize};
use tracing::{debug, info};

use crate::{error::StorageError, traits::StateStore};

/// Redis-based state store.
pub struct RedisStore {
    conn: ConnectionManager,
}

impl RedisStore {
    /// Connect to Redis.
    ///
    /// # Errors
    ///
    /// Returns an error if connection fails.
    pub async fn connect(url: &str) -> Result<Self, StorageError> {
        info!(url = %url, "Connecting to Redis");

        let client =
            Client::open(url).map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        info!("Connected to Redis");

        Ok(Self { conn })
    }
}

#[async_trait]
impl StateStore for RedisStore {
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>, StorageError> {
        let mut conn = self.conn.clone();

        let value: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| StorageError::GetFailed(e.to_string()))?;

        match value {
            Some(v) => {
                let parsed = serde_json::from_str(&v)
                    .map_err(|e| StorageError::DeserializationError(e.to_string()))?;
                Ok(Some(parsed))
            }
            None => Ok(None),
        }
    }

    async fn set<T: Serialize + Send + Sync>(
        &self,
        key: &str,
        value: &T,
        ttl_ms: Option<u64>,
    ) -> Result<(), StorageError> {
        let mut conn = self.conn.clone();

        let serialized = serde_json::to_string(value)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        debug!(key = %key, ttl_ms = ?ttl_ms, "Setting key");

        if let Some(ttl) = ttl_ms {
            conn.set_ex(key, &serialized, ttl / 1000)
                .await
                .map_err(|e| StorageError::SetFailed(e.to_string()))
        } else {
            conn.set(key, &serialized)
                .await
                .map_err(|e| StorageError::SetFailed(e.to_string()))
        }
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let mut conn = self.conn.clone();

        conn.del(key)
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let mut conn = self.conn.clone();

        conn.exists(key)
            .await
            .map_err(|e| StorageError::QueryFailed(e.to_string()))
    }
}

/// Standard key patterns for Redis.
pub struct KeyPatterns;

impl KeyPatterns {
    /// Agent state key.
    #[must_use]
    pub fn agent_state(agent_id: &str) -> String {
        format!("agents:{agent_id}:state")
    }

    /// Agent metrics key.
    #[must_use]
    pub fn agent_metrics(agent_id: &str) -> String {
        format!("agents:{agent_id}:metrics")
    }

    /// Task state key.
    #[must_use]
    pub fn task_state(task_id: &str) -> String {
        format!("tasks:{task_id}:state")
    }

    /// Session key.
    #[must_use]
    pub fn session(session_id: &str) -> String {
        format!("sessions:{session_id}")
    }

    /// Cache key.
    #[must_use]
    pub fn cache(key: &str) -> String {
        format!("cache:{key}")
    }
}
