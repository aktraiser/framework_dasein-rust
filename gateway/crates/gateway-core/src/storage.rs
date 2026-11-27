//! `SQLite` storage for Gateway admin dashboard
//!
//! Provides persistence for:
//! - API keys for client authentication
//! - Provider configurations
//! - Usage tracking and logging
//! - Sandbox configurations

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    FromRow, SqlitePool,
};
use thiserror::Error;

/// Storage error types
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Record not found: {0}")]
    NotFound(String),
    #[error("Hash error: {0}")]
    Hash(String),
    #[error("Invalid API key")]
    InvalidApiKey,
}

pub type StorageResult<T> = Result<T, StorageError>;

/// What became of a request to execute in a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxClaim {
    /// The caller may execute, and must release the claim afterwards.
    Claimed,
    /// The box is already executing.
    Busy,
    /// As many boxes as allowed are executing.
    Full,
    /// No such box for this owner.
    Missing,
}

/// API Key record
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
}

/// Provider configuration record
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub api_key_encrypted: Option<String>,
    pub api_base: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// Usage log record
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UsageLog {
    pub id: i64,
    pub api_key_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cost_usd: f64,
    pub duration_ms: i32,
    pub created_at: DateTime<Utc>,
}

/// Sandbox configuration record
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SandboxConfig {
    pub runtime: String,
    pub enabled: bool,
    pub min_ready: i32,
    pub max_total: i32,
    pub memory_mb: i32,
    pub timeout_seconds: i32,
}

/// Admin session record
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AdminSession {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Usage statistics for a time period
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageStats {
    pub total_requests: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
    pub requests_by_provider: Vec<(String, i64)>,
    pub requests_by_model: Vec<(String, i64)>,
}

/// Storage handle for `SQLite` operations
#[derive(Clone)]
pub struct Storage {
    pool: SqlitePool,
    credential_cipher: Option<aes_gcm::Aes256Gcm>,
}

impl Storage {
    /// Create a new storage instance with the given database URL
    ///
    /// # Errors
    /// Returns an error if the database connection or schema migrations fail.
    pub async fn new(database_url: &str) -> StorageResult<Self> {
        // Gateway processes started together meet here: switching a new
        // file to WAL and creating the schema need the database to
        // themselves for a moment, and the loser is told "locked" at once.
        let mut pause = std::time::Duration::from_millis(50);
        let mut attempts = 0;
        loop {
            attempts += 1;
            match Self::open(database_url).await {
                Err(StorageError::Database(sqlx::Error::Database(error)))
                    if attempts < 20
                        && matches!(error.code().as_deref(), Some("5" | "6" | "261" | "517")) =>
                {
                    tracing::debug!(%error, attempts, "Database busy at start, trying again");
                    tokio::time::sleep(pause).await;
                    pause = (pause * 2).min(std::time::Duration::from_secs(1));
                }
                opened => return opened,
            }
        }
    }

    async fn open(database_url: &str) -> StorageResult<Self> {
        // Several gateway processes may share the file: readers do not
        // block the writer in WAL mode, and a writer waits for another
        // instead of failing at once.
        let options: SqliteConnectOptions = database_url.parse()?;
        let options = options
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        let storage = Self {
            pool,
            credential_cipher: None,
        };
        storage.run_migrations().await?;
        Ok(storage)
    }

    /// Create an in-memory storage for testing
    ///
    /// # Errors
    /// Returns an error if the database connection or schema migrations fail.
    pub async fn in_memory() -> StorageResult<Self> {
        Self::new("sqlite::memory:").await
    }

    /// Supply a master key from a secret manager instead of an environment variable.
    #[must_use]
    pub fn with_master_key(mut self, key: &[u8; 32]) -> Self {
        use aes_gcm::KeyInit;
        self.credential_cipher = Some(aes_gcm::Aes256Gcm::new(key.into()));
        self
    }

    /// Decrypt a stored provider credential using this storage instance's key.
    ///
    /// # Errors
    /// Returns an error if the database operation fails, the record is missing, or credential encryption/decryption cannot be completed.
    pub fn decrypt_provider_key(&self, provider: &Provider) -> StorageResult<Option<String>> {
        decrypt_provider_key_with_cipher(provider, self.credential_cipher.as_ref())
    }

    /// Run database migrations
    #[allow(clippy::too_many_lines)] // Keep the initial schema migration together.
    async fn run_migrations(&self) -> StorageResult<()> {
        sqlx::query("CREATE TABLE IF NOT EXISTS docker_boxes (id TEXT PRIMARY KEY, owner TEXT NOT NULL, policy TEXT NOT NULL)").execute(&self.pool).await?;
        // Which box is executing, in which gateway process, until when.
        sqlx::query("CREATE TABLE IF NOT EXISTS box_runs (box_id TEXT PRIMARY KEY, run_id TEXT NOT NULL, expires_at INTEGER NOT NULL)").execute(&self.pool).await?;
        // Failed dashboard logins per client; `*` counts everyone.
        sqlx::query("CREATE TABLE IF NOT EXISTS login_failures (client TEXT PRIMARY KEY, failures INTEGER NOT NULL, window_start INTEGER NOT NULL)").execute(&self.pool).await?;
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS api_keys (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                key_hash TEXT NOT NULL,
                key_prefix TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                last_used_at TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                rate_limit_rpm INTEGER,
                rate_limit_tpm INTEGER
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                api_key_encrypted TEXT,
                api_base TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS usage_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                api_key_id TEXT,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cost_usd REAL NOT NULL,
                duration_ms INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS sandbox_configs (
                runtime TEXT PRIMARY KEY,
                enabled INTEGER NOT NULL DEFAULT 1,
                min_ready INTEGER NOT NULL DEFAULT 2,
                max_total INTEGER NOT NULL DEFAULT 10,
                memory_mb INTEGER NOT NULL DEFAULT 512,
                timeout_seconds INTEGER NOT NULL DEFAULT 300
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS admin_sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at TEXT NOT NULL
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        // Per-key usage in each clock minute, for rate limits that survive a
        // restart and are shared by every process using this database.
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS key_rate_windows (
                api_key_id TEXT NOT NULL,
                minute INTEGER NOT NULL,
                requests INTEGER NOT NULL DEFAULT 0,
                tokens INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (api_key_id, minute)
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        // Create indexes for better query performance
        sqlx::query(
            r"
            CREATE INDEX IF NOT EXISTS idx_usage_logs_created_at ON usage_logs(created_at)
            ",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r"
            CREATE INDEX IF NOT EXISTS idx_usage_logs_api_key_id ON usage_logs(api_key_id)
            ",
        )
        .execute(&self.pool)
        .await?;

        // Insert default sandbox configs if not present
        for runtime in &["python", "node", "typescript", "rust", "go", "bash"] {
            sqlx::query(
                r"
                INSERT OR IGNORE INTO sandbox_configs (runtime, enabled, min_ready, max_total, memory_mb, timeout_seconds)
                VALUES (?, 1, 2, 10, 512, 300)
                ",
            )
            .bind(runtime)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Get the underlying pool
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // ==================== API Keys ====================

    /// Generate a new API key and return it (only shown once)
    ///
    /// # Errors
    /// Returns an error if the database operation or key hashing fails, or the requested key is missing or invalid.
    pub async fn create_api_key(
        &self,
        name: &str,
        rate_limit_rpm: Option<i32>,
        token_limit: Option<i32>,
    ) -> StorageResult<(ApiKey, String)> {
        let id = uuid::Uuid::new_v4().to_string();
        let raw_key = generate_api_key();
        let key_prefix = raw_key[..8].to_string();
        let key_hash = on_blocking_pool({
            let raw_key = raw_key.clone();
            move || hash_api_key(&raw_key)
        })
        .await?;

        let api_key = ApiKey {
            id: id.clone(),
            name: name.to_string(),
            key_hash,
            key_prefix,
            created_at: Utc::now(),
            last_used_at: None,
            enabled: true,
            rate_limit_rpm,
            rate_limit_tpm: token_limit,
        };

        sqlx::query(
            r"
            INSERT INTO api_keys (id, name, key_hash, key_prefix, created_at, enabled, rate_limit_rpm, rate_limit_tpm)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(&api_key.id)
        .bind(&api_key.name)
        .bind(&api_key.key_hash)
        .bind(&api_key.key_prefix)
        .bind(api_key.created_at)
        .bind(api_key.enabled)
        .bind(api_key.rate_limit_rpm)
        .bind(api_key.rate_limit_tpm)
        .execute(&self.pool)
        .await?;

        Ok((api_key, raw_key))
    }

    /// List all API keys (without the actual key)
    ///
    /// # Errors
    /// Returns an error if the database operation or key hashing fails, or the requested key is missing or invalid.
    pub async fn list_api_keys(&self) -> StorageResult<Vec<ApiKey>> {
        let keys = sqlx::query_as::<_, ApiKey>(
            r"
            SELECT id, name, key_hash, key_prefix, created_at, last_used_at, enabled, rate_limit_rpm, rate_limit_tpm
            FROM api_keys
            ORDER BY created_at DESC
            ",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(keys)
    }

    /// Get an API key by ID
    ///
    /// # Errors
    /// Returns an error if the database operation or key hashing fails, or the requested key is missing or invalid.
    pub async fn get_api_key(&self, id: &str) -> StorageResult<ApiKey> {
        sqlx::query_as::<_, ApiKey>(
            r"
            SELECT id, name, key_hash, key_prefix, created_at, last_used_at, enabled, rate_limit_rpm, rate_limit_tpm
            FROM api_keys
            WHERE id = ?
            ",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::NotFound(format!("API key {id}")))
    }

    /// Validate an API key and return the key record if valid
    ///
    /// # Errors
    /// Returns an error if the database operation or key hashing fails, or the requested key is missing or invalid.
    pub async fn validate_api_key(&self, raw_key: &str) -> StorageResult<ApiKey> {
        let prefix = if raw_key.len() >= 8 {
            &raw_key[..8]
        } else {
            return Err(StorageError::InvalidApiKey);
        };

        // Find keys with matching prefix (should be very few)
        let candidates = sqlx::query_as::<_, ApiKey>(
            r"
            SELECT id, name, key_hash, key_prefix, created_at, last_used_at, enabled, rate_limit_rpm, rate_limit_tpm
            FROM api_keys
            WHERE key_prefix = ? AND enabled = 1
            ",
        )
        .bind(prefix)
        .fetch_all(&self.pool)
        .await?;

        for key in candidates {
            // Argon2 is deliberately slow: keep it off the async workers.
            let matches = on_blocking_pool({
                let (raw_key, hash) = (raw_key.to_owned(), key.key_hash.clone());
                move || verify_api_key(&raw_key, &hash)
            })
            .await?;
            if matches {
                // Update last_used_at
                sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
                    .bind(Utc::now())
                    .bind(&key.id)
                    .execute(&self.pool)
                    .await?;
                return Ok(key);
            }
        }

        Err(StorageError::InvalidApiKey)
    }

    /// Toggle API key enabled status
    ///
    /// # Errors
    /// Returns an error if the database operation or key hashing fails, or the requested key is missing or invalid.
    pub async fn toggle_api_key(&self, id: &str) -> StorageResult<bool> {
        let result = sqlx::query("UPDATE api_keys SET enabled = NOT enabled WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!("API key {id}")));
        }

        let key = self.get_api_key(id).await?;
        Ok(key.enabled)
    }

    /// Revoke (delete) an API key
    ///
    /// # Errors
    /// Returns an error if the database operation or key hashing fails, or the requested key is missing or invalid.
    pub async fn revoke_api_key(&self, id: &str) -> StorageResult<()> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!("API key {id}")));
        }

        Ok(())
    }

    // ==================== Providers ====================

    /// Add a new provider
    ///
    /// # Errors
    /// Returns an error if the database operation fails, the record is missing, or credential encryption/decryption cannot be completed.
    pub async fn create_provider(
        &self,
        name: &str,
        api_key: Option<&str>,
        api_base: Option<&str>,
    ) -> StorageResult<Provider> {
        let id = uuid::Uuid::new_v4().to_string();
        let provider = Provider {
            id: id.clone(),
            name: name.to_string(),
            api_key_encrypted: api_key
                .map(|key| seal_provider_key(&id, key, self.credential_cipher.as_ref()))
                .transpose()?,
            api_base: api_base.map(String::from),
            enabled: true,
            created_at: Utc::now(),
        };

        sqlx::query(
            r"
            INSERT INTO providers (id, name, api_key_encrypted, api_base, enabled, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(&provider.id)
        .bind(&provider.name)
        .bind(&provider.api_key_encrypted)
        .bind(&provider.api_base)
        .bind(provider.enabled)
        .bind(provider.created_at)
        .execute(&self.pool)
        .await?;

        Ok(provider)
    }

    /// List all providers
    ///
    /// # Errors
    /// Returns an error if the database operation fails, the record is missing, or credential encryption/decryption cannot be completed.
    pub async fn list_providers(&self) -> StorageResult<Vec<Provider>> {
        let providers = sqlx::query_as::<_, Provider>(
            r"
            SELECT id, name, api_key_encrypted, api_base, enabled, created_at
            FROM providers
            ORDER BY created_at DESC
            ",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(providers)
    }

    /// Get a provider by ID
    ///
    /// # Errors
    /// Returns an error if the database operation fails, the record is missing, or credential encryption/decryption cannot be completed.
    pub async fn get_provider(&self, id: &str) -> StorageResult<Provider> {
        sqlx::query_as::<_, Provider>(
            r"
            SELECT id, name, api_key_encrypted, api_base, enabled, created_at
            FROM providers
            WHERE id = ?
            ",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::NotFound(format!("Provider {id}")))
    }

    /// Update a provider
    ///
    /// # Errors
    /// Returns an error if the database operation fails, the record is missing, or credential encryption/decryption cannot be completed.
    pub async fn update_provider(
        &self,
        id: &str,
        api_key: Option<&str>,
        api_base: Option<&str>,
    ) -> StorageResult<Provider> {
        let existing = self.get_provider(id).await?;

        let new_api_key = api_key
            .map(|key| seal_provider_key(id, key, self.credential_cipher.as_ref()))
            .transpose()?
            .or(existing.api_key_encrypted);
        let new_api_base = api_base.map(String::from).or(existing.api_base);

        sqlx::query("UPDATE providers SET api_key_encrypted = ?, api_base = ? WHERE id = ?")
            .bind(&new_api_key)
            .bind(&new_api_base)
            .bind(id)
            .execute(&self.pool)
            .await?;

        self.get_provider(id).await
    }

    /// Toggle provider enabled status
    ///
    /// # Errors
    /// Returns an error if the database operation fails, the record is missing, or credential encryption/decryption cannot be completed.
    pub async fn toggle_provider(&self, id: &str) -> StorageResult<bool> {
        let result = sqlx::query("UPDATE providers SET enabled = NOT enabled WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!("Provider {id}")));
        }

        let provider = self.get_provider(id).await?;
        Ok(provider.enabled)
    }

    /// Delete a provider
    ///
    /// # Errors
    /// Returns an error if the database operation fails, the record is missing, or credential encryption/decryption cannot be completed.
    pub async fn delete_provider(&self, id: &str) -> StorageResult<()> {
        let result = sqlx::query("DELETE FROM providers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!("Provider {id}")));
        }

        Ok(())
    }

    // ==================== Usage Logs ====================

    /// Log a usage record
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    #[allow(clippy::too_many_arguments)] // Preserve the existing public API.
    pub async fn log_usage(
        &self,
        api_key_id: Option<&str>,
        provider: &str,
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
        cost_usd: f64,
        duration_ms: i32,
    ) -> StorageResult<i64> {
        let result = sqlx::query(
            r"
            INSERT INTO usage_logs (api_key_id, provider, model, input_tokens, output_tokens, cost_usd, duration_ms, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(api_key_id)
        .bind(provider)
        .bind(model)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cost_usd)
        .bind(duration_ms)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Get recent usage logs
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn get_usage_logs(&self, limit: i32) -> StorageResult<Vec<UsageLog>> {
        let logs = sqlx::query_as::<_, UsageLog>(
            r"
            SELECT id, api_key_id, provider, model, input_tokens, output_tokens, cost_usd, duration_ms, created_at
            FROM usage_logs
            ORDER BY created_at DESC
            LIMIT ?
            ",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(logs)
    }

    /// Get usage statistics for last N hours
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn get_usage_stats(&self, hours: i32) -> StorageResult<UsageStats> {
        let since = Utc::now() - chrono::Duration::hours(i64::from(hours));

        // Get totals
        let totals: (i64, i64, i64, f64) = sqlx::query_as(
            r"
            SELECT
                COUNT(*) as total_requests,
                COALESCE(SUM(input_tokens), 0) as total_input_tokens,
                COALESCE(SUM(output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(cost_usd), 0.0) as total_cost_usd
            FROM usage_logs
            WHERE created_at >= ?
            ",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await?;

        // Get requests by provider
        let by_provider: Vec<(String, i64)> = sqlx::query_as(
            r"
            SELECT provider, COUNT(*) as count
            FROM usage_logs
            WHERE created_at >= ?
            GROUP BY provider
            ORDER BY count DESC
            ",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        // Get requests by model
        let by_model: Vec<(String, i64)> = sqlx::query_as(
            r"
            SELECT model, COUNT(*) as count
            FROM usage_logs
            WHERE created_at >= ?
            GROUP BY model
            ORDER BY count DESC
            ",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        Ok(UsageStats {
            total_requests: totals.0,
            total_input_tokens: totals.1,
            total_output_tokens: totals.2,
            total_cost_usd: totals.3,
            requests_by_provider: by_provider,
            requests_by_model: by_model,
        })
    }

    // ==================== Sandbox Configs ====================

    /// List all sandbox configurations
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn list_sandbox_configs(&self) -> StorageResult<Vec<SandboxConfig>> {
        let configs = sqlx::query_as::<_, SandboxConfig>(
            r"
            SELECT runtime, enabled, min_ready, max_total, memory_mb, timeout_seconds
            FROM sandbox_configs
            ORDER BY runtime
            ",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(configs)
    }

    /// Get a sandbox configuration by runtime
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn get_sandbox_config(&self, runtime: &str) -> StorageResult<SandboxConfig> {
        sqlx::query_as::<_, SandboxConfig>(
            r"
            SELECT runtime, enabled, min_ready, max_total, memory_mb, timeout_seconds
            FROM sandbox_configs
            WHERE runtime = ?
            ",
        )
        .bind(runtime)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::NotFound(format!("Sandbox config {runtime}")))
    }

    /// Update a sandbox configuration
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn update_sandbox_config(
        &self,
        runtime: &str,
        enabled: Option<bool>,
        min_ready: Option<i32>,
        max_total: Option<i32>,
        memory_mb: Option<i32>,
        timeout_seconds: Option<i32>,
    ) -> StorageResult<SandboxConfig> {
        let existing = self.get_sandbox_config(runtime).await?;

        let new_enabled = enabled.unwrap_or(existing.enabled);
        let new_min_ready = min_ready.unwrap_or(existing.min_ready);
        let new_max_total = max_total.unwrap_or(existing.max_total);
        let new_memory_mb = memory_mb.unwrap_or(existing.memory_mb);
        let new_timeout = timeout_seconds.unwrap_or(existing.timeout_seconds);

        sqlx::query(
            r"
            UPDATE sandbox_configs
            SET enabled = ?, min_ready = ?, max_total = ?, memory_mb = ?, timeout_seconds = ?
            WHERE runtime = ?
            ",
        )
        .bind(new_enabled)
        .bind(new_min_ready)
        .bind(new_max_total)
        .bind(new_memory_mb)
        .bind(new_timeout)
        .bind(runtime)
        .execute(&self.pool)
        .await?;

        self.get_sandbox_config(runtime).await
    }

    // ==================== Rate limits ====================

    /// Count one request for an API key, unless its limits for the current
    /// clock minute are used up. A limit of `None`, zero or less means
    /// unlimited. The check and the count are one atomic statement, so
    /// concurrent requests cannot slip past the limit together.
    ///
    /// Tokens are only known once a response arrives: the request that
    /// crosses the token limit goes through, the following ones wait.
    ///
    /// Returns `Err(wait)` with the time until the minute ends when refused.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn admit_request(
        &self,
        api_key_id: &str,
        requests_per_minute: Option<i32>,
        tokens_per_minute: Option<i32>,
    ) -> StorageResult<Result<(), std::time::Duration>> {
        let limit = |value: Option<i32>| value.filter(|v| *v > 0).map(i64::from);
        let now = Utc::now().timestamp();
        let admitted = sqlx::query(
            r"
            INSERT INTO key_rate_windows (api_key_id, minute, requests, tokens)
            VALUES (?1, ?2, 1, 0)
            ON CONFLICT (api_key_id, minute) DO UPDATE SET requests = requests + 1
            WHERE (?3 IS NULL OR requests < ?3) AND (?4 IS NULL OR tokens < ?4)
            ",
        )
        .bind(api_key_id)
        .bind(now.div_euclid(60))
        .bind(limit(requests_per_minute))
        .bind(limit(tokens_per_minute))
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0;
        Ok(if admitted {
            Ok(())
        } else {
            let remaining = 60 - now.rem_euclid(60);
            Err(std::time::Duration::from_secs(remaining.unsigned_abs()))
        })
    }

    /// Set aside the tokens a request is expected to use, unless the key's
    /// token limit for the current minute is already reached. Done in one
    /// atomic statement, so requests arriving together each see what the
    /// others reserved and cannot all slip under the limit.
    ///
    /// Call [`settle_tokens`](Self::settle_tokens) once the real figure is
    /// known. Returns `Err(wait)` with the time until the minute ends when
    /// refused.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn reserve_tokens(
        &self,
        api_key_id: &str,
        estimate: u32,
        tokens_per_minute: Option<i32>,
    ) -> StorageResult<Result<(), std::time::Duration>> {
        let now = Utc::now().timestamp();
        let reserved = sqlx::query(
            r"
            INSERT INTO key_rate_windows (api_key_id, minute, requests, tokens)
            VALUES (?1, ?2, 0, ?3)
            ON CONFLICT (api_key_id, minute) DO UPDATE SET tokens = tokens + excluded.tokens
            WHERE (?4 IS NULL OR tokens < ?4)
            ",
        )
        .bind(api_key_id)
        .bind(now.div_euclid(60))
        .bind(i64::from(estimate))
        .bind(tokens_per_minute.filter(|v| *v > 0).map(i64::from))
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0;
        Ok(if reserved {
            Ok(())
        } else {
            let remaining = 60 - now.rem_euclid(60);
            Err(std::time::Duration::from_secs(remaining.unsigned_abs()))
        })
    }

    /// Replace a reservation by what the request really used. `actual` is
    /// zero when the request failed, which gives the reservation back.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn settle_tokens(
        &self,
        api_key_id: &str,
        reserved: u32,
        actual: u32,
    ) -> StorageResult<()> {
        self.adjust_tokens(api_key_id, i64::from(actual) - i64::from(reserved))
            .await
    }

    /// Add the tokens a request consumed to the key's current minute.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn record_tokens(&self, api_key_id: &str, tokens: u32) -> StorageResult<()> {
        self.adjust_tokens(api_key_id, i64::from(tokens)).await
    }

    /// Change the token count of the key's current minute, never below zero
    /// (a reservation made in the previous minute may be settled in this one).
    async fn adjust_tokens(&self, api_key_id: &str, delta: i64) -> StorageResult<()> {
        sqlx::query(
            r"
            INSERT INTO key_rate_windows (api_key_id, minute, requests, tokens)
            VALUES (?1, ?2, 0, MAX(0, ?3))
            ON CONFLICT (api_key_id, minute) DO UPDATE SET tokens = MAX(0, tokens + ?3)
            ",
        )
        .bind(api_key_id)
        .bind(Utc::now().timestamp().div_euclid(60))
        .bind(delta)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete rate-limit counters of minutes that are over.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn cleanup_rate_windows(&self) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM key_rate_windows WHERE minute < ?")
            .bind(Utc::now().timestamp().div_euclid(60))
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    // ==================== Boxes ====================

    /// Record a new box unless `maximum` boxes exist already. Returns whether
    /// it was recorded. One statement, so that gateway processes creating
    /// boxes together cannot go over the maximum.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn create_box(
        &self,
        id: &str,
        owner: &str,
        policy: &str,
        maximum: i64,
    ) -> StorageResult<bool> {
        let created = sqlx::query(
            "INSERT INTO docker_boxes (id, owner, policy) SELECT ?1, ?2, ?3 \
             WHERE (SELECT COUNT(*) FROM docker_boxes) < ?4",
        )
        .bind(id)
        .bind(owner)
        .bind(policy)
        .bind(maximum)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(created > 0)
    }

    /// Take the right to execute in a box for `seconds`, for the run
    /// `run_id`. A box runs one execution at a time and at most `capacity`
    /// boxes execute at once, counted over every gateway process sharing the
    /// database. A claim nobody released, because its process died, ends by
    /// itself after `seconds`.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn claim_box_run(
        &self,
        box_id: &str,
        owner: &str,
        run_id: &str,
        seconds: i64,
        capacity: i64,
    ) -> StorageResult<BoxClaim> {
        let now = Utc::now().timestamp();
        let claimed = sqlx::query(
            r"
            INSERT INTO box_runs (box_id, run_id, expires_at)
            SELECT ?1, ?2, ?3
            WHERE EXISTS (SELECT 1 FROM docker_boxes WHERE id = ?1 AND owner = ?4)
              AND (SELECT COUNT(*) FROM box_runs WHERE expires_at > ?5 AND box_id != ?1) < ?6
            ON CONFLICT (box_id) DO UPDATE
              SET run_id = excluded.run_id, expires_at = excluded.expires_at
              WHERE box_runs.expires_at <= ?5
            ",
        )
        .bind(box_id)
        .bind(run_id)
        .bind(now + seconds)
        .bind(owner)
        .bind(now)
        .bind(capacity)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if claimed > 0 {
            return Ok(BoxClaim::Claimed);
        }
        // Refused: say why.
        if !self.box_exists(box_id, owner).await? {
            return Ok(BoxClaim::Missing);
        }
        Ok(if self.box_is_running(box_id).await? {
            BoxClaim::Busy
        } else {
            BoxClaim::Full
        })
    }

    /// Give back the right taken with [`claim_box_run`](Self::claim_box_run).
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn release_box_run(&self, box_id: &str, run_id: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM box_runs WHERE box_id = ? AND run_id = ?")
            .bind(box_id)
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Whether some gateway process is executing in the box.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn box_is_running(&self, box_id: &str) -> StorageResult<bool> {
        let running: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM box_runs WHERE box_id = ? AND expires_at > ?")
                .bind(box_id)
                .bind(Utc::now().timestamp())
                .fetch_optional(&self.pool)
                .await?;
        Ok(running.is_some())
    }

    /// Whether the box exists and belongs to `owner`.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn box_exists(&self, box_id: &str, owner: &str) -> StorageResult<bool> {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM docker_boxes WHERE id = ? AND owner = ?")
                .bind(box_id)
                .bind(owner)
                .fetch_optional(&self.pool)
                .await?;
        Ok(exists.is_some())
    }

    // ==================== Dashboard login failures ====================

    /// Whether dashboard logins from `client` are refused: it reached
    /// `per_client` failures, or everyone together reached `overall`, in a
    /// window of `window_seconds` that is still open. Counted over every
    /// gateway process sharing the database.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn login_blocked(
        &self,
        client: &str,
        per_client: u32,
        overall: u32,
        window_seconds: i64,
    ) -> StorageResult<bool> {
        let blocked: Option<i64> = sqlx::query_scalar(
            r"
            SELECT 1 FROM login_failures
            WHERE window_start > ?1
              AND ((client = ?2 AND failures >= ?3) OR (client = '*' AND failures >= ?4))
            LIMIT 1
            ",
        )
        .bind(Utc::now().timestamp() - window_seconds)
        .bind(client)
        .bind(per_client)
        .bind(overall)
        .fetch_optional(&self.pool)
        .await?;
        Ok(blocked.is_some())
    }

    /// Count a failed dashboard login for `client` and for everyone. At most
    /// `max_clients` clients are remembered: closed windows go first, then
    /// the oldest.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn record_login_failure(
        &self,
        client: &str,
        window_seconds: i64,
        max_clients: i64,
    ) -> StorageResult<()> {
        let now = Utc::now().timestamp();
        let closed = now - window_seconds;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM login_failures WHERE window_start <= ?")
            .bind(closed)
            .execute(&mut *transaction)
            .await?;
        for counted in [client, "*"] {
            sqlx::query(
                r"
                INSERT INTO login_failures (client, failures, window_start) VALUES (?1, 1, ?2)
                ON CONFLICT (client) DO UPDATE SET failures = failures + 1
                ",
            )
            .bind(counted)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        // `*` and `max_clients` clients, the most recent windows.
        sqlx::query(
            r"
            DELETE FROM login_failures WHERE client IN (
                SELECT client FROM login_failures WHERE client != '*'
                ORDER BY window_start DESC LIMIT -1 OFFSET ?
            )
            ",
        )
        .bind(max_clients)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Forget a client's failed dashboard logins, after a successful one.
    ///
    /// # Errors
    /// Returns a storage error if the database query or connection fails.
    pub async fn clear_login_failures(&self, client: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM login_failures WHERE client = ? AND client != '*'")
            .bind(client)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ==================== Admin Sessions ====================

    /// Create a new admin session
    ///
    /// # Errors
    /// Returns an error if the session database cannot be queried or updated.
    pub async fn create_admin_session(&self) -> StorageResult<AdminSession> {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = Utc::now();
        let expires_at = created_at + chrono::Duration::hours(24);

        let session = AdminSession {
            id: id.clone(),
            created_at,
            expires_at,
        };

        sqlx::query(
            r"
            INSERT INTO admin_sessions (id, created_at, expires_at)
            VALUES (?, ?, ?)
            ",
        )
        .bind(&session.id)
        .bind(session.created_at)
        .bind(session.expires_at)
        .execute(&self.pool)
        .await?;

        Ok(session)
    }

    /// Validate an admin session
    ///
    /// # Errors
    /// Returns an error if the session database cannot be queried or updated.
    pub async fn validate_admin_session(&self, session_id: &str) -> StorageResult<bool> {
        let session = sqlx::query_as::<_, AdminSession>(
            r"
            SELECT id, created_at, expires_at
            FROM admin_sessions
            WHERE id = ? AND expires_at > ?
            ",
        )
        .bind(session_id)
        // Bound, not `datetime('now')`: stored timestamps and SQLite's own
        // text format do not compare correctly as strings.
        .bind(Utc::now())
        .fetch_optional(&self.pool)
        .await?;

        Ok(session.is_some())
    }

    /// Delete an admin session (logout)
    ///
    /// # Errors
    /// Returns an error if the session database cannot be queried or updated.
    pub async fn delete_admin_session(&self, session_id: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM admin_sessions WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Clean up expired sessions
    ///
    /// # Errors
    /// Returns an error if the session database cannot be queried or updated.
    pub async fn cleanup_expired_sessions(&self) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= ?")
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }
}

/// Generate a random API key (sk-...)
fn generate_api_key() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    format!("sk-{}", base64_encode(&bytes))
}

/// Base64 encode bytes (URL-safe variant)
fn base64_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for byte in bytes {
        write!(result, "{byte:02x}").unwrap();
    }
    result
}

/// Hash an API key using argon2
fn hash_api_key(key: &str) -> StorageResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(key.as_bytes(), &salt)
        .map_err(|e| StorageError::Hash(e.to_string()))?;
    Ok(hash.to_string())
}

/// Run CPU-heavy hashing on the blocking thread pool, so it cannot stall the
/// async workers that serve every other request.
async fn on_blocking_pool<T: Send + 'static>(
    work: impl FnOnce() -> StorageResult<T> + Send + 'static,
) -> StorageResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| StorageError::Hash(format!("hashing task failed: {e}")))?
}

/// Verify an API key against a hash
fn verify_api_key(key: &str, hash: &str) -> StorageResult<bool> {
    let parsed_hash = PasswordHash::new(hash).map_err(|e| StorageError::Hash(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(key.as_bytes(), &parsed_hash)
        .is_ok())
}

fn provider_cipher() -> StorageResult<aes_gcm::Aes256Gcm> {
    use aes_gcm::KeyInit;
    use base64::Engine;
    let encoded = std::env::var("GATEWAY_MASTER_KEY").map_err(|_| {
        StorageError::Hash(
            "GATEWAY_MASTER_KEY is required to store provider credentials (base64, 32 bytes)"
                .into(),
        )
    })?;
    let key = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| StorageError::Hash("Invalid master key encoding".into()))?;
    aes_gcm::Aes256Gcm::new_from_slice(&key)
        .map_err(|_| StorageError::Hash("Master key must contain 32 bytes".into()))
}

fn seal_provider_key(
    id: &str,
    plaintext: &str,
    cipher: Option<&aes_gcm::Aes256Gcm>,
) -> StorageResult<String> {
    use aes_gcm::{
        aead::{Aead, AeadCore, OsRng, Payload},
        Aes256Gcm,
    };
    use base64::Engine;
    let cipher = cipher.cloned().map_or_else(provider_cipher, Ok)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext.as_bytes(),
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| StorageError::Hash("Credential encryption failed".into()))?;
    let mut bytes = nonce.to_vec();
    bytes.extend(ciphertext);
    Ok(format!(
        "v1:{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Decrypt a provider credential. Legacy plaintext values are rejected explicitly.
///
/// # Errors
/// Returns an error if the database operation fails, the record is missing, or credential encryption/decryption cannot be completed.
pub fn decrypt_provider_key(provider: &Provider) -> StorageResult<Option<String>> {
    decrypt_provider_key_with_cipher(provider, None)
}

fn decrypt_provider_key_with_cipher(
    provider: &Provider,
    cipher: Option<&aes_gcm::Aes256Gcm>,
) -> StorageResult<Option<String>> {
    use aes_gcm::aead::{Aead, Payload};
    use base64::Engine;
    let Some(value) = &provider.api_key_encrypted else {
        return Ok(None);
    };
    let value = value.strip_prefix("v1:").ok_or_else(|| StorageError::Hash("Legacy plaintext credential: re-enter the provider key with GATEWAY_MASTER_KEY configured".into()))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| StorageError::Hash("Invalid encrypted credential".into()))?;
    if bytes.len() < 28 {
        return Err(StorageError::Hash("Truncated encrypted credential".into()));
    }
    let plaintext = cipher
        .cloned()
        .map_or_else(provider_cipher, Ok)?
        .decrypt(
            aes_gcm::Nonce::from_slice(&bytes[..12]),
            Payload {
                msg: &bytes[12..],
                aad: provider.id.as_bytes(),
            },
        )
        .map_err(|_| StorageError::Hash("Credential authentication failed".into()))?;
    String::from_utf8(plaintext)
        .map(Some)
        .map_err(|_| StorageError::Hash("Invalid credential encoding".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_api_key_lifecycle() {
        let storage = Storage::in_memory().await.unwrap();

        // Create a key
        let (key, raw_key) = storage
            .create_api_key("Test Key", Some(100), Some(10000))
            .await
            .unwrap();

        assert_eq!(key.name, "Test Key");
        assert!(key.enabled);
        assert!(raw_key.starts_with("sk-"));

        // Validate the key
        let validated = storage.validate_api_key(&raw_key).await.unwrap();
        assert_eq!(validated.id, key.id);

        // List keys
        let keys = storage.list_api_keys().await.unwrap();
        assert_eq!(keys.len(), 1);

        // Toggle key
        let enabled = storage.toggle_api_key(&key.id).await.unwrap();
        assert!(!enabled);

        // Validation should fail for disabled key
        let result = storage.validate_api_key(&raw_key).await;
        assert!(result.is_err());

        // Revoke key
        storage.revoke_api_key(&key.id).await.unwrap();
        let keys = storage.list_api_keys().await.unwrap();
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn test_provider_lifecycle() {
        let storage = Storage::in_memory()
            .await
            .unwrap()
            .with_master_key(&[7; 32]);

        // Create provider
        let provider = storage
            .create_provider("openai", Some("sk-test"), Some("https://api.openai.com"))
            .await
            .unwrap();

        assert_eq!(
            storage.decrypt_provider_key(&provider).unwrap().as_deref(),
            Some("sk-test")
        );
        assert!(!provider
            .api_key_encrypted
            .as_ref()
            .unwrap()
            .contains("sk-test"));
        let mut tampered = provider.clone();
        tampered.id = "other-provider".into();
        assert!(storage.decrypt_provider_key(&tampered).is_err());
        assert_eq!(provider.name, "openai");
        assert!(provider.enabled);

        // List providers
        let providers = storage.list_providers().await.unwrap();
        assert_eq!(providers.len(), 1);

        // Toggle provider
        let enabled = storage.toggle_provider(&provider.id).await.unwrap();
        assert!(!enabled);

        // Delete provider
        storage.delete_provider(&provider.id).await.unwrap();
        let providers = storage.list_providers().await.unwrap();
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn test_usage_logging() {
        let storage = Storage::in_memory().await.unwrap();

        // Log usage
        storage
            .log_usage(None, "openai", "gpt-4", 100, 50, 0.01, 500)
            .await
            .unwrap();

        storage
            .log_usage(None, "anthropic", "claude-3", 200, 100, 0.02, 600)
            .await
            .unwrap();

        // Get logs
        let logs = storage.get_usage_logs(10).await.unwrap();
        assert_eq!(logs.len(), 2);

        // Get stats
        let stats = storage.get_usage_stats(24).await.unwrap();
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.total_input_tokens, 300);
        assert_eq!(stats.total_output_tokens, 150);
    }

    #[tokio::test]
    async fn test_sandbox_configs() {
        let storage = Storage::in_memory().await.unwrap();

        // Default configs should be created
        let configs = storage.list_sandbox_configs().await.unwrap();
        assert!(!configs.is_empty());

        // Update config
        let updated = storage
            .update_sandbox_config("python", Some(false), Some(5), None, None, None)
            .await
            .unwrap();

        assert!(!updated.enabled);
        assert_eq!(updated.min_ready, 5);
    }

    #[tokio::test]
    async fn rate_limits_are_counted_in_the_database() {
        let storage = Storage::in_memory().await.unwrap();
        // Tests may straddle a minute boundary; the limits are high enough
        // for that not to matter except where a refusal is expected.
        assert!(storage
            .admit_request("a", Some(2), None)
            .await
            .unwrap()
            .is_ok());
        assert!(storage
            .admit_request("a", Some(2), None)
            .await
            .unwrap()
            .is_ok());
        let third = storage.admit_request("a", Some(2), None).await.unwrap();
        // Refused, unless the clock minute just rolled over.
        if let Err(wait) = third {
            assert!((1..=60).contains(&wait.as_secs()));
        }
        // Other keys and unlimited keys are unaffected.
        assert!(storage
            .admit_request("b", Some(2), None)
            .await
            .unwrap()
            .is_ok());
        for _ in 0..50 {
            assert!(storage
                .admit_request("c", None, Some(0))
                .await
                .unwrap()
                .is_ok());
        }
        // Tokens count against the per-minute token limit.
        storage.record_tokens("d", 150).await.unwrap();
        let after_tokens = storage.admit_request("d", None, Some(100)).await.unwrap();
        assert!(after_tokens.is_err() || Utc::now().timestamp().rem_euclid(60) == 0);
        // Counters live in the database, not in the process.
        let (requests, tokens): (i64, i64) =
            sqlx::query_as("SELECT SUM(requests), SUM(tokens) FROM key_rate_windows")
                .fetch_one(storage.pool())
                .await
                .unwrap();
        assert!(requests >= 53 && tokens == 150, "{requests} {tokens}");
    }

    #[tokio::test]
    async fn session_expired_earlier_today_is_rejected() {
        let storage = Storage::in_memory().await.unwrap();
        let session = storage.create_admin_session().await.unwrap();
        // Expired a minute ago: still "today", which a text comparison with
        // SQLite's `datetime('now')` used to accept until midnight UTC.
        sqlx::query("UPDATE admin_sessions SET expires_at = ? WHERE id = ?")
            .bind(Utc::now() - chrono::Duration::minutes(1))
            .bind(&session.id)
            .execute(storage.pool())
            .await
            .unwrap();
        assert!(!storage.validate_admin_session(&session.id).await.unwrap());
        assert_eq!(storage.cleanup_expired_sessions().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_admin_sessions() {
        let storage = Storage::in_memory().await.unwrap();

        // Create session
        let session = storage.create_admin_session().await.unwrap();

        // Validate session
        let valid = storage.validate_admin_session(&session.id).await.unwrap();
        assert!(valid);

        // Invalid session
        let valid = storage.validate_admin_session("invalid").await.unwrap();
        assert!(!valid);

        // Delete session
        storage.delete_admin_session(&session.id).await.unwrap();
        let valid = storage.validate_admin_session(&session.id).await.unwrap();
        assert!(!valid);
    }
}

/// Two handles on one database file stand for two gateway processes: each
/// has its own connections, and they only meet in the file.
#[cfg(test)]
mod shared_database_tests {
    use super::*;

    struct Shared {
        path: std::path::PathBuf,
        first: Storage,
        second: Storage,
    }

    impl Shared {
        async fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("gateway-{}.db", uuid::Uuid::new_v4().simple()));
            let url = format!("sqlite:{}?mode=rwc", path.display());
            // Opened together, as processes started together are.
            let (first, second) = tokio::join!(Storage::new(&url), Storage::new(&url));
            Self {
                path,
                first: first.unwrap(),
                second: second.unwrap(),
            }
        }
    }

    impl Drop for Shared {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn request_limit_is_shared() {
        let shared = Shared::new().await;
        let (key, _) = shared
            .first
            .create_api_key("shared", Some(40), None)
            .await
            .unwrap();
        // Not across a minute boundary, where a new window would start.
        while Utc::now().timestamp().rem_euclid(60) > 50 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let mut attempts = Vec::new();
        for n in 0..200 {
            let storage = if n % 2 == 0 {
                shared.first.clone()
            } else {
                shared.second.clone()
            };
            let id = key.id.clone();
            attempts.push(tokio::spawn(async move {
                storage.admit_request(&id, Some(40), None).await.unwrap()
            }));
        }
        let mut admitted = 0;
        for attempt in attempts {
            if attempt.await.unwrap().is_ok() {
                admitted += 1;
            }
        }
        assert_eq!(admitted, 40);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_box_executes_in_one_process_at_a_time() {
        let shared = Shared::new().await;
        assert!(shared
            .first
            .create_box("box", "alice", "{}", 10)
            .await
            .unwrap());

        let mut claims = Vec::new();
        for n in 0..20 {
            let storage = if n % 2 == 0 {
                shared.first.clone()
            } else {
                shared.second.clone()
            };
            claims.push(tokio::spawn(async move {
                storage
                    .claim_box_run("box", "alice", &format!("run-{n}"), 60, 16)
                    .await
                    .unwrap()
            }));
        }
        let mut outcomes = Vec::new();
        for claim in claims {
            outcomes.push(claim.await.unwrap());
        }
        let claimed = outcomes
            .iter()
            .filter(|outcome| **outcome == BoxClaim::Claimed)
            .count();
        assert_eq!(claimed, 1, "{outcomes:?}");
        assert!(outcomes
            .iter()
            .all(|outcome| matches!(outcome, BoxClaim::Claimed | BoxClaim::Busy)));
        // Seen as running from the other process, whichever claimed it.
        assert!(shared.second.box_is_running("box").await.unwrap());
        assert!(shared.first.box_is_running("box").await.unwrap());

        // Someone else's run ID does not release it; the right one does.
        shared
            .second
            .release_box_run("box", "not-the-run")
            .await
            .unwrap();
        assert!(shared.first.box_is_running("box").await.unwrap());
        for n in 0..20 {
            shared
                .second
                .release_box_run("box", &format!("run-{n}"))
                .await
                .unwrap();
        }
        assert!(!shared.first.box_is_running("box").await.unwrap());
        assert_eq!(
            shared
                .first
                .claim_box_run("box", "alice", "again", 60, 16)
                .await
                .unwrap(),
            BoxClaim::Claimed
        );
    }

    #[tokio::test]
    async fn box_claims_respect_owner_capacity_and_expiry() {
        let shared = Shared::new().await;
        for id in ["a", "b", "c"] {
            assert!(shared.first.create_box(id, "alice", "{}", 3).await.unwrap());
        }
        // The fourth box is one too many, from either process.
        assert!(!shared
            .second
            .create_box("d", "alice", "{}", 3)
            .await
            .unwrap());

        assert_eq!(
            shared
                .first
                .claim_box_run("a", "mallory", "r", 60, 2)
                .await
                .unwrap(),
            BoxClaim::Missing
        );
        assert_eq!(
            shared
                .first
                .claim_box_run("a", "alice", "r1", 60, 2)
                .await
                .unwrap(),
            BoxClaim::Claimed
        );
        // A claim whose process died: it ends at its expiry.
        assert_eq!(
            shared
                .second
                .claim_box_run("b", "alice", "r2", 1, 2)
                .await
                .unwrap(),
            BoxClaim::Claimed
        );
        assert_eq!(
            shared
                .first
                .claim_box_run("c", "alice", "r3", 60, 2)
                .await
                .unwrap(),
            BoxClaim::Full
        );
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        assert_eq!(
            shared
                .first
                .claim_box_run("c", "alice", "r3", 60, 2)
                .await
                .unwrap(),
            BoxClaim::Claimed
        );
        assert_eq!(
            shared
                .second
                .claim_box_run("b", "alice", "r4", 60, 3)
                .await
                .unwrap(),
            BoxClaim::Claimed
        );
    }

    #[tokio::test]
    async fn failed_logins_are_counted_together() {
        let shared = Shared::new().await;
        let blocked = |storage: &Storage, client: &'static str| {
            let storage = storage.clone();
            async move { storage.login_blocked(client, 5, 12, 300).await.unwrap() }
        };
        // Five failures spread over two processes block the client on both.
        for n in 0..5 {
            assert!(!blocked(&shared.first, "10.0.0.1").await);
            let storage = if n % 2 == 0 {
                &shared.first
            } else {
                &shared.second
            };
            storage
                .record_login_failure("10.0.0.1", 300, 100)
                .await
                .unwrap();
        }
        assert!(blocked(&shared.first, "10.0.0.1").await);
        assert!(blocked(&shared.second, "10.0.0.1").await);
        assert!(!blocked(&shared.second, "10.0.0.2").await);

        // A successful login clears the client, not the overall count.
        shared
            .second
            .clear_login_failures("10.0.0.1")
            .await
            .unwrap();
        assert!(!blocked(&shared.first, "10.0.0.1").await);
        shared.second.clear_login_failures("*").await.unwrap();

        // Many addresses: the overall limit blocks everyone.
        for n in 0..7 {
            shared
                .second
                .record_login_failure(&format!("10.1.0.{n}"), 300, 100)
                .await
                .unwrap();
        }
        assert!(blocked(&shared.first, "10.9.9.9").await);

        // Only so many clients are remembered.
        for n in 0..30 {
            shared
                .first
                .record_login_failure(&format!("10.2.0.{n}"), 300, 10)
                .await
                .unwrap();
        }
        let remembered: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM login_failures WHERE client != '*'")
                .fetch_one(shared.first.pool())
                .await
                .unwrap();
        assert_eq!(remembered, 10);
        // A closed window no longer blocks.
        assert!(!shared
            .first
            .login_blocked("10.9.9.9", 5, 12, 0)
            .await
            .unwrap());
    }
}
