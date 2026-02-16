# agentic-storage

Storage backends for state and vector search.

## Installation

```toml
[dependencies]
agentic-storage = { version = "0.1", features = ["redis", "qdrant"] }
```

## RedisStore

Key-value state storage with TTL support.

```rust
use dasein_agentic_storage::RedisStore;

let store = RedisStore::connect("redis://localhost:6379").await?;

// Set with TTL (1 hour)
store.set("agent:123:state", &state, Some(3600000)).await?;

// Get
let state: Option<AgentState> = store.get("agent:123:state").await?;

// Delete
store.delete("agent:123:state").await?;

// Check existence
let exists = store.exists("agent:123:state").await?;
```

## QdrantStore

Vector storage for embeddings and similarity search.

```rust
use dasein_agentic_storage::{QdrantStore, VectorPoint};

let store = QdrantStore::new("http://localhost:6334").await?;

// Create collection
store.create_collection("memories", 1536).await?;

// Upsert vectors
let points = vec![
    VectorPoint {
        id: "mem-1".to_string(),
        vector: embedding_vector,
        payload: [("text".to_string(), json!("Hello world"))].into(),
    }
];
store.upsert("memories", points).await?;

// Search similar vectors
let results = store.search(
    "memories",
    query_vector,
    10,              // limit
    Some(0.7),       // score threshold
).await?;

for result in results {
    println!("ID: {}, Score: {}", result.id, result.score);
}
```

## Traits

### StateStore

```rust
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>, StorageError>;
    async fn set<T: Serialize + Send + Sync>(
        &self,
        key: &str,
        value: &T,
        ttl_ms: Option<u64>,
    ) -> Result<(), StorageError>;
    async fn delete(&self, key: &str) -> Result<(), StorageError>;
    async fn exists(&self, key: &str) -> Result<bool, StorageError>;
}
```

### VectorStore

```rust
#[async_trait]
pub trait VectorStore: Send + Sync {
    async fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<(), StorageError>;
    async fn search(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<VectorSearchResult>, StorageError>;
    async fn delete(&self, collection: &str, ids: Vec<String>) -> Result<(), StorageError>;
    async fn create_collection(&self, name: &str, vector_size: u64) -> Result<(), StorageError>;
    async fn collection_exists(&self, name: &str) -> Result<bool, StorageError>;
}
```

## Types

```rust
pub struct VectorPoint {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: HashMap<String, Value>,
}

pub struct VectorSearchResult {
    pub id: String,
    pub score: f32,
    pub payload: HashMap<String, Value>,
}
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `redis` | Enable RedisStore (default) |
| `qdrant` | Enable QdrantStore |
| `all` | All storage backends |
