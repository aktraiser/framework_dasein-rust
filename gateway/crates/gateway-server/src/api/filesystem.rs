//! Filesystem API for sandbox file operations

use axum::{
    extract::{State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gateway_sandbox::FileInfo;
use serde::{Deserialize, Serialize};

use crate::api::chat::ApiError;
use crate::state::AppState;

/// POST /v1/files - Upload a file
pub async fn upload_file(
    State(state): State<AppState>,
    Json(request): Json<UploadFileRequest>,
) -> Result<Json<UploadFileResponse>, ApiError> {
    // Decode base64 content if provided
    let content = if let Some(base64_content) = &request.content_base64 {
        BASE64.decode(base64_content)
            .map_err(|e| ApiError::BadRequest(format!("Invalid base64: {}", e)))?
    } else if let Some(text_content) = &request.content {
        text_content.as_bytes().to_vec()
    } else {
        return Err(ApiError::BadRequest("Either 'content' or 'content_base64' is required".to_string()));
    };

    state
        .code_executor
        .write_file(&request.path, &content)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(UploadFileResponse {
        path: request.path,
        size: content.len() as u64,
        success: true,
    }))
}

/// GET /v1/files - Download a file or list directory
pub async fn get_file(
    State(state): State<AppState>,
    Query(params): Query<GetFileParams>,
) -> Result<impl IntoResponse, ApiError> {
    let path = params.path.as_deref().unwrap_or("/");

    // Check if it's a directory
    let is_dir = state
        .code_executor
        .stat(path)
        .await
        .map(|info| info.is_dir)
        .unwrap_or(false);

    if is_dir || params.list.unwrap_or(false) {
        // List directory
        let files = state
            .code_executor
            .list_files(path)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        Ok(Json(ListFilesResponse {
            path: path.to_string(),
            files,
        }).into_response())
    } else {
        // Read file
        let content = state
            .code_executor
            .read_file(path)
            .await
            .map_err(|e| ApiError::NotFound(format!("File not found: {}", e)))?;

        let response = if params.raw.unwrap_or(false) {
            // Return raw content
            (
                StatusCode::OK,
                [("content-type", "application/octet-stream")],
                content,
            ).into_response()
        } else {
            // Return base64 encoded
            Json(ReadFileResponse {
                path: path.to_string(),
                content_base64: BASE64.encode(&content),
                size: content.len() as u64,
            }).into_response()
        };

        Ok(response)
    }
}

/// DELETE /v1/files - Delete a file or directory
pub async fn delete_file(
    State(state): State<AppState>,
    Query(params): Query<DeleteFileParams>,
) -> Result<Json<DeleteFileResponse>, ApiError> {
    let path = params.path.as_deref()
        .ok_or_else(|| ApiError::BadRequest("'path' parameter is required".to_string()))?;

    let recursive = params.recursive.unwrap_or(false);

    state
        .code_executor
        .delete_file(path, recursive)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(DeleteFileResponse {
        path: path.to_string(),
        deleted: true,
    }))
}

/// POST /v1/files/mkdir - Create a directory
pub async fn create_directory(
    State(state): State<AppState>,
    Json(request): Json<MkdirRequest>,
) -> Result<Json<MkdirResponse>, ApiError> {
    state
        .code_executor
        .mkdir(&request.path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MkdirResponse {
        path: request.path,
        created: true,
    }))
}

/// GET /v1/files/stat - Get file information
pub async fn stat_file(
    State(state): State<AppState>,
    Query(params): Query<StatFileParams>,
) -> Result<Json<FileInfo>, ApiError> {
    let path = params.path.as_deref()
        .ok_or_else(|| ApiError::BadRequest("'path' parameter is required".to_string()))?;

    let info = state
        .code_executor
        .stat(path)
        .await
        .map_err(|e| ApiError::NotFound(format!("File not found: {}", e)))?;

    Ok(Json(info))
}

/// GET /v1/files/exists - Check if file exists
pub async fn file_exists(
    State(state): State<AppState>,
    Query(params): Query<ExistsParams>,
) -> Result<Json<ExistsResponse>, ApiError> {
    let path = params.path.as_deref()
        .ok_or_else(|| ApiError::BadRequest("'path' parameter is required".to_string()))?;

    let exists = state
        .code_executor
        .file_exists(path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ExistsResponse {
        path: path.to_string(),
        exists,
    }))
}

// ==================== Request/Response Types ====================

#[derive(Debug, Deserialize)]
pub struct UploadFileRequest {
    pub path: String,
    /// Text content (for text files)
    pub content: Option<String>,
    /// Base64-encoded content (for binary files)
    pub content_base64: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UploadFileResponse {
    pub path: String,
    pub size: u64,
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct GetFileParams {
    pub path: Option<String>,
    /// If true, list directory contents instead of reading file
    pub list: Option<bool>,
    /// If true, return raw bytes instead of base64
    pub raw: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ReadFileResponse {
    pub path: String,
    pub content_base64: String,
    pub size: u64,
}

#[derive(Debug, Serialize)]
pub struct ListFilesResponse {
    pub path: String,
    pub files: Vec<FileInfo>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteFileParams {
    pub path: Option<String>,
    pub recursive: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DeleteFileResponse {
    pub path: String,
    pub deleted: bool,
}

#[derive(Debug, Deserialize)]
pub struct MkdirRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct MkdirResponse {
    pub path: String,
    pub created: bool,
}

#[derive(Debug, Deserialize)]
pub struct StatFileParams {
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExistsParams {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExistsResponse {
    pub path: String,
    pub exists: bool,
}
