//! Embeddings API

use crate::api::chat::ApiError;
use crate::state::AppState;
use axum::{extract::State, Json};
use gateway_core::llm::{EmbeddingRequest, EmbeddingResponse};

/// POST /v1/embeddings
pub async fn create_embeddings(
    State(state): State<AppState>,
    Json(request): Json<EmbeddingRequest>,
) -> Result<Json<EmbeddingResponse>, ApiError> {
    let response = state
        .runtime_router()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .embeddings(request)
        .await
        .map_err(|e| ApiError::Provider(e.to_string()))?;
    Ok(Json(response))
}
