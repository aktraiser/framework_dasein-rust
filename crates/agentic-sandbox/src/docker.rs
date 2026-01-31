//! Docker-based sandbox execution.
//!
//! Provides isolated code execution using Docker containers.
//!
//! # Example
//!
//! ```rust,no_run
//! use agentic_sandbox::{Sandbox, DockerSandbox};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let sandbox = DockerSandbox::new()
//!         .image("rust:1.75")
//!         .build();
//!
//!     let result = sandbox.execute("rustc --version").await?;
//!     println!("Output: {}", result.stdout);
//!
//!     Ok(())
//! }
//! ```

use crate::error::SandboxError;
use crate::traits::{ExecutionResult, Sandbox};
use async_trait::async_trait;

/// Docker sandbox for isolated code execution.
#[derive(Debug, Clone)]
pub struct DockerSandbox {
    image: String,
    timeout_ms: u64,
    memory_limit: Option<String>,
    cpu_limit: Option<f64>,
}

impl DockerSandbox {
    /// Create a new Docker sandbox builder.
    pub fn new() -> DockerSandboxBuilder {
        DockerSandboxBuilder::default()
    }
}

impl Default for DockerSandbox {
    fn default() -> Self {
        Self {
            image: "rust:1.75-slim".to_string(),
            timeout_ms: 30_000,
            memory_limit: Some("512m".to_string()),
            cpu_limit: Some(1.0),
        }
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    async fn execute(&self, _code: &str) -> Result<ExecutionResult, SandboxError> {
        // TODO: Implement Docker execution
        // For now, return an error indicating Docker is not yet implemented
        Err(SandboxError::ExecutionFailed(
            "Docker sandbox not yet implemented. Use ProcessSandbox or RemoteSandbox.".to_string(),
        ))
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        // TODO: Check if Docker is available
        Ok(false)
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        // TODO: Stop any running containers
        Ok(())
    }
}

/// Builder for DockerSandbox.
#[derive(Debug, Default)]
pub struct DockerSandboxBuilder {
    image: Option<String>,
    timeout_ms: Option<u64>,
    memory_limit: Option<String>,
    cpu_limit: Option<f64>,
}

impl DockerSandboxBuilder {
    /// Set the Docker image to use.
    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.image = Some(image.into());
        self
    }

    /// Set the execution timeout in milliseconds.
    pub fn timeout_ms(mut self, timeout: u64) -> Self {
        self.timeout_ms = Some(timeout);
        self
    }

    /// Set the memory limit (e.g., "512m", "1g").
    pub fn memory_limit(mut self, limit: impl Into<String>) -> Self {
        self.memory_limit = Some(limit.into());
        self
    }

    /// Set the CPU limit (e.g., 1.0 = 1 CPU core).
    pub fn cpu_limit(mut self, limit: f64) -> Self {
        self.cpu_limit = Some(limit);
        self
    }

    /// Build the DockerSandbox.
    pub fn build(self) -> DockerSandbox {
        DockerSandbox {
            image: self.image.unwrap_or_else(|| "rust:1.75-slim".to_string()),
            timeout_ms: self.timeout_ms.unwrap_or(30_000),
            memory_limit: self.memory_limit.or(Some("512m".to_string())),
            cpu_limit: self.cpu_limit.or(Some(1.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_sandbox_builder() {
        let sandbox = DockerSandbox::new()
            .image("node:18")
            .timeout_ms(60_000)
            .memory_limit("1g")
            .cpu_limit(2.0)
            .build();

        assert_eq!(sandbox.image, "node:18");
        assert_eq!(sandbox.timeout_ms, 60_000);
        assert_eq!(sandbox.memory_limit, Some("1g".to_string()));
        assert_eq!(sandbox.cpu_limit, Some(2.0));
    }

    #[test]
    fn test_docker_sandbox_defaults() {
        let sandbox = DockerSandbox::default();
        assert_eq!(sandbox.image, "rust:1.75-slim");
        assert_eq!(sandbox.timeout_ms, 30_000);
    }
}
