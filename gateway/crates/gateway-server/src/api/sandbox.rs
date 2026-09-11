use crate::middleware::auth::Principal;
// Sandbox API

use crate::api::chat::ApiError;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use gateway_core::sandbox::{
    CreateSandboxRequest, CreateSandboxResponse, ExecuteRequest, ExecuteResponse, Runtime,
    SandboxStatus,
};
use serde::{Deserialize, Serialize};

/// POST /v1/sandboxes
pub async fn create_sandbox(
    State(state): State<AppState>,
    Extension(owner): Extension<Principal>,
    Json(request): Json<CreateSandboxRequest>,
) -> Result<Json<CreateSandboxResponse>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Box storage is not configured".into()))?;
    Ok(Json(state.boxes.create(storage, &owner.0, request).await?))
}

pub async fn get_sandbox(
    State(state): State<AppState>,
    Extension(owner): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Json<SandboxInfo>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Box storage is not configured".into()))?;
    let (status, backend_ready) = state.boxes.info(storage, &owner.0, &id).await?;
    let running = usize::from(status == SandboxStatus::Running);
    Ok(Json(SandboxInfo {
        id,
        status,
        backend_ready,
        pool_stats: PoolStatsResponse {
            available: usize::from(backend_ready) - running.min(usize::from(backend_ready)),
            in_use: running,
            max_total: 1,
        },
    }))
}

pub async fn execute_code(
    State(state): State<AppState>,
    Extension(owner): Extension<Principal>,
    Path(id): Path<String>,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Box storage is not configured".into()))?;
    Ok(Json(
        state.boxes.execute(storage, &owner.0, &id, request).await?,
    ))
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
    Extension(owner): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Box storage is not configured".into()))?;
    state.boxes.delete(storage, &owner.0, &id).await?;
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
