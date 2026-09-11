//! Skill registry for storing and retrieving skills.
//!
//! Uses Redis (StateStore) for full skill data and Qdrant (VectorStore)
//! for semantic search via embeddings.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dasein_agentic_llm::Embedder;
use dasein_agentic_storage::{StateStore, VectorPoint, VectorStore};
use tracing::{debug, instrument};

use crate::error::SkillError;
use crate::types::{Skill, SkillMatch};

const SKILLS_COLLECTION: &str = "skills";
const SKILL_KEY_PREFIX: &str = "skill:";

/// Registry for storing and retrieving skills.
///
/// The registry uses a dual-storage approach:
/// - **Redis (StateStore)**: Stores full skill definitions as JSON
/// - **Qdrant (VectorStore)**: Stores embeddings for semantic search
///
/// # Example
///
/// ```rust,ignore
/// use dasein_agentic_skills::{SkillRegistry, Skill};
///
/// let registry = SkillRegistry::new(state_store, vector_store, embedder).await?;
///
/// // Register a skill
/// let skill = Skill::new("code_review", "Reviews code quality");
/// registry.register(skill).await?;
///
/// // Search for skills semantically
/// let matches = registry.search("check my code for bugs", 3).await?;
/// ```
pub struct SkillRegistry<S, V, E>
where
    S: StateStore,
    V: VectorStore,
    E: Embedder,
{
    state_store: Arc<S>,
    vector_store: Arc<V>,
    embedder: Arc<E>,
    collection_name: String,
}

impl<S, V, E> SkillRegistry<S, V, E>
where
    S: StateStore,
    V: VectorStore,
    E: Embedder,
{
    /// Create a new skill registry.
    ///
    /// This will create the vector collection if it doesn't exist.
    pub async fn new(
        state_store: Arc<S>,
        vector_store: Arc<V>,
        embedder: Arc<E>,
    ) -> Result<Self, SkillError> {
        let collection_name = SKILLS_COLLECTION.to_string();
        let dimension = embedder.dimension() as u64;

        // Ensure collection exists
        if !vector_store
            .collection_exists(&collection_name)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?
        {
            vector_store
                .create_collection(&collection_name, dimension)
                .await
                .map_err(|e| SkillError::StorageError(e.to_string()))?;

            debug!("Created skills collection with dimension {}", dimension);
        }

        Ok(Self {
            state_store,
            vector_store,
            embedder,
            collection_name,
        })
    }

    /// Create a registry with a custom collection name.
    pub async fn with_collection(
        state_store: Arc<S>,
        vector_store: Arc<V>,
        embedder: Arc<E>,
        collection_name: impl Into<String>,
    ) -> Result<Self, SkillError> {
        let collection_name = collection_name.into();
        let dimension = embedder.dimension() as u64;

        if !vector_store
            .collection_exists(&collection_name)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?
        {
            vector_store
                .create_collection(&collection_name, dimension)
                .await
                .map_err(|e| SkillError::StorageError(e.to_string()))?;
        }

        Ok(Self {
            state_store,
            vector_store,
            embedder,
            collection_name,
        })
    }

    /// Register a new skill.
    ///
    /// This stores the skill in Redis and its embedding in Qdrant.
    #[instrument(skip(self, skill), fields(skill_id = %skill.id, skill_name = %skill.name))]
    pub async fn register(&self, skill: Skill) -> Result<(), SkillError> {
        let key = format!("{}{}", SKILL_KEY_PREFIX, skill.id);

        // Check if skill already exists
        if self
            .state_store
            .exists(&key)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?
        {
            return Err(SkillError::AlreadyExists(skill.id));
        }

        // Generate embedding
        let embedding_text = skill.embedding_text();
        let vector = self
            .embedder
            .embed(&embedding_text)
            .await
            .map_err(|e| SkillError::EmbeddingError(e.to_string()))?;

        // Store skill in Redis
        self.state_store
            .set(&key, &skill, None)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?;

        // Store embedding in Qdrant
        let mut payload = HashMap::new();
        payload.insert("skill_id".to_string(), serde_json::json!(skill.id));
        payload.insert("name".to_string(), serde_json::json!(skill.name));
        payload.insert("enabled".to_string(), serde_json::json!(skill.enabled));

        let point = VectorPoint {
            id: skill.id.clone(),
            vector,
            payload,
        };

        self.vector_store
            .upsert(&self.collection_name, vec![point])
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?;

        debug!("Registered skill: {}", skill.name);
        Ok(())
    }

    /// Update an existing skill.
    #[instrument(skip(self, skill), fields(skill_id = %skill.id))]
    pub async fn update(&self, mut skill: Skill) -> Result<(), SkillError> {
        let key = format!("{}{}", SKILL_KEY_PREFIX, skill.id);

        // Check if skill exists
        if !self
            .state_store
            .exists(&key)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?
        {
            return Err(SkillError::NotFound(skill.id));
        }

        // Update timestamp
        skill.updated_at = chrono::Utc::now();

        // Regenerate embedding
        let embedding_text = skill.embedding_text();
        let vector = self
            .embedder
            .embed(&embedding_text)
            .await
            .map_err(|e| SkillError::EmbeddingError(e.to_string()))?;

        // Update in Redis
        self.state_store
            .set(&key, &skill, None)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?;

        // Update embedding in Qdrant
        let mut payload = HashMap::new();
        payload.insert("skill_id".to_string(), serde_json::json!(skill.id));
        payload.insert("name".to_string(), serde_json::json!(skill.name));
        payload.insert("enabled".to_string(), serde_json::json!(skill.enabled));

        let point = VectorPoint {
            id: skill.id.clone(),
            vector,
            payload,
        };

        self.vector_store
            .upsert(&self.collection_name, vec![point])
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?;

        debug!("Updated skill: {}", skill.name);
        Ok(())
    }

    /// Get a skill by ID.
    pub async fn get(&self, skill_id: &str) -> Result<Option<Skill>, SkillError> {
        let key = format!("{}{}", SKILL_KEY_PREFIX, skill_id);

        self.state_store
            .get(&key)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))
    }

    /// Delete a skill.
    #[instrument(skip(self), fields(skill_id = %skill_id))]
    pub async fn delete(&self, skill_id: &str) -> Result<(), SkillError> {
        let key = format!("{}{}", SKILL_KEY_PREFIX, skill_id);

        // Delete from Redis
        self.state_store
            .delete(&key)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?;

        // Delete from Qdrant
        self.vector_store
            .delete(&self.collection_name, vec![skill_id.to_string()])
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?;

        debug!("Deleted skill: {}", skill_id);
        Ok(())
    }

    /// Search for skills semantically.
    ///
    /// Returns skills that match the query, sorted by relevance.
    #[instrument(skip(self), fields(query = %query, limit = %limit))]
    pub async fn search(&self, query: &str, limit: u64) -> Result<Vec<SkillMatch>, SkillError> {
        self.search_with_threshold(query, limit, None).await
    }

    /// Search for skills with a minimum score threshold.
    pub async fn search_with_threshold(
        &self,
        query: &str,
        limit: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<SkillMatch>, SkillError> {
        // Generate query embedding
        let query_vector = self
            .embedder
            .embed(query)
            .await
            .map_err(|e| SkillError::EmbeddingError(e.to_string()))?;

        // Search in Qdrant
        let results = self
            .vector_store
            .search(&self.collection_name, query_vector, limit, score_threshold)
            .await
            .map_err(|e| SkillError::StorageError(e.to_string()))?;

        // Fetch full skills from Redis
        let mut matches = Vec::with_capacity(results.len());

        for result in results {
            if let Some(skill) = self.get(&result.id).await? {
                // Only include enabled skills
                if skill.enabled {
                    matches.push(SkillMatch::new(skill, result.score));
                }
            }
        }

        debug!("Found {} matching skills for query", matches.len());
        Ok(matches)
    }

    /// Record a successful skill execution.
    pub async fn record_success(
        &self,
        skill_id: &str,
        execution_ms: u64,
    ) -> Result<(), SkillError> {
        if let Some(mut skill) = self.get(skill_id).await? {
            skill.stats.record_success(execution_ms);
            self.update(skill).await?;
        }
        Ok(())
    }

    /// Record a failed skill execution.
    pub async fn record_failure(
        &self,
        skill_id: &str,
        execution_ms: u64,
    ) -> Result<(), SkillError> {
        if let Some(mut skill) = self.get(skill_id).await? {
            skill.stats.record_failure(execution_ms);
            self.update(skill).await?;
        }
        Ok(())
    }

    /// Get skills that need improvement (low success rate).
    pub async fn get_skills_needing_improvement(
        &self,
        threshold: f64,
    ) -> Result<Vec<Skill>, SkillError> {
        // This is a simplified implementation that would need pagination
        // for large skill sets. In production, you'd want to:
        // 1. Store success rates in Qdrant payload for filtering
        // 2. Or use Redis sorted sets for efficient querying
        //
        // For now, we return an empty vec (caller should implement their own logic)
        debug!("Looking for skills with success rate < {}", threshold);
        Ok(vec![])
    }
}

/// Trait for skill registry operations.
///
/// This allows for easier testing and alternative implementations.
#[async_trait]
pub trait SkillRegistryTrait: Send + Sync {
    /// Register a skill.
    async fn register(&self, skill: Skill) -> Result<(), SkillError>;

    /// Get a skill by ID.
    async fn get(&self, skill_id: &str) -> Result<Option<Skill>, SkillError>;

    /// Delete a skill.
    async fn delete(&self, skill_id: &str) -> Result<(), SkillError>;

    /// Search for skills.
    async fn search(&self, query: &str, limit: u64) -> Result<Vec<SkillMatch>, SkillError>;
}

#[async_trait]
impl<S, V, E> SkillRegistryTrait for SkillRegistry<S, V, E>
where
    S: StateStore,
    V: VectorStore,
    E: Embedder,
{
    async fn register(&self, skill: Skill) -> Result<(), SkillError> {
        self.register(skill).await
    }

    async fn get(&self, skill_id: &str) -> Result<Option<Skill>, SkillError> {
        self.get(skill_id).await
    }

    async fn delete(&self, skill_id: &str) -> Result<(), SkillError> {
        self.delete(skill_id).await
    }

    async fn search(&self, query: &str, limit: u64) -> Result<Vec<SkillMatch>, SkillError> {
        self.search(query, limit).await
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would go here with mock implementations
    // of StateStore, VectorStore, and Embedder
}
