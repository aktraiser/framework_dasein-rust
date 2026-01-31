//! SQLite storage for Gateway admin dashboard
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
use sqlx::{sqlite::SqlitePoolOptions, FromRow, SqlitePool};
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

/// Storage handle for SQLite operations
#[derive(Clone)]
pub struct Storage {
    pool: SqlitePool,
}

impl Storage {
    /// Create a new storage instance with the given database URL
    pub async fn new(database_url: &str) -> StorageResult<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        let storage = Self { pool };
        storage.run_migrations().await?;
        Ok(storage)
    }

    /// Create an in-memory storage for testing
    pub async fn in_memory() -> StorageResult<Self> {
        Self::new("sqlite::memory:").await
    }

    /// Run database migrations
    async fn run_migrations(&self) -> StorageResult<()> {
        sqlx::query(
            r#"
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
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                api_key_encrypted TEXT,
                api_base TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
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
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sandbox_configs (
                runtime TEXT PRIMARY KEY,
                enabled INTEGER NOT NULL DEFAULT 1,
                min_ready INTEGER NOT NULL DEFAULT 2,
                max_total INTEGER NOT NULL DEFAULT 10,
                memory_mb INTEGER NOT NULL DEFAULT 512,
                timeout_seconds INTEGER NOT NULL DEFAULT 300
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS admin_sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Create indexes for better query performance
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_usage_logs_created_at ON usage_logs(created_at)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_usage_logs_api_key_id ON usage_logs(api_key_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Insert default sandbox configs if not present
        for runtime in &["python", "node", "typescript", "rust", "go", "bash"] {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO sandbox_configs (runtime, enabled, min_ready, max_total, memory_mb, timeout_seconds)
                VALUES (?, 1, 2, 10, 512, 300)
                "#,
            )
            .bind(runtime)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Get the underlying pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // ==================== API Keys ====================

    /// Generate a new API key and return it (only shown once)
    pub async fn create_api_key(
        &self,
        name: &str,
        rate_limit_rpm: Option<i32>,
        rate_limit_tpm: Option<i32>,
    ) -> StorageResult<(ApiKey, String)> {
        let id = uuid::Uuid::new_v4().to_string();
        let raw_key = generate_api_key();
        let key_prefix = raw_key[..8].to_string();
        let key_hash = hash_api_key(&raw_key)?;

        let api_key = ApiKey {
            id: id.clone(),
            name: name.to_string(),
            key_hash,
            key_prefix,
            created_at: Utc::now(),
            last_used_at: None,
            enabled: true,
            rate_limit_rpm,
            rate_limit_tpm,
        };

        sqlx::query(
            r#"
            INSERT INTO api_keys (id, name, key_hash, key_prefix, created_at, enabled, rate_limit_rpm, rate_limit_tpm)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
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
    pub async fn list_api_keys(&self) -> StorageResult<Vec<ApiKey>> {
        let keys = sqlx::query_as::<_, ApiKey>(
            r#"
            SELECT id, name, key_hash, key_prefix, created_at, last_used_at, enabled, rate_limit_rpm, rate_limit_tpm
            FROM api_keys
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(keys)
    }

    /// Get an API key by ID
    pub async fn get_api_key(&self, id: &str) -> StorageResult<ApiKey> {
        sqlx::query_as::<_, ApiKey>(
            r#"
            SELECT id, name, key_hash, key_prefix, created_at, last_used_at, enabled, rate_limit_rpm, rate_limit_tpm
            FROM api_keys
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::NotFound(format!("API key {id}")))
    }

    /// Validate an API key and return the key record if valid
    pub async fn validate_api_key(&self, raw_key: &str) -> StorageResult<ApiKey> {
        let prefix = if raw_key.len() >= 8 {
            &raw_key[..8]
        } else {
            return Err(StorageError::InvalidApiKey);
        };

        // Find keys with matching prefix (should be very few)
        let candidates = sqlx::query_as::<_, ApiKey>(
            r#"
            SELECT id, name, key_hash, key_prefix, created_at, last_used_at, enabled, rate_limit_rpm, rate_limit_tpm
            FROM api_keys
            WHERE key_prefix = ? AND enabled = 1
            "#,
        )
        .bind(prefix)
        .fetch_all(&self.pool)
        .await?;

        for key in candidates {
            if verify_api_key(raw_key, &key.key_hash)? {
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
            api_key_encrypted: api_key.map(String::from), // TODO: Encrypt in phase 2
            api_base: api_base.map(String::from),
            enabled: true,
            created_at: Utc::now(),
        };

        sqlx::query(
            r#"
            INSERT INTO providers (id, name, api_key_encrypted, api_base, enabled, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
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
    pub async fn list_providers(&self) -> StorageResult<Vec<Provider>> {
        let providers = sqlx::query_as::<_, Provider>(
            r#"
            SELECT id, name, api_key_encrypted, api_base, enabled, created_at
            FROM providers
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(providers)
    }

    /// Get a provider by ID
    pub async fn get_provider(&self, id: &str) -> StorageResult<Provider> {
        sqlx::query_as::<_, Provider>(
            r#"
            SELECT id, name, api_key_encrypted, api_base, enabled, created_at
            FROM providers
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::NotFound(format!("Provider {id}")))
    }

    /// Update a provider
    pub async fn update_provider(
        &self,
        id: &str,
        api_key: Option<&str>,
        api_base: Option<&str>,
    ) -> StorageResult<Provider> {
        let existing = self.get_provider(id).await?;

        let new_api_key = api_key.map(String::from).or(existing.api_key_encrypted);
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
            r#"
            INSERT INTO usage_logs (api_key_id, provider, model, input_tokens, output_tokens, cost_usd, duration_ms, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
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
    pub async fn get_usage_logs(&self, limit: i32) -> StorageResult<Vec<UsageLog>> {
        let logs = sqlx::query_as::<_, UsageLog>(
            r#"
            SELECT id, api_key_id, provider, model, input_tokens, output_tokens, cost_usd, duration_ms, created_at
            FROM usage_logs
            ORDER BY created_at DESC
            LIMIT ?
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(logs)
    }

    /// Get usage statistics for last N hours
    pub async fn get_usage_stats(&self, hours: i32) -> StorageResult<UsageStats> {
        let since = Utc::now() - chrono::Duration::hours(i64::from(hours));

        // Get totals
        let totals: (i64, i64, i64, f64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) as total_requests,
                COALESCE(SUM(input_tokens), 0) as total_input_tokens,
                COALESCE(SUM(output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(cost_usd), 0.0) as total_cost_usd
            FROM usage_logs
            WHERE created_at >= ?
            "#,
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await?;

        // Get requests by provider
        let by_provider: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT provider, COUNT(*) as count
            FROM usage_logs
            WHERE created_at >= ?
            GROUP BY provider
            ORDER BY count DESC
            "#,
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        // Get requests by model
        let by_model: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT model, COUNT(*) as count
            FROM usage_logs
            WHERE created_at >= ?
            GROUP BY model
            ORDER BY count DESC
            "#,
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
    pub async fn list_sandbox_configs(&self) -> StorageResult<Vec<SandboxConfig>> {
        let configs = sqlx::query_as::<_, SandboxConfig>(
            r#"
            SELECT runtime, enabled, min_ready, max_total, memory_mb, timeout_seconds
            FROM sandbox_configs
            ORDER BY runtime
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(configs)
    }

    /// Get a sandbox configuration by runtime
    pub async fn get_sandbox_config(&self, runtime: &str) -> StorageResult<SandboxConfig> {
        sqlx::query_as::<_, SandboxConfig>(
            r#"
            SELECT runtime, enabled, min_ready, max_total, memory_mb, timeout_seconds
            FROM sandbox_configs
            WHERE runtime = ?
            "#,
        )
        .bind(runtime)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::NotFound(format!("Sandbox config {runtime}")))
    }

    /// Update a sandbox configuration
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
            r#"
            UPDATE sandbox_configs
            SET enabled = ?, min_ready = ?, max_total = ?, memory_mb = ?, timeout_seconds = ?
            WHERE runtime = ?
            "#,
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

    // ==================== Admin Sessions ====================

    /// Create a new admin session
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
            r#"
            INSERT INTO admin_sessions (id, created_at, expires_at)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(&session.id)
        .bind(session.created_at)
        .bind(session.expires_at)
        .execute(&self.pool)
        .await?;

        Ok(session)
    }

    /// Validate an admin session
    pub async fn validate_admin_session(&self, session_id: &str) -> StorageResult<bool> {
        let session = sqlx::query_as::<_, AdminSession>(
            r#"
            SELECT id, created_at, expires_at
            FROM admin_sessions
            WHERE id = ? AND expires_at > datetime('now')
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(session.is_some())
    }

    /// Delete an admin session (logout)
    pub async fn delete_admin_session(&self, session_id: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM admin_sessions WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired_sessions(&self) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= datetime('now')")
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

/// Verify an API key against a hash
fn verify_api_key(key: &str, hash: &str) -> StorageResult<bool> {
    let parsed_hash =
        PasswordHash::new(hash).map_err(|e| StorageError::Hash(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(key.as_bytes(), &parsed_hash)
        .is_ok())
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
        let storage = Storage::in_memory().await.unwrap();

        // Create provider
        let provider = storage
            .create_provider("openai", Some("sk-test"), Some("https://api.openai.com"))
            .await
            .unwrap();

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
