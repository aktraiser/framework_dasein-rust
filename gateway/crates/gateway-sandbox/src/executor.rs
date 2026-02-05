//! Code execution interface

use crate::firecracker::FirecrackerResult;
use crate::pool::SandboxPool;
use gateway_core::sandbox::{
    CreateSandboxRequest, ExecuteResponse, Runtime, SandboxId,
};
use std::sync::Arc;
use tracing::{info, instrument};

/// High-level code executor
pub struct CodeExecutor {
    pool: Arc<SandboxPool>,
}

impl CodeExecutor {
    pub fn new(pool: Arc<SandboxPool>) -> Self {
        Self { pool }
    }

    /// Execute code with automatic sandbox management
    #[instrument(skip(self, code))]
    pub async fn execute(
        &self,
        runtime: Runtime,
        code: String,
        packages: Vec<String>,
        timeout_seconds: Option<u32>,
    ) -> FirecrackerResult<ExecuteResponse> {
        // Acquire sandbox
        let sandbox_id = self
            .pool
            .acquire(CreateSandboxRequest {
                runtime,
                packages,
                timeout_seconds,
                memory_mb: None,
            })
            .await?;

        info!(sandbox_id = %sandbox_id.as_str(), "Executing code");

        // Execute code
        let result = self.execute_in_sandbox(&sandbox_id, code, timeout_seconds).await;

        // Release sandbox
        self.pool.release(&sandbox_id).await?;

        result
    }

    async fn execute_in_sandbox(
        &self,
        _sandbox_id: &SandboxId,
        code: String,
        _timeout_seconds: Option<u32>,
    ) -> FirecrackerResult<ExecuteResponse> {
        // In a real implementation, this would:
        // 1. Get the sandbox instance from the pool
        // 2. Call the firecracker manager's execute method
        // 3. Handle timeouts

        let start = std::time::Instant::now();

        // Placeholder - would actually execute via Firecracker
        let response = ExecuteResponse {
            stdout: format!("Executed {} bytes of code", code.len()),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: start.elapsed().as_millis() as u64,
            error: None,
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
