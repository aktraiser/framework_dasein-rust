//! Embeddings API

use crate::api::chat::{record_usage, reserve_tokens, ApiError};
use crate::middleware::auth::{Principal, TokenLimit};
use crate::state::AppState;
use axum::{extract::State, Extension, Json};
use gateway_core::llm::{EmbeddingRequest, EmbeddingResponse};

/// POST /v1/embeddings
pub async fn create_embeddings(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    limit: Option<Extension<TokenLimit>>,
    Json(request): Json<EmbeddingRequest>,
) -> Result<Json<EmbeddingResponse>, ApiError> {
    let started = std::time::Instant::now();
    let model = request.model.clone();
    let principal = principal.map(|Extension(principal)| principal);
    // Embeddings consume their input, at about four characters per token.
    let input_chars = match &request.input {
        gateway_core::llm::EmbeddingInput::Single(text) => text.chars().count(),
        gateway_core::llm::EmbeddingInput::Batch(texts) => {
            texts.iter().map(|text| text.chars().count()).sum()
        }
    };
    let estimate = u32::try_from(input_chars.div_ceil(4)).unwrap_or(u32::MAX);
    let mut reservation = reserve_tokens(
        &state,
        principal.as_ref(),
        limit.map(|Extension(limit)| limit),
        estimate,
    )
    .await?;
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
        principal.as_ref(),
        &provider,
        &model,
        Some(&response.usage),
        started,
        reservation.take(),
    )
    .await;
    Ok(Json(response))
}
