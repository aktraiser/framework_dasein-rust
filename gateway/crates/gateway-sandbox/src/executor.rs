//! Code execution interface

use crate::firecracker::FirecrackerResult;
use crate::pool::SandboxPool;
use crate::remote::{RemoteSandbox, RemoteSandboxConfig, FileInfo};
use gateway_core::sandbox::{
    CreateSandboxRequest, ExecuteResponse, Runtime, SandboxId,
};
use std::sync::Arc;
use tracing::{info, instrument, warn};

/// Execution backend selection
#[derive(Debug, Clone)]
pub enum ExecutionBackend {
    /// Local Firecracker VMs (not yet implemented)
    Local,
    /// Remote Firecracker via HTTP API
    Remote(RemoteSandboxConfig),
}

impl Default for ExecutionBackend {
    fn default() -> Self {
        // Check if remote URL is configured
        if std::env::var("FIRECRACKER_REMOTE_URL").is_ok() {
            ExecutionBackend::Remote(RemoteSandboxConfig::default())
        } else {
            ExecutionBackend::Local
        }
    }
}

/// High-level code executor
pub struct CodeExecutor {
    pool: Arc<SandboxPool>,
    backend: ExecutionBackend,
    remote: Option<RemoteSandbox>,
}

impl CodeExecutor {
    pub fn new(pool: Arc<SandboxPool>) -> Self {
        Self::with_backend(pool, ExecutionBackend::default())
    }

    pub fn with_backend(pool: Arc<SandboxPool>, backend: ExecutionBackend) -> Self {
        let remote = match &backend {
            ExecutionBackend::Remote(config) => {
                match RemoteSandbox::new(config.clone()) {
                    Ok(r) => {
                        info!(base_url = %config.base_url, "Using remote Firecracker backend");
                        Some(r)
                    }
                    Err(e) => {
                        warn!("Failed to create remote sandbox client: {}", e);
                        None
                    }
                }
            }
            ExecutionBackend::Local => None,
        };

        Self {
            pool,
            backend,
            remote,
        }
    }

    /// Create executor with remote backend
    pub fn with_remote(pool: Arc<SandboxPool>, config: RemoteSandboxConfig) -> Self {
        Self::with_backend(pool, ExecutionBackend::Remote(config))
    }

    /// Check if the execution backend is ready
    pub async fn is_ready(&self) -> bool {
        match &self.remote {
            Some(remote) => remote.is_ready().await.unwrap_or(false),
            None => true, // Local always "ready" (placeholder)
        }
    }

    /// Execute code with automatic sandbox management
    #[instrument(skip(self, code), fields(runtime = ?runtime, code_len = code.len()))]
    pub async fn execute(
        &self,
        runtime: Runtime,
        code: String,
        packages: Vec<String>,
        timeout_seconds: Option<u32>,
    ) -> FirecrackerResult<ExecuteResponse> {
        // If remote backend is available, use it directly
        if let Some(ref remote) = self.remote {
            let timeout_ms = timeout_seconds.map(|s| s as u64 * 1000);

            match remote.execute_runtime(&runtime, &code, timeout_ms).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    warn!("Remote execution failed: {}, falling back to local", e);
                    // Fall through to local execution
                }
            }
        }

        // Local execution (with pool)
        let sandbox_id = self
            .pool
            .acquire(CreateSandboxRequest {
                runtime: runtime.clone(),
                packages,
                timeout_seconds,
                memory_mb: None,
            })
            .await?;

        info!(sandbox_id = %sandbox_id.as_str(), "Executing code locally");

        let result = self.execute_in_sandbox(&sandbox_id, code, timeout_seconds).await;

        self.pool.release(&sandbox_id).await?;

        result
    }

    async fn execute_in_sandbox(
        &self,
        _sandbox_id: &SandboxId,
        code: String,
        _timeout_seconds: Option<u32>,
    ) -> FirecrackerResult<ExecuteResponse> {
        // Local placeholder - would use Firecracker API over Unix socket
        let start = std::time::Instant::now();

        let response = ExecuteResponse {
            stdout: format!("[Local placeholder] Would execute {} bytes of code", code.len()),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: start.elapsed().as_millis() as u64,
            error: Some("Local execution not implemented - configure FIRECRACKER_REMOTE_URL".to_string()),
        };

        Ok(response)
    }

    /// Execute code without pooling (one-shot)
    pub async fn execute_oneshot(
        &self,
        runtime: Runtime,
        code: String,
        timeout_seconds: Option<u32>,
    ) -> FirecrackerResult<ExecuteResponse> {
        self.execute(runtime, code, vec![], timeout_seconds).await
    }

    /// Execute raw shell command on remote
    pub async fn execute_raw(&self, code: &str, timeout_ms: Option<u64>) -> FirecrackerResult<ExecuteResponse> {
        if let Some(ref remote) = self.remote {
            remote.execute(code, timeout_ms).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }

    // ==================== Filesystem API ====================

    /// Write a file to the sandbox
    pub async fn write_file(&self, path: &str, content: &[u8]) -> FirecrackerResult<()> {
        if let Some(ref remote) = self.remote {
            remote.write_file(path, content).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }

    /// Write text to a file
    pub async fn write_file_text(&self, path: &str, content: &str) -> FirecrackerResult<()> {
        self.write_file(path, content.as_bytes()).await
    }

    /// Read a file from the sandbox
    pub async fn read_file(&self, path: &str) -> FirecrackerResult<Vec<u8>> {
        if let Some(ref remote) = self.remote {
            remote.read_file(path).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }

    /// Read a file as text
    pub async fn read_file_text(&self, path: &str) -> FirecrackerResult<String> {
        if let Some(ref remote) = self.remote {
            remote.read_file_text(path).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }

    /// List files in a directory
    pub async fn list_files(&self, path: &str) -> FirecrackerResult<Vec<FileInfo>> {
        if let Some(ref remote) = self.remote {
            remote.list_files(path).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }

    /// Delete a file or directory
    pub async fn delete_file(&self, path: &str, recursive: bool) -> FirecrackerResult<()> {
        if let Some(ref remote) = self.remote {
            remote.delete(path, recursive).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }

    /// Check if a file exists
    pub async fn file_exists(&self, path: &str) -> FirecrackerResult<bool> {
        if let Some(ref remote) = self.remote {
            remote.exists(path).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }

    /// Create a directory
    pub async fn mkdir(&self, path: &str) -> FirecrackerResult<()> {
        if let Some(ref remote) = self.remote {
            remote.mkdir(path).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }

    /// Get file information
    pub async fn stat(&self, path: &str) -> FirecrackerResult<FileInfo> {
        if let Some(ref remote) = self.remote {
            remote.stat(path).await.map_err(|e| {
                crate::firecracker::FirecrackerError::ApiError(e.to_string())
            })
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string()
            ))
        }
    }
}

/// Builder for code execution requests
pub struct ExecutionBuilder {
    runtime: Runtime,
    code: String,
    packages: Vec<String>,
    timeout: Option<u32>,
    stdin: Option<String>,
}

impl ExecutionBuilder {
    pub fn new(runtime: Runtime, code: impl Into<String>) -> Self {
        Self {
            runtime,
            code: code.into(),
            packages: vec![],
            timeout: None,
            stdin: None,
        }
    }

    pub fn packages(mut self, packages: Vec<String>) -> Self {
        self.packages = packages;
        self
    }

    pub fn timeout(mut self, seconds: u32) -> Self {
        self.timeout = Some(seconds);
        self
    }

    pub fn stdin(mut self, input: impl Into<String>) -> Self {
        self.stdin = Some(input.into());
        self
    }

    pub async fn run(self, executor: &CodeExecutor) -> FirecrackerResult<ExecuteResponse> {
        executor
            .execute(self.runtime, self.code, self.packages, self.timeout)
            .await
    }
}
