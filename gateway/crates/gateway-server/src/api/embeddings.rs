//! Embeddings API

use crate::api::chat::{record_usage, ApiError};
use crate::middleware::auth::Principal;
use crate::state::AppState;
use axum::{extract::State, Extension, Json};
use gateway_core::llm::{EmbeddingRequest, EmbeddingResponse};

/// POST /v1/embeddings
pub async fn create_embeddings(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Json(request): Json<EmbeddingRequest>,
) -> Result<Json<EmbeddingResponse>, ApiError> {
    let started = std::time::Instant::now();
    let model = request.model.clone();
    let router = state
        .runtime_router()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let provider = router.provider_name(&model).unwrap_or_default();
    let response = router
        .embeddings(request)
        .await
        .map_err(|e| ApiError::Provider(e.to_string()))?;
    record_usage(
        &state,
        principal.as_ref().map(|Extension(principal)| principal),
        &provider,
        &model,
        Some(&response.usage),
        started,
    )
    .await;
    Ok(Json(response))
}
