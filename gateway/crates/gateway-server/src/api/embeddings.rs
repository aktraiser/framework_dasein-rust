//! Embeddings API

use axum::{extract::State, Json};
use gateway_core::llm::{EmbeddingRequest, EmbeddingResponse};
use crate::api::chat::ApiError;
use crate::state::AppState;

/// POST /v1/embeddings
pub async fn create_embeddings(
    State(_state): State<AppState>,
    Json(request): Json<EmbeddingRequest>,
) -> Result<Json<EmbeddingResponse>, ApiError> {
    // TODO: Route to appropriate embedding provider
    Err(ApiError::Internal(format!(
        "Embeddings not yet implemented for model: {}",
        request.model
    )))
}
