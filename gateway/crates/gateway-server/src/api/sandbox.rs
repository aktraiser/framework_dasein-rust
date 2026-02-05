//! Sandbox API

use axum::{
    extract::{Path, State},
    Json,
};
use gateway_core::sandbox::{
    CreateSandboxRequest, CreateSandboxResponse, ExecuteRequest, ExecuteResponse, Runtime, SandboxId,
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

    // Check if remote backend is ready
    let backend_ready = state.code_executor.is_ready().await;

    Ok(Json(SandboxInfo {
        id,
        status: SandboxStatus::Ready,
        backend_ready,
        pool_stats: PoolStatsResponse {
            available: stats.available,
            in_use: stats.in_use,
            max_total: stats.max_total,
        },
    }))
}

/// POST /v1/sandboxes/:id/execute
pub async fn execute_code(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    // Execute code using the code executor (remote or local)
    let response = state
        .code_executor
        .execute(
            request.runtime.clone(),
            request.code,
            vec![],
            request.timeout_seconds,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(response))
}

/// POST /v1/execute - Direct code execution without sandbox management
pub async fn execute_direct(
    State(state): State<AppState>,
    Json(request): Json<DirectExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    let response = state
        .code_executor
        .execute(
            request.runtime,
            request.code,
            request.packages.unwrap_or_default(),
            request.timeout_seconds,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(response))
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
    pub backend_ready: bool,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct DirectExecuteRequest {
    pub runtime: Runtime,
    pub code: String,
    #[serde(default)]
    pub packages: Option<Vec<String>>,
    pub timeout_seconds: Option<u32>,
}
