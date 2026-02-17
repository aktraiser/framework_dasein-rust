//! Sandbox types for code execution

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Sandbox identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SandboxId(String);

impl SandboxId {
    pub fn new() -> Self {
        Self(format!("sb-{}", Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SandboxId {
    fn default() -> Self {
        Self::new()
    }
}

/// Sandbox runtime type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Python,
    Node,
    TypeScript,
    Rust,
    Go,
    Bash,
}

/// Request to create a sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSandboxRequest {
    pub runtime: Runtime,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
}

/// Response from sandbox creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSandboxResponse {
    pub sandbox_id: SandboxId,
    pub status: SandboxStatus,
}

/// Sandbox status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxStatus {
    Creating,
    Ready,
    Running,
    Stopped,
    Failed,
}

/// Request to execute code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRequest {
    /// Runtime to use for execution
    #[serde(default = "default_runtime")]
    pub runtime: Runtime,
    /// Code to execute
    pub code: String,
    /// Timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// Standard input
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
}

fn default_runtime() -> Runtime {
    Runtime::Python
}

/// Response from code execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// File upload/download request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRequest {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}
