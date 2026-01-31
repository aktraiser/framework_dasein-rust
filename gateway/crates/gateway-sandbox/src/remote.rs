//! Remote Firecracker sandbox via HTTP API
//!
//! Connects to a remote Firecracker VM running fc-agent

use gateway_core::sandbox::{ExecuteResponse, Runtime};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use tracing::{debug, info, instrument};

#[derive(Debug, Error)]
pub enum RemoteSandboxError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Timeout after {0}ms")]
    Timeout(u64),

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type RemoteResult<T> = Result<T, RemoteSandboxError>;

/// Configuration for remote Firecracker sandbox
#[derive(Debug, Clone)]
pub struct RemoteSandboxConfig {
    /// Base URL of the fc-agent (e.g., "http://65.108.230.227:8080")
    pub base_url: String,
    /// Optional API key for authentication
    pub api_key: Option<String>,
    /// Default timeout in milliseconds
    pub default_timeout_ms: u64,
    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,
}

impl Default for RemoteSandboxConfig {
    fn default() -> Self {
        Self {
            base_url: std::env::var("FIRECRACKER_REMOTE_URL")
                .unwrap_or_else(|_| "http://65.108.230.227:8080".to_string()),
            api_key: std::env::var("FIRECRACKER_API_KEY").ok(),
            default_timeout_ms: 30000,
            connect_timeout_secs: 10,
        }
    }
}

impl RemoteSandboxConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            ..Default::default()
        }
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.default_timeout_ms = timeout_ms;
        self
    }
}

/// fc-agent health response
#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(default)]
    pub firecracker_available: bool,
    #[serde(default)]
    pub kvm_available: bool,
    #[serde(default)]
    pub version: Option<String>,
}

/// fc-agent execute request
#[derive(Debug, Serialize)]
pub struct ExecuteRequest {
    pub code: String,
    pub timeout_ms: u64,
}

/// fc-agent execute response
#[derive(Debug, Deserialize)]
pub struct AgentExecuteResponse {
    pub success: bool,
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// Remote sandbox client
pub struct RemoteSandbox {
    config: RemoteSandboxConfig,
    client: Client,
}

impl RemoteSandbox {
    /// Create a new remote sandbox client
    pub fn new(config: RemoteSandboxConfig) -> RemoteResult<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.connect_timeout_secs + 60))
            .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
            .build()?;

        info!(base_url = %config.base_url, "Created remote sandbox client");

        Ok(Self { config, client })
    }

    /// Create with default configuration
    pub fn with_defaults() -> RemoteResult<Self> {
        Self::new(RemoteSandboxConfig::default())
    }

    /// Check if the remote server is healthy
    #[instrument(skip(self))]
    pub async fn is_ready(&self) -> RemoteResult<bool> {
        let url = format!("{}/health", self.config.base_url);
        debug!(url = %url, "Checking health");

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Ok(false);
        }

        let health: HealthResponse = response.json().await?;
        debug!(?health, "Health check response");

        Ok(health.status == "healthy")
    }

    /// Get health details
    #[instrument(skip(self))]
    pub async fn health(&self) -> RemoteResult<HealthResponse> {
        let url = format!("{}/health", self.config.base_url);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(RemoteSandboxError::ServerError(format!(
                "Health check failed: {}",
                response.status()
            )));
        }

        Ok(response.json().await?)
    }

    /// Execute code on the remote Firecracker VM
    #[instrument(skip(self, code), fields(code_len = code.len()))]
    pub async fn execute(
        &self,
        code: &str,
        timeout_ms: Option<u64>,
    ) -> RemoteResult<ExecuteResponse> {
        let url = format!("{}/execute", self.config.base_url);
        let timeout = timeout_ms.unwrap_or(self.config.default_timeout_ms);

        debug!(url = %url, timeout_ms = timeout, "Executing code");

        let request = ExecuteRequest {
            code: code.to_string(),
            timeout_ms: timeout,
        };

        // Debug: log the exact code being sent
        debug!(
            code_preview = %code.chars().take(200).collect::<String>(),
            code_has_backslash_bang = %code.contains("\\!"),
            "Sending code to fc-agent"
        );

        let start = std::time::Instant::now();

        let mut req = self.client.post(&url).json(&request);

        // Add API key if configured
        if let Some(ref api_key) = self.config.api_key {
            req = req.header("X-API-Key", api_key);
        }

        let response = req.send().await.map_err(|e| {
            if e.is_timeout() {
                RemoteSandboxError::Timeout(timeout)
            } else if e.is_connect() {
                RemoteSandboxError::ConnectionFailed(e.to_string())
            } else {
                RemoteSandboxError::Http(e)
            }
        })?;

        if !response.status().is_success() {
            return Err(RemoteSandboxError::ServerError(format!(
                "Execute failed: {}",
                response.status()
            )));
        }

        let agent_response: AgentExecuteResponse = response.json().await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        debug!(
            success = agent_response.success,
            exit_code = agent_response.exit_code,
            duration_ms = duration_ms,
            "Execution complete"
        );

        if !agent_response.success {
            if let Some(error) = &agent_response.error {
                if error == "timeout" {
                    return Err(RemoteSandboxError::Timeout(timeout));
                }
                return Err(RemoteSandboxError::ExecutionFailed(error.clone()));
            }
        }

        Ok(ExecuteResponse {
            stdout: agent_response.stdout,
            stderr: agent_response.stderr,
            exit_code: agent_response.exit_code,
            duration_ms,
            error: agent_response.error,
        })
    }

    /// Execute code with a specific runtime
    ///
    /// Wraps the code with the appropriate interpreter command
    #[instrument(skip(self, code), fields(runtime = ?runtime, code_len = code.len()))]
    pub async fn execute_runtime(
        &self,
        runtime: &Runtime,
        code: &str,
        timeout_ms: Option<u64>,
    ) -> RemoteResult<ExecuteResponse> {
        debug!(
            original_code = %code,
            has_backslash_bang = %code.contains("\\!"),
            "Before wrapping"
        );
        let wrapped_code = self.wrap_code_for_runtime(runtime, code);
        debug!(
            wrapped_preview = %wrapped_code.chars().take(300).collect::<String>(),
            "After wrapping"
        );
        self.execute(&wrapped_code, timeout_ms).await
    }

    /// Wrap code with the appropriate interpreter
    fn wrap_code_for_runtime(&self, runtime: &Runtime, code: &str) -> String {
        match runtime {
            Runtime::Python => {
                // Use heredoc to handle multi-line code safely
                format!("python3 << 'PYTHON_EOF'\n{}\nPYTHON_EOF", code)
            }
            Runtime::Node | Runtime::TypeScript => {
                format!("node << 'NODE_EOF'\n{}\nNODE_EOF", code)
            }
            Runtime::Go => {
                // For Go, we need to write to a file and run
                // Set required env vars and use full path
                format!(
                    r#"cat > /tmp/main.go << 'GO_EOF'
{}
GO_EOF
export HOME=/root
export GOCACHE=/tmp/go-cache
mkdir -p $GOCACHE
cd /tmp && /usr/local/go/bin/go run main.go"#,
                    code
                )
            }
            Runtime::Rust => {
                // For Rust, write and compile
                // Use full path since rustc might not be in PATH
                format!(
                    r#"cat > /tmp/main.rs << 'RUST_EOF'
{}
RUST_EOF
/root/.cargo/bin/rustc /tmp/main.rs -o /tmp/main && /tmp/main"#,
                    code
                )
            }
            Runtime::Bash => code.to_string(),
        }
    }

    // ==================== Filesystem API ====================

    /// Write a file to the sandbox
    #[instrument(skip(self, content), fields(path = %path, content_len = content.len()))]
    pub async fn write_file(&self, path: &str, content: &[u8]) -> RemoteResult<()> {
        let encoded = BASE64.encode(content);

        // Create parent directory and write file
        let code = format!(
            "mkdir -p \"$(dirname '{}')\" && echo '{}' | base64 -d > '{}'",
            path, encoded, path
        );

        let result = self.execute(&code, None).await?;

        if result.exit_code != 0 {
            return Err(RemoteSandboxError::ExecutionFailed(format!(
                "Failed to write file: {}",
                result.stderr
            )));
        }

        debug!(path = %path, "File written successfully");
        Ok(())
    }

    /// Write text content to a file
    pub async fn write_file_text(&self, path: &str, content: &str) -> RemoteResult<()> {
        self.write_file(path, content.as_bytes()).await
    }

    /// Read a file from the sandbox
    #[instrument(skip(self), fields(path = %path))]
    pub async fn read_file(&self, path: &str) -> RemoteResult<Vec<u8>> {
        let code = format!("base64 '{}'", path);

        let result = self.execute(&code, None).await?;

        if result.exit_code != 0 {
            return Err(RemoteSandboxError::ExecutionFailed(format!(
                "Failed to read file: {}",
                result.stderr
            )));
        }

        // Decode base64 content
        let decoded = BASE64.decode(result.stdout.trim())
            .map_err(|e| RemoteSandboxError::ExecutionFailed(format!("Base64 decode error: {}", e)))?;

        debug!(path = %path, size = decoded.len(), "File read successfully");
        Ok(decoded)
    }

    /// Read a file as text
    pub async fn read_file_text(&self, path: &str) -> RemoteResult<String> {
        let content = self.read_file(path).await?;
        String::from_utf8(content)
            .map_err(|e| RemoteSandboxError::ExecutionFailed(format!("UTF-8 decode error: {}", e)))
    }

    /// Delete a file or directory
    #[instrument(skip(self), fields(path = %path))]
    pub async fn delete(&self, path: &str, recursive: bool) -> RemoteResult<()> {
        let code = if recursive {
            format!("rm -rf '{}'", path)
        } else {
            format!("rm -f '{}'", path)
        };

        let result = self.execute(&code, None).await?;

        if result.exit_code != 0 {
            return Err(RemoteSandboxError::ExecutionFailed(format!(
                "Failed to delete: {}",
                result.stderr
            )));
        }

        debug!(path = %path, "Deleted successfully");
        Ok(())
    }

    /// List files in a directory
    #[instrument(skip(self), fields(path = %path))]
    pub async fn list_files(&self, path: &str) -> RemoteResult<Vec<FileInfo>> {
        // Use ls with specific format for easy parsing
        // Format: type permissions size mtime name
        let code = format!(
            "ls -la '{}' 2>/dev/null | tail -n +2 | awk '{{print $1, $5, $6, $7, $8, $9}}'",
            path
        );

        let result = self.execute(&code, None).await?;

        if result.exit_code != 0 {
            return Err(RemoteSandboxError::ExecutionFailed(format!(
                "Failed to list files: {}",
                result.stderr
            )));
        }

        let files: Vec<FileInfo> = result.stdout
            .lines()
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(6, ' ').collect();
                if parts.len() >= 6 {
                    let perms = parts[0];
                    let is_dir = perms.starts_with('d');
                    let size: u64 = parts[1].parse().unwrap_or(0);
                    let name = parts[5].to_string();

                    // Skip . and ..
                    if name == "." || name == ".." {
                        return None;
                    }

                    Some(FileInfo {
                        name,
                        path: format!("{}/{}", path.trim_end_matches('/'), parts[5]),
                        is_dir,
                        size,
                    })
                } else {
                    None
                }
            })
            .collect();

        debug!(path = %path, count = files.len(), "Listed files");
        Ok(files)
    }

    /// Check if a file or directory exists
    #[instrument(skip(self), fields(path = %path))]
    pub async fn exists(&self, path: &str) -> RemoteResult<bool> {
        let code = format!("test -e '{}' && echo 'exists' || echo 'not_found'", path);

        let result = self.execute(&code, None).await?;
        Ok(result.stdout.trim() == "exists")
    }

    /// Create a directory
    #[instrument(skip(self), fields(path = %path))]
    pub async fn mkdir(&self, path: &str) -> RemoteResult<()> {
        let code = format!("mkdir -p '{}'", path);

        let result = self.execute(&code, None).await?;

        if result.exit_code != 0 {
            return Err(RemoteSandboxError::ExecutionFailed(format!(
                "Failed to create directory: {}",
                result.stderr
            )));
        }

        debug!(path = %path, "Directory created");
        Ok(())
    }

    /// Get file information
    #[instrument(skip(self), fields(path = %path))]
    pub async fn stat(&self, path: &str) -> RemoteResult<FileInfo> {
        let code = format!(
            "stat -c '%F %s %n' '{}' 2>/dev/null || stat -f '%HT %z %N' '{}'",
            path, path
        );

        let result = self.execute(&code, None).await?;

        if result.exit_code != 0 {
            return Err(RemoteSandboxError::ExecutionFailed(format!(
                "File not found: {}",
                path
            )));
        }

        let parts: Vec<&str> = result.stdout.trim().splitn(3, ' ').collect();
        if parts.len() >= 3 {
            let is_dir = parts[0].contains("directory") || parts[0].contains("Directory");
            let size: u64 = parts[1].parse().unwrap_or(0);
            let name = std::path::Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            Ok(FileInfo {
                name,
                path: path.to_string(),
                is_dir,
                size,
            })
        } else {
            Err(RemoteSandboxError::ExecutionFailed("Failed to parse stat output".to_string()))
        }
    }
}

/// File information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Builder pattern for RemoteSandbox
pub struct RemoteSandboxBuilder {
    config: RemoteSandboxConfig,
}

impl RemoteSandboxBuilder {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            config: RemoteSandboxConfig::new(base_url),
        }
    }

    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.config.api_key = Some(api_key.into());
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.config.default_timeout_ms = timeout_ms;
        self
    }

    pub fn connect_timeout_secs(mut self, secs: u64) -> Self {
        self.config.connect_timeout_secs = secs;
        self
    }

    pub fn build(self) -> RemoteResult<RemoteSandbox> {
        RemoteSandbox::new(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_python() {
        let config = RemoteSandboxConfig::default();
        let sandbox = RemoteSandbox::new(config).unwrap();

        let code = "print('hello')";
        let wrapped = sandbox.wrap_code_for_runtime(&Runtime::Python, code);
        assert!(wrapped.contains("python3"));
        assert!(wrapped.contains("PYTHON_EOF"));
    }

    #[test]
    fn test_wrap_rust() {
        let config = RemoteSandboxConfig::default();
        let sandbox = RemoteSandbox::new(config).unwrap();

        let code = "fn main() { println!(\"hello\"); }";
        let wrapped = sandbox.wrap_code_for_runtime(&Runtime::Rust, code);
        assert!(wrapped.contains("rustc"));
        assert!(wrapped.contains("RUST_EOF"));
    }
}
