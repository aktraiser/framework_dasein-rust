//! API routes

pub mod chat;
pub mod embeddings;
pub mod sandbox;
pub mod health;

use axum::{
    routing::{get, post},
    Router,
};
use crate::state::AppState;

/// Create the main API router
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // OpenAI-compatible endpoints
        .route("/v1/chat/completions", post(chat::chat_completions))
        .route("/v1/embeddings", post(embeddings::create_embeddings))
        .route("/v1/models", get(chat::list_models))
        // Sandbox endpoints
        .route("/v1/sandboxes", post(sandbox::create_sandbox))
        .route("/v1/sandboxes/:id", get(sandbox::get_sandbox))
        .route("/v1/sandboxes/:id/execute", post(sandbox::execute_code))
        .route("/v1/sandboxes/:id", axum::routing::delete(sandbox::delete_sandbox))
        // Health endpoints
        .route("/health", get(health::health_check))
        .route("/metrics", get(health::metrics))
        .with_state(state)
}
