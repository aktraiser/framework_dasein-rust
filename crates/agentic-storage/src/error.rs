//! Storage error types.

use thiserror::Error;

/// Errors that can occur with storage operations.
#[derive(Error, Debug)]
pub enum StorageError {
    /// Connection failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Get operation failed
    #[error("Get failed: {0}")]
    GetFailed(String),

    /// Set operation failed
    #[error("Set failed: {0}")]
    SetFailed(String),

    /// Delete operation failed
    #[error("Delete failed: {0}")]
    DeleteFailed(String),

    /// Query failed
    #[error("Query failed: {0}")]
    QueryFailed(String),

    /// Create failed
    #[error("Create failed: {0}")]
    CreateFailed(String),

    /// Upsert failed
    #[error("Upsert failed: {0}")]
    UpsertFailed(String),

    /// Search failed
    #[error("Search failed: {0}")]
    SearchFailed(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
}
