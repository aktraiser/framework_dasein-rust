//! Chat completion API

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use gateway_core::llm::{ChatCompletionRequest, ChatCompletionResponse};
use serde::{Deserialize, Serialize};
use crate::state::AppState;

/// POST /v1/chat/completions
pub async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletionResponse>, ApiError> {
    // Get fallback providers for this model
    let fallbacks = state
        .config
        .llm
        .models
        .get(&request.model)
        .map(|m| m.fallbacks.clone())
        .unwrap_or_default();

    // Route the request
    let response = state
        .llm_router
        .chat_completion(request, &fallbacks)
        .await
        .map_err(|e| ApiError::Provider(e.to_string()))?;

    Ok(Json(response))
}

/// GET /v1/models
pub async fn list_models(State(state): State<AppState>) -> Json<ModelsResponse> {
    let models: Vec<ModelInfo> = state
        .config
        .llm
        .models
        .keys()
        .map(|name| ModelInfo {
            id: name.clone(),
            object: "model".to_string(),
            owned_by: "agentic-gateway".to_string(),
        })
        .collect();

    Json(ModelsResponse {
        object: "list".to_string(),
        data: models,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: String,
    pub owned_by: String,
}

/// API error type
#[derive(Debug)]
pub enum ApiError {
    Provider(String),
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Provider(msg) => (StatusCode::BAD_GATEWAY, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        let body = serde_json::json!({
            "error": {
                "message": message,
                "type": "api_error",
            }
        });

        (status, Json(body)).into_response()
    }
}
