//! Qdrant vector store implementation.

use async_trait::async_trait;
use qdrant_client::{
    qdrant::{
        point_id::PointIdOptions, CreateCollectionBuilder, DeletePointsBuilder, Distance,
        PointStruct, SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
    },
    Payload, Qdrant,
};
use std::collections::HashMap;
use tracing::{debug, instrument};

use crate::{
    error::StorageError,
    traits::{VectorPoint, VectorSearchResult, VectorStore},
};

/// Qdrant vector store.
pub struct QdrantStore {
    client: Qdrant,
}

impl QdrantStore {
    /// Create a new Qdrant store.
    ///
    /// # Arguments
    ///
    /// * `url` - Qdrant server URL (e.g., `http://localhost:6334`)
    ///
    /// # Errors
    ///
    /// Returns an error if connection fails.
    pub fn new(url: &str) -> Result<Self, StorageError> {
        let client = Qdrant::from_url(url)
            .build()
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        Ok(Self { client })
    }

    /// Create with API key for cloud deployments.
    ///
    /// # Arguments
    ///
    /// * `url` - Qdrant cloud URL
    /// * `api_key` - API key for authentication
    ///
    /// # Errors
    ///
    /// Returns an error if connection fails.
    pub fn with_api_key(url: &str, api_key: &str) -> Result<Self, StorageError> {
        let client = Qdrant::from_url(url)
            .api_key(api_key)
            .build()
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        Ok(Self { client })
    }
}

#[async_trait]
impl VectorStore for QdrantStore {
    #[instrument(skip(self, points), fields(collection = %collection, point_count = points.len()))]
    async fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<(), StorageError> {
        debug!(
            "Upserting {} points to collection {}",
            points.len(),
            collection
        );

        let qdrant_points: Vec<PointStruct> = points
            .into_iter()
            .map(|p| {
                let payload: Payload = p
                    .payload
                    .into_iter()
                    .map(|(k, v)| (k, v.into()))
                    .collect::<HashMap<String, qdrant_client::qdrant::Value>>()
                    .into();

                PointStruct::new(p.id, p.vector, payload)
            })
            .collect();

        self.client
            .upsert_points(UpsertPointsBuilder::new(collection, qdrant_points).wait(true))
            .await
            .map_err(|e| StorageError::UpsertFailed(e.to_string()))?;

        Ok(())
    }

    #[instrument(skip(self, vector), fields(collection = %collection, limit = limit))]
    async fn search(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<VectorSearchResult>, StorageError> {
        debug!("Searching collection {} with limit {}", collection, limit);

        let mut search_builder =
            SearchPointsBuilder::new(collection, vector, limit).with_payload(true);

        if let Some(threshold) = score_threshold {
            search_builder = search_builder.score_threshold(threshold);
        }

        let response = self
            .client
            .search_points(search_builder)
            .await
            .map_err(|e| StorageError::SearchFailed(e.to_string()))?;

        let results = response
            .result
            .into_iter()
            .map(|scored| {
                let id = match scored.id {
                    Some(point_id) => match point_id.point_id_options {
                        Some(PointIdOptions::Uuid(uuid)) => uuid,
                        Some(PointIdOptions::Num(num)) => num.to_string(),
                        None => String::new(),
                    },
                    None => String::new(),
                };

                let payload: HashMap<String, serde_json::Value> = scored
                    .payload
                    .into_iter()
                    .map(|(k, v)| (k, qdrant_value_to_json(v)))
                    .collect();

                VectorSearchResult {
                    id,
                    score: scored.score,
                    payload,
                }
            })
            .collect();

        Ok(results)
    }

    #[instrument(skip(self, ids), fields(collection = %collection, id_count = ids.len()))]
    async fn delete(&self, collection: &str, ids: Vec<String>) -> Result<(), StorageError> {
        debug!(
            "Deleting {} points from collection {}",
            ids.len(),
            collection
        );

        let point_ids: Vec<qdrant_client::qdrant::PointId> = ids
            .into_iter()
            .map(|id| qdrant_client::qdrant::PointId {
                point_id_options: Some(PointIdOptions::Uuid(id)),
            })
            .collect();

        self.client
            .delete_points(
                DeletePointsBuilder::new(collection)
                    .points(point_ids)
                    .wait(true),
            )
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;

        Ok(())
    }

    #[instrument(skip(self), fields(collection = %name, vector_size = vector_size))]
    async fn create_collection(&self, name: &str, vector_size: u64) -> Result<(), StorageError> {
        debug!(
            "Creating collection {} with vector size {}",
            name, vector_size
        );

        self.client
            .create_collection(
                CreateCollectionBuilder::new(name)
                    .vectors_config(VectorParamsBuilder::new(vector_size, Distance::Cosine)),
            )
            .await
            .map_err(|e| StorageError::CreateFailed(e.to_string()))?;

        Ok(())
    }

    #[instrument(skip(self), fields(collection = %name))]
    async fn collection_exists(&self, name: &str) -> Result<bool, StorageError> {
        let exists = self
            .client
            .collection_exists(name)
            .await
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        Ok(exists)
    }
}

/// Convert Qdrant Value to `serde_json::Value`.
fn qdrant_value_to_json(value: qdrant_client::qdrant::Value) -> serde_json::Value {
    use qdrant_client::qdrant::value::Kind;

    match value.kind {
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(b),
        Some(Kind::IntegerValue(i)) => serde_json::Value::Number(i.into()),
        Some(Kind::DoubleValue(d)) => serde_json::Number::from_f64(d)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        Some(Kind::StringValue(s)) => serde_json::Value::String(s),
        Some(Kind::ListValue(list)) => {
            let arr: Vec<serde_json::Value> =
                list.values.into_iter().map(qdrant_value_to_json).collect();
            serde_json::Value::Array(arr)
        }
        Some(Kind::StructValue(s)) => {
            let obj: serde_json::Map<String, serde_json::Value> = s
                .fields
                .into_iter()
                .map(|(k, v)| (k, qdrant_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Some(Kind::NullValue(_)) | None => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qdrant_value_to_json() {
        use qdrant_client::qdrant::value::Kind;
        use qdrant_client::qdrant::Value;

        let value = Value {
            kind: Some(Kind::StringValue("test".to_string())),
        };
        let json = qdrant_value_to_json(value);
        assert_eq!(json, serde_json::Value::String("test".to_string()));

        let value = Value {
            kind: Some(Kind::IntegerValue(42)),
        };
        let json = qdrant_value_to_json(value);
        assert_eq!(json, serde_json::json!(42));
    }
}
