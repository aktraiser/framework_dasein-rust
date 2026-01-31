//! # agentic-storage
//!
//! Storage adapters for the Agentic Framework.
//!
//! - Redis for state storage
//! - Qdrant for vector storage

mod error;
mod traits;

#[cfg(feature = "redis")]
mod redis_store;

#[cfg(feature = "qdrant")]
mod qdrant_store;

pub use error::StorageError;
pub use traits::{StateStore, VectorPoint, VectorSearchResult, VectorStore};

#[cfg(feature = "redis")]
pub use redis_store::{KeyPatterns, RedisStore};

#[cfg(feature = "qdrant")]
pub use qdrant_store::QdrantStore;
