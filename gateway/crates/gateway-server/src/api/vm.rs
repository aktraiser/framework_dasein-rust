//! VM management API
//!
//! Endpoints to control the remote Firecracker VM

use axum::{extract::State, Json};
use gateway_sandbox::VmStatus;
use serde::{Deserialize, Serialize};

use crate::api::chat::ApiError;
use crate::state::AppState;

/// POST /v1/vm/start - Start the Firecracker VM
pub async fn start_vm(State(state): State<AppState>) -> Result<Json<VmActionResponse>, ApiError> {
    let orchestrator = state
        .orchestrator
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("VM orchestrator not configured. Set FIRECRACKER_SSH_PASSWORD.".to_string()))?;

    // Connect if not already connected
    orchestrator
        .connect()
        .await
        .map_err(|e| ApiError::Internal(format!("SSH connection failed: {}", e)))?;

    // Start VM
    orchestrator
        .start_vm()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to start VM: {}", e)))?;

    Ok(Json(VmActionResponse {
        success: true,
        message: "Firecracker VM started successfully".to_string(),
    }))
}

/// POST /v1/vm/stop - Stop the Firecracker VM
pub async fn stop_vm(State(state): State<AppState>) -> Result<Json<VmActionResponse>, ApiError> {
    let orchestrator = state
        .orchestrator
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("VM orchestrator not configured".to_string()))?;

    // Connect if not already connected
    orchestrator
        .connect()
        .await
        .map_err(|e| ApiError::Internal(format!("SSH connection failed: {}", e)))?;

    // Stop VM
    orchestrator
        .stop_vm()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to stop VM: {}", e)))?;

    Ok(Json(VmActionResponse {
        success: true,
        message: "Firecracker VM stopped".to_string(),
    }))
}

/// POST /v1/vm/restart - Restart the Firecracker VM
pub async fn restart_vm(State(state): State<AppState>) -> Result<Json<VmActionResponse>, ApiError> {
    let orchestrator = state
        .orchestrator
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("VM orchestrator not configured".to_string()))?;

    // Connect if not already connected
    orchestrator
        .connect()
        .await
        .map_err(|e| ApiError::Internal(format!("SSH connection failed: {}", e)))?;

    // Restart VM
    orchestrator
        .restart_vm()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to restart VM: {}", e)))?;

    Ok(Json(VmActionResponse {
        success: true,
        message: "Firecracker VM restarted successfully".to_string(),
    }))
}

/// GET /v1/vm/status - Get VM status
pub async fn vm_status(State(state): State<AppState>) -> Result<Json<VmStatusResponse>, ApiError> {
    let orchestrator = state
        .orchestrator
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("VM orchestrator not configured".to_string()))?;

    // Connect if not already connected
    orchestrator
        .connect()
        .await
        .map_err(|e| ApiError::Internal(format!("SSH connection failed: {}", e)))?;

    // Get status
    let status = orchestrator
        .status()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get status: {}", e)))?;

    Ok(Json(VmStatusResponse {
        status,
        orchestrator_enabled: true,
    }))
}

/// POST /v1/vm/ensure - Ensure VM is running (start if needed)
pub async fn ensure_vm(State(state): State<AppState>) -> Result<Json<VmActionResponse>, ApiError> {
    let orchestrator = state
        .orchestrator
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("VM orchestrator not configured".to_string()))?;

    // Connect if not already connected
    orchestrator
        .connect()
        .await
        .map_err(|e| ApiError::Internal(format!("SSH connection failed: {}", e)))?;

    // Ensure running
    orchestrator
        .ensure_running()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to ensure VM running: {}", e)))?;

    Ok(Json(VmActionResponse {
        success: true,
        message: "Firecracker VM is running".to_string(),
    }))
}

/// POST /v1/vm/exec - Execute SSH command on host server
pub async fn exec_ssh(
    State(state): State<AppState>,
    Json(request): Json<SshExecRequest>,
) -> Result<Json<SshExecResponse>, ApiError> {
    let orchestrator = state
        .orchestrator
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("VM orchestrator not configured".to_string()))?;

    // Connect if not already connected
    orchestrator
        .connect()
        .await
        .map_err(|e| ApiError::Internal(format!("SSH connection failed: {}", e)))?;

    // Execute command
    let (exit_code, stdout, stderr) = orchestrator
        .exec(&request.command)
        .await
        .map_err(|e| ApiError::Internal(format!("Command failed: {}", e)))?;

    Ok(Json(SshExecResponse {
        exit_code,
        stdout,
        stderr,
    }))
}

// ==================== Request/Response Types ====================

#[derive(Debug, Serialize)]
pub struct VmActionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct VmStatusResponse {
    pub status: VmStatus,
    pub orchestrator_enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct SshExecRequest {
    pub command: String,
}

#[derive(Debug, Serialize)]
pub struct SshExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
