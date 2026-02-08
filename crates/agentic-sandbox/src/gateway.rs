//! Gateway sandbox that delegates execution to the Agentic Gateway.
//!
//! This sandbox connects to a running gateway instance which manages
//! Firecracker microVMs for secure code execution.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐       HTTPS        ┌─────────────────────┐
//! │  agentic-rs     │  ───────────────►  │  Agentic Gateway    │
//! │                 │                    │                     │
//! │  GatewaySandbox │  ◄───────────────  │  ┌───────────────┐  │
//! │                 │      Response      │  │ Firecracker   │  │
//! │  - Session mgmt │                    │  │ Pool (warm)   │  │
//! │  - Auto cleanup │                    │  └───────────────┘  │
//! └─────────────────┘                    └─────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use agentic_sandbox::{GatewaySandbox, Sandbox};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let sandbox = GatewaySandbox::builder("http://localhost:8080")
//!         .runtime("python")
//!         .build()
//!         .await?;
//!
//!     let result = sandbox.execute("print('Hello from Firecracker!')").await?;
//!     println!("Output: {}", result.stdout);
//!
//!     // Sandbox session is automatically cleaned up on drop
//!     Ok(())
//! }
//! ```

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

use crate::{
    error::SandboxError,
    traits::{ExecutionResult, Sandbox, SandboxArtifact},
};

/// Supported runtimes in the gateway.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    #[default]
    Python,
    Node,
    Rust,
    Bash,
}

impl std::fmt::Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Python => write!(f, "python"),
            Self::Node => write!(f, "node"),
            Self::Rust => write!(f, "rust"),
            Self::Bash => write!(f, "bash"),
        }
    }
}

/// Request to create a sandbox session.
#[derive(Debug, Clone, Serialize)]
struct CreateSandboxRequest {
    runtime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
}

/// Response from creating a sandbox.
#[derive(Debug, Clone, Deserialize)]
struct CreateSandboxResponse {
    id: String,
    #[allow(dead_code)]
    status: String,
}

/// Request to execute code in a sandbox.
#[derive(Debug, Clone, Serialize)]
struct ExecuteCodeRequest {
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
}

/// Response from code execution.
#[derive(Debug, Clone, Deserialize)]
struct ExecuteCodeResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
    duration_ms: u64,
    #[serde(default)]
    artifacts: Vec<ArtifactResponse>,
    error: Option<String>,
}

/// Artifact in the response.
#[derive(Debug, Clone, Deserialize)]
struct ArtifactResponse {
    path: String,
    content: String,
}

/// Health check response.
#[derive(Debug, Clone, Deserialize)]
pub struct GatewayHealthResponse {
    pub status: String,
    #[serde(default)]
    pub sandboxes_ready: u32,
    #[serde(default)]
    pub sandboxes_total: u32,
}

/// Configuration for `GatewaySandbox`.
#[derive(Debug, Clone)]
pub struct GatewaySandboxConfig {
    /// Base URL of the gateway (e.g., `http://localhost:8080`)
    pub base_url: String,
    /// API key for authentication (if required)
    pub api_key: Option<String>,
    /// Runtime to use
    pub runtime: Runtime,
    /// Memory in MB for the sandbox
    pub memory_mb: Option<u32>,
    /// Request timeout in milliseconds
    pub timeout_ms: u64,
    /// Number of retries on failure
    pub max_retries: u32,
}

impl Default for GatewaySandboxConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8080".into(),
            api_key: None,
            runtime: Runtime::Python,
            memory_mb: None,
            timeout_ms: 30000,
            max_retries: 2,
        }
    }
}

/// Internal state for the sandbox session.
#[derive(Debug)]
struct SandboxSession {
    id: String,
    base_url: String,
    api_key: Option<String>,
    client: Client,
}

impl SandboxSession {
    /// Delete the sandbox session.
    async fn cleanup(&self) -> Result<(), SandboxError> {
        let url = format!("{}/v1/sandboxes/{}", self.base_url, self.id);

        let mut request = self.client.delete(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => {
                info!(sandbox_id = %self.id, "Sandbox session cleaned up");
                Ok(())
            }
            Ok(response) => {
                warn!(
                    sandbox_id = %self.id,
                    status = %response.status(),
                    "Failed to cleanup sandbox"
                );
                Ok(()) // Don't fail on cleanup errors
            }
            Err(e) => {
                warn!(sandbox_id = %self.id, error = %e, "Failed to cleanup sandbox");
                Ok(()) // Don't fail on cleanup errors
            }
        }
    }
}

/// Gateway sandbox that uses the Agentic Gateway for Firecracker execution.
///
/// This sandbox manages a session with the gateway, automatically creating
/// and cleaning up sandbox instances.
pub struct GatewaySandbox {
    config: GatewaySandboxConfig,
    client: Client,
    session: Arc<RwLock<Option<SandboxSession>>>,
}

impl GatewaySandbox {
    /// Create a new `GatewaySandbox` builder.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The URL of the gateway server
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use agentic_sandbox::GatewaySandbox;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let sandbox = GatewaySandbox::builder("http://localhost:8080")
    ///     .runtime("python")
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn builder(base_url: impl Into<String>) -> GatewaySandboxBuilder {
        GatewaySandboxBuilder::new(base_url.into())
    }

    /// Create with full config (does not create session yet).
    fn with_config(config: GatewaySandboxConfig) -> Self {
        let http_timeout = config.timeout_ms + 30_000;
        let client = Client::builder()
            .timeout(Duration::from_millis(http_timeout))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            client,
            session: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the sandbox session with the gateway.
    pub async fn init(&self) -> Result<(), SandboxError> {
        let mut session = self.session.write().await;
        if session.is_some() {
            return Ok(()); // Already initialized
        }

        let url = format!("{}/v1/sandboxes", self.config.base_url);

        let request_body = CreateSandboxRequest {
            runtime: self.config.runtime.to_string(),
            memory_mb: self.config.memory_mb,
            timeout_ms: Some(self.config.timeout_ms),
        };

        let mut request = self.client.post(&url).json(&request_body);
        if let Some(ref key) = self.config.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }

        let response = request
            .send()
            .await
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SandboxError::ExecutionFailed(format!(
                "Failed to create sandbox: {status} - {body}"
            )));
        }

        let create_response: CreateSandboxResponse = response
            .json()
            .await
            .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;

        info!(
            sandbox_id = %create_response.id,
            runtime = %self.config.runtime,
            "Sandbox session created"
        );

        *session = Some(SandboxSession {
            id: create_response.id,
            base_url: self.config.base_url.clone(),
            api_key: self.config.api_key.clone(),
            client: self.client.clone(),
        });

        Ok(())
    }

    /// Get the sandbox session ID.
    pub async fn session_id(&self) -> Option<String> {
        self.session.read().await.as_ref().map(|s| s.id.clone())
    }

    /// Check gateway health.
    pub async fn health(&self) -> Result<GatewayHealthResponse, SandboxError> {
        let url = format!("{}/health", self.config.base_url);

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.config.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }

        let response = request
            .send()
            .await
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(SandboxError::ConnectionFailed(format!(
                "Health check failed: {}",
                response.status()
            )));
        }

        response
            .json::<GatewayHealthResponse>()
            .await
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))
    }

    /// Execute code with retry logic.
    async fn execute_with_retry(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        // Ensure session is initialized
        self.init().await?;

        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                debug!("Retry attempt {}/{}", attempt, self.config.max_retries);
                tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
            }

            match self.execute_once(code).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    warn!("Execution attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| SandboxError::ExecutionFailed("Unknown error".into())))
    }

    /// Execute code once (no retry).
    async fn execute_once(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        let session = self.session.read().await;
        let session = session
            .as_ref()
            .ok_or_else(|| SandboxError::ExecutionFailed("Session not initialized".into()))?;

        let url = format!(
            "{}/v1/sandboxes/{}/execute",
            self.config.base_url, session.id
        );

        let request_body = ExecuteCodeRequest {
            code: code.to_string(),
            timeout_ms: Some(self.config.timeout_ms),
        };

        let mut request = self.client.post(&url).json(&request_body);
        if let Some(ref key) = self.config.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }

        let response = request
            .send()
            .await
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SandboxError::ConnectionFailed("Invalid API key".into()));
        }

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SandboxError::ExecutionFailed(
                "Sandbox session expired or not found".into(),
            ));
        }

        if response.status() == reqwest::StatusCode::REQUEST_TIMEOUT {
            return Err(SandboxError::Timeout);
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SandboxError::ExecutionFailed(format!(
                "Server error {status}: {body}"
            )));
        }

        let exec_response: ExecuteCodeResponse = response
            .json()
            .await
            .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;

        if let Some(error) = exec_response.error {
            return Err(SandboxError::ExecutionFailed(error));
        }

        let artifacts = exec_response
            .artifacts
            .into_iter()
            .map(|a| SandboxArtifact {
                path: a.path,
                content: a.content,
            })
            .collect();

        Ok(ExecutionResult {
            exit_code: exec_response.exit_code,
            stdout: exec_response.stdout,
            stderr: exec_response.stderr,
            execution_time_ms: exec_response.duration_ms,
            artifacts,
        })
    }

    /// Cleanup the sandbox session.
    pub async fn cleanup(&self) -> Result<(), SandboxError> {
        let mut session = self.session.write().await;
        if let Some(s) = session.take() {
            s.cleanup().await?;
        }
        Ok(())
    }
}

impl Drop for GatewaySandbox {
    fn drop(&mut self) {
        // Schedule async cleanup
        let session = self.session.clone();
        tokio::spawn(async move {
            let mut guard = session.write().await;
            if let Some(s) = guard.take() {
                let _ = s.cleanup().await;
            }
        });
    }
}

impl std::fmt::Debug for GatewaySandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewaySandbox")
            .field("base_url", &self.config.base_url)
            .field("runtime", &self.config.runtime)
            .field("timeout_ms", &self.config.timeout_ms)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Sandbox for GatewaySandbox {
    #[instrument(skip(self, code), fields(sandbox = "gateway", url = %self.config.base_url))]
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        info!(runtime = %self.config.runtime, "Executing code via gateway");
        debug!("Code: {}...", &code[..code.len().min(100)]);

        self.execute_with_retry(code).await
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        match self.health().await {
            Ok(health) => Ok(health.sandboxes_ready > 0),
            Err(_) => Ok(false),
        }
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        self.cleanup().await
    }
}

/// Builder for `GatewaySandbox`.
pub struct GatewaySandboxBuilder {
    config: GatewaySandboxConfig,
    auto_init: bool,
}

impl GatewaySandboxBuilder {
    fn new(base_url: String) -> Self {
        Self {
            config: GatewaySandboxConfig {
                base_url,
                ..Default::default()
            },
            auto_init: true,
        }
    }

    /// Set API key for authentication.
    #[must_use]
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.config.api_key = Some(key.into());
        self
    }

    /// Set the runtime (python, node, rust, bash).
    #[must_use]
    pub fn runtime(mut self, runtime: impl Into<String>) -> Self {
        let runtime_str = runtime.into();
        self.config.runtime = match runtime_str.to_lowercase().as_str() {
            "python" => Runtime::Python,
            "node" | "nodejs" | "javascript" | "js" => Runtime::Node,
            "rust" => Runtime::Rust,
            "bash" | "shell" | "sh" => Runtime::Bash,
            _ => Runtime::Python, // Default
        };
        self
    }

    /// Set memory in MB.
    #[must_use]
    pub const fn memory_mb(mut self, mb: u32) -> Self {
        self.config.memory_mb = Some(mb);
        self
    }

    /// Set request timeout in milliseconds.
    #[must_use]
    pub const fn timeout_ms(mut self, ms: u64) -> Self {
        self.config.timeout_ms = ms;
        self
    }

    /// Set max retries.
    #[must_use]
    pub const fn max_retries(mut self, n: u32) -> Self {
        self.config.max_retries = n;
        self
    }

    /// Don't auto-initialize the session on build.
    #[must_use]
    pub const fn lazy(mut self) -> Self {
        self.auto_init = false;
        self
    }

    /// Build the `GatewaySandbox`.
    ///
    /// If `lazy()` was not called, this will create the sandbox session.
    ///
    /// # Errors
    ///
    /// Returns an error if auto-init is enabled and session creation fails.
    pub async fn build(self) -> Result<GatewaySandbox, SandboxError> {
        let sandbox = GatewaySandbox::with_config(self.config);

        if self.auto_init {
            sandbox.init().await?;
        }

        Ok(sandbox)
    }

    /// Build without initializing (sync version for lazy init).
    #[must_use]
    pub fn build_lazy(self) -> GatewaySandbox {
        GatewaySandbox::with_config(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_sync() {
        let sandbox = GatewaySandbox::builder("http://localhost:8080")
            .api_key("test-key")
            .runtime("python")
            .timeout_ms(5000)
            .max_retries(3)
            .build_lazy();

        assert_eq!(sandbox.config.base_url, "http://localhost:8080");
        assert_eq!(sandbox.config.api_key, Some("test-key".into()));
        assert_eq!(sandbox.config.timeout_ms, 5000);
        assert_eq!(sandbox.config.max_retries, 3);
    }

    #[test]
    fn test_runtime_parsing() {
        let sandbox = GatewaySandbox::builder("http://localhost:8080")
            .runtime("javascript")
            .build_lazy();

        assert!(matches!(sandbox.config.runtime, Runtime::Node));
    }

    #[test]
    fn test_default_config() {
        let config = GatewaySandboxConfig::default();
        assert_eq!(config.base_url, "http://localhost:8080");
        assert!(config.api_key.is_none());
        assert_eq!(config.timeout_ms, 30000);
    }
}
