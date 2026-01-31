//! Storage traits.

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;

use crate::error::StorageError;

/// State store trait (Redis-like).
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Get a value by key.
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>, StorageError>;

    /// Set a value with optional TTL.
    async fn set<T: Serialize + Send + Sync>(
        &self,
        key: &str,
        value: &T,
        ttl_ms: Option<u64>,
    ) -> Result<(), StorageError>;

    /// Delete a key.
    async fn delete(&self, key: &str) -> Result<(), StorageError>;

    /// Check if key exists.
    async fn exists(&self, key: &str) -> Result<bool, StorageError>;
}

/// A point in vector space.
#[derive(Debug, Clone)]
pub struct VectorPoint {
    /// Unique ID
    pub id: String,
    /// Vector embedding
    pub vector: Vec<f32>,
    /// Associated metadata
    pub payload: HashMap<String, serde_json::Value>,
}

/// Result from vector search.
#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    /// Point ID
    pub id: String,
    /// Similarity score
    pub score: f32,
    /// Point payload
    pub payload: HashMap<String, serde_json::Value>,
}

/// Vector store trait (Qdrant-like).
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Upsert points into a collection.
    async fn upsert(&self, collection: &str, points: Vec<VectorPoint>)
        -> Result<(), StorageError>;

    /// Search for similar vectors.
    async fn search(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<VectorSearchResult>, StorageError>;

    /// Delete points by ID.
    async fn delete(&self, collection: &str, ids: Vec<String>) -> Result<(), StorageError>;

    /// Create a collection.
    async fn create_collection(&self, name: &str, vector_size: u64) -> Result<(), StorageError>;

    /// Check if collection exists.
    async fn collection_exists(&self, name: &str) -> Result<bool, StorageError>;
}
