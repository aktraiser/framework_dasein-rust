//! Sandbox API

use axum::{
    extract::{Path, State},
    Json,
};
use gateway_core::sandbox::{
    CreateSandboxRequest, CreateSandboxResponse, ExecuteRequest, ExecuteResponse, SandboxId,
    SandboxStatus,
};
use serde::{Deserialize, Serialize};
use crate::api::chat::ApiError;
use crate::state::AppState;

/// POST /v1/sandboxes
pub async fn create_sandbox(
    State(state): State<AppState>,
    Json(request): Json<CreateSandboxRequest>,
) -> Result<Json<CreateSandboxResponse>, ApiError> {
    let sandbox_id = state
        .sandbox_pool
        .acquire(request)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CreateSandboxResponse {
        sandbox_id,
        status: SandboxStatus::Ready,
    }))
}

/// GET /v1/sandboxes/:id
pub async fn get_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SandboxInfo>, ApiError> {
    let stats = state.sandbox_pool.stats().await;

    Ok(Json(SandboxInfo {
        id,
        status: SandboxStatus::Ready,
        pool_stats: PoolStatsResponse {
            available: stats.available,
            in_use: stats.in_use,
            max_total: stats.max_total,
        },
    }))
}

/// POST /v1/sandboxes/:id/execute
pub async fn execute_code(
    State(_state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    // TODO: Execute via sandbox pool
    let _ = id;

    Ok(Json(ExecuteResponse {
        stdout: format!("Executed {} bytes", request.code.len()),
        stderr: String::new(),
        exit_code: 0,
        duration_ms: 0,
        error: None,
    }))
}

/// DELETE /v1/sandboxes/:id
pub async fn delete_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let sandbox_id = SandboxId::default(); // Would parse from id
    let _ = id;

    state
        .sandbox_pool
        .release(&sandbox_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(DeleteResponse { deleted: true }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub id: String,
    pub status: SandboxStatus,
    pub pool_stats: PoolStatsResponse,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PoolStatsResponse {
    pub available: usize,
    pub in_use: usize,
    pub max_total: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteResponse {
    pub deleted: bool,
}
