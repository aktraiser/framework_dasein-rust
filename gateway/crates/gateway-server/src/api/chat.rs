//! Chat completion API

use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use gateway_core::llm::ChatCompletionRequest;
use serde::{Deserialize, Serialize};

/// POST /v1/chat/completions
pub async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, ApiError> {
    if request.stream.unwrap_or(false) {
        use axum::response::sse::{Event, KeepAlive, Sse};
        use futures::StreamExt;
        let stream = state
            .runtime_router()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .chat_completion_stream(request)
            .await
            .map_err(|e| ApiError::Provider(e.to_string()))?;
        let events = stream.map(|item| Ok::<_, std::convert::Infallible>(match item {
            Ok(data) => Event::default().data(data),
            Err(error) => Event::default().event("error").data(serde_json::json!({"error":{"message":error.to_string(),"type":"upstream_error"}}).to_string()),
        }));
        return Ok(Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response());
    }
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
        .runtime_router()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .chat_completion(request, &fallbacks)
        .await
        .map_err(|e| ApiError::Provider(e.to_string()))?;

    Ok(Json(response).into_response())
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
    Unavailable(String),
    ServiceUnavailable(String),
    Busy(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Busy(msg) => (StatusCode::TOO_MANY_REQUESTS, msg),
            ApiError::ServiceUnavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg),
            ApiError::Provider(msg) => (StatusCode::BAD_GATEWAY, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::Unavailable(msg) => (StatusCode::NOT_IMPLEMENTED, msg),
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
