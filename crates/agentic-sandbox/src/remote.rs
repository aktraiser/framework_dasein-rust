//! Remote sandbox that delegates execution to a remote Firecracker server.
//!
//! This sandbox connects to a remote server (e.g., Hetzner with KVM) that runs
//! Firecracker microVMs for secure code execution.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐       HTTPS        ┌─────────────────────┐
//! │  Your Machine   │  ───────────────►  │  Remote Server      │
//! │  (macOS/Win)    │                    │  (Linux + KVM)      │
//! │                 │                    │                     │
//! │  RemoteSandbox  │  ◄───────────────  │  Firecracker API    │
//! └─────────────────┘      Response      └─────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use agentic_sandbox::{RemoteSandbox, Sandbox};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let sandbox = RemoteSandbox::new("http://65.108.230.227:8080")
//!         .api_key("your-secret-key")
//!         .timeout_ms(30000)
//!         .build();
//!
//!     let result = sandbox.execute("echo 'Hello from Firecracker!'").await?;
//!     println!("Output: {}", result.stdout);
//!
//!     Ok(())
//! }
//! ```

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, instrument, warn};

use crate::{
    error::SandboxError,
    traits::{ExecutionResult, Sandbox, SandboxArtifact},
};

/// Request sent to the remote Firecracker server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRequest {
    /// Code/command to execute
    pub code: String,
    /// Language (optional, for proper execution)
    pub language: Option<String>,
    /// Timeout in milliseconds
    pub timeout_ms: u64,
}

/// Response from the remote Firecracker server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResponse {
    /// Whether execution succeeded
    pub success: bool,
    /// Exit code
    pub exit_code: i32,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
    /// Generated artifacts
    #[serde(default)]
    pub artifacts: Vec<ArtifactResponse>,
    /// Error message if any
    pub error: Option<String>,
}

/// Artifact in the response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactResponse {
    pub path: String,
    pub content: String,
}

/// Health check response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub firecracker_available: bool,
    pub kvm_available: bool,
    pub version: String,
}

/// Configuration for RemoteSandbox.
#[derive(Debug, Clone)]
pub struct RemoteSandboxConfig {
    /// Base URL of the remote server (e.g., "http://65.108.230.227:8080")
    pub base_url: String,
    /// API key for authentication
    pub api_key: Option<String>,
    /// Request timeout in milliseconds
    pub timeout_ms: u64,
    /// Default language for execution
    pub default_language: Option<String>,
    /// Number of retries on failure
    pub max_retries: u32,
}

impl Default for RemoteSandboxConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8080".into(),
            api_key: None,
            timeout_ms: 30000,
            default_language: None,
            max_retries: 2,
        }
    }
}

/// Remote sandbox that delegates to a Firecracker server.
///
/// Use this when you want to execute code in Firecracker microVMs
/// but your local machine doesn't support KVM (e.g., macOS, Windows).
#[derive(Clone)]
pub struct RemoteSandbox {
    config: RemoteSandboxConfig,
    client: Client,
}

impl RemoteSandbox {
    /// Create a new RemoteSandbox builder.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The URL of the remote Firecracker server
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// let sandbox = RemoteSandbox::new("http://65.108.230.227:8080")
    ///     .api_key("secret")
    ///     .build();
    /// ```
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> RemoteSandboxBuilder {
        RemoteSandboxBuilder::new(base_url.into())
    }

    /// Create with full config.
    #[must_use]
    pub fn with_config(config: RemoteSandboxConfig) -> Self {
        // HTTP client timeout should be longer than execution timeout
        // to account for network latency and server processing
        let http_timeout = config.timeout_ms + 30_000; // Add 30s buffer
        let client = Client::builder()
            .timeout(Duration::from_millis(http_timeout))
            .build()
            .expect("Failed to create HTTP client");

        Self { config, client }
    }

    /// Get the base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Check server health.
    pub async fn health(&self) -> Result<HealthResponse, SandboxError> {
        let url = format!("{}/health", self.config.base_url);

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.config.api_key {
            request = request.header("X-API-Key", key);
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
            .json::<HealthResponse>()
            .await
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))
    }

    /// Execute code with retry logic.
    async fn execute_with_retry(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
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
        let url = format!("{}/execute", self.config.base_url);

        let request_body = ExecuteRequest {
            code: code.to_string(),
            language: self.config.default_language.clone(),
            timeout_ms: self.config.timeout_ms,
        };

        let mut request = self.client.post(&url).json(&request_body);

        if let Some(ref key) = self.config.api_key {
            request = request.header("X-API-Key", key);
        }

        let response = request
            .send()
            .await
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SandboxError::ConnectionFailed("Invalid API key".into()));
        }

        if response.status() == reqwest::StatusCode::REQUEST_TIMEOUT {
            return Err(SandboxError::Timeout);
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SandboxError::ExecutionFailed(format!(
                "Server error {}: {}",
                status, body
            )));
        }

        let exec_response: ExecuteResponse = response
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
            execution_time_ms: exec_response.execution_time_ms,
            artifacts,
        })
    }
}

impl std::fmt::Debug for RemoteSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSandbox")
            .field("base_url", &self.config.base_url)
            .field("timeout_ms", &self.config.timeout_ms)
            .finish()
    }
}

#[async_trait]
impl Sandbox for RemoteSandbox {
    #[instrument(skip(self, code), fields(sandbox = "remote", url = %self.config.base_url))]
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        info!("Executing code on remote Firecracker server");
        debug!("Code: {}...", &code[..code.len().min(100)]);

        self.execute_with_retry(code).await
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        match self.health().await {
            Ok(health) => Ok(health.firecracker_available),
            Err(_) => Ok(false),
        }
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        // Remote sandbox doesn't need cleanup on client side
        Ok(())
    }
}

/// Builder for RemoteSandbox.
pub struct RemoteSandboxBuilder {
    config: RemoteSandboxConfig,
}

impl RemoteSandboxBuilder {
    fn new(base_url: String) -> Self {
        Self {
            config: RemoteSandboxConfig {
                base_url,
                ..Default::default()
            },
        }
    }

    /// Set API key for authentication.
    #[must_use]
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.config.api_key = Some(key.into());
        self
    }

    /// Set request timeout in milliseconds.
    #[must_use]
    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.config.timeout_ms = ms;
        self
    }

    /// Set default language.
    #[must_use]
    pub fn language(mut self, lang: impl Into<String>) -> Self {
        self.config.default_language = Some(lang.into());
        self
    }

    /// Set max retries.
    #[must_use]
    pub fn max_retries(mut self, n: u32) -> Self {
        self.config.max_retries = n;
        self
    }

    /// Build the RemoteSandbox.
    #[must_use]
    pub fn build(self) -> RemoteSandbox {
        RemoteSandbox::with_config(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder() {
        let sandbox = RemoteSandbox::new("http://localhost:8080")
            .api_key("test-key")
            .timeout_ms(5000)
            .language("rust")
            .max_retries(3)
            .build();

        assert_eq!(sandbox.config.base_url, "http://localhost:8080");
        assert_eq!(sandbox.config.api_key, Some("test-key".into()));
        assert_eq!(sandbox.config.timeout_ms, 5000);
        assert_eq!(sandbox.config.default_language, Some("rust".into()));
        assert_eq!(sandbox.config.max_retries, 3);
    }

    #[test]
    fn test_default_config() {
        let config = RemoteSandboxConfig::default();
        assert_eq!(config.base_url, "http://localhost:8080");
        assert!(config.api_key.is_none());
        assert_eq!(config.timeout_ms, 30000);
    }
}
