//! Code execution interface

use crate::firecracker::FirecrackerResult;
use crate::pool::SandboxPool;
use crate::remote::{FileInfo, RemoteSandbox, RemoteSandboxConfig};
use gateway_core::sandbox::{ExecuteResponse, Runtime};
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
            ExecutionBackend::Remote(config) => match RemoteSandbox::new(config.clone()) {
                Ok(r) => {
                    info!(base_url = %config.base_url, "Using remote Firecracker backend");
                    Some(r)
                }
                Err(e) => {
                    warn!("Failed to create remote sandbox client: {}", e);
                    None
                }
            },
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

    /// Configured execution backend.
    pub fn backend(&self) -> &ExecutionBackend {
        &self.backend
    }

    /// Configured pool for future local guest execution.
    pub fn pool(&self) -> &Arc<SandboxPool> {
        &self.pool
    }

    /// Check if the execution backend is ready.
    pub async fn is_ready(&self) -> bool {
        match &self.remote {
            Some(remote) => remote.is_ready().await.unwrap_or(false),
            None => false, // Local guest execution protocol is not implemented.
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
        if !packages.is_empty() {
            return Err(crate::firecracker::FirecrackerError::ApiError(
                "Package installation is not implemented for remote execution".into(),
            ));
        }
        let remote =
            self.remote.as_ref().ok_or_else(|| {
                crate::firecracker::FirecrackerError::ApiError(
            "Local guest execution is not implemented; configure a validated remote backend".into())
            })?;
        remote
            .execute_runtime(
                &runtime,
                &code,
                timeout_seconds.map(|s| u64::from(s) * 1000),
            )
            .await
            .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
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
    pub async fn execute_raw(
        &self,
        code: &str,
        timeout_ms: Option<u64>,
    ) -> FirecrackerResult<ExecuteResponse> {
        if let Some(ref remote) = self.remote {
            remote
                .execute(code, timeout_ms)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
            ))
        }
    }

    // ==================== Filesystem API ====================

    /// Write a file to the sandbox
    pub async fn write_file(&self, path: &str, content: &[u8]) -> FirecrackerResult<()> {
        if let Some(ref remote) = self.remote {
            remote
                .write_file(path, content)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
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
            remote
                .read_file(path)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
            ))
        }
    }

    /// Read a file as text
    pub async fn read_file_text(&self, path: &str) -> FirecrackerResult<String> {
        if let Some(ref remote) = self.remote {
            remote
                .read_file_text(path)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
            ))
        }
    }

    /// List files in a directory
    pub async fn list_files(&self, path: &str) -> FirecrackerResult<Vec<FileInfo>> {
        if let Some(ref remote) = self.remote {
            remote
                .list_files(path)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
            ))
        }
    }

    /// Delete a file or directory
    pub async fn delete_file(&self, path: &str, recursive: bool) -> FirecrackerResult<()> {
        if let Some(ref remote) = self.remote {
            remote
                .delete(path, recursive)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
            ))
        }
    }

    /// Check if a file exists
    pub async fn file_exists(&self, path: &str) -> FirecrackerResult<bool> {
        if let Some(ref remote) = self.remote {
            remote
                .exists(path)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
            ))
        }
    }

    /// Create a directory
    pub async fn mkdir(&self, path: &str) -> FirecrackerResult<()> {
        if let Some(ref remote) = self.remote {
            remote
                .mkdir(path)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
            ))
        }
    }

    /// Get file information
    pub async fn stat(&self, path: &str) -> FirecrackerResult<FileInfo> {
        if let Some(ref remote) = self.remote {
            remote
                .stat(path)
                .await
                .map_err(|e| crate::firecracker::FirecrackerError::ApiError(e.to_string()))
        } else {
            Err(crate::firecracker::FirecrackerError::ApiError(
                "Remote backend not configured".to_string(),
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

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn local_execution_cannot_return_fake_success() {
        let pool = std::sync::Arc::new(crate::pool::SandboxPool::new(
            Default::default(),
            Default::default(),
        ));
        let executor = super::CodeExecutor::new(pool);
        assert!(!executor.is_ready().await);
        assert!(executor
            .execute_oneshot(
                gateway_core::sandbox::Runtime::Python,
                "print(1)".into(),
                None
            )
            .await
            .is_err());
    }
}
