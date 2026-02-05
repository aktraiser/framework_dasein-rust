//! Firecracker microVM management

use gateway_core::sandbox::{
    CreateSandboxRequest, ExecuteRequest, ExecuteResponse, Runtime,
    SandboxId, SandboxStatus,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use thiserror::Error;
use tokio::process::{Child, Command};
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum FirecrackerError {
    #[error("Failed to start Firecracker: {0}")]
    StartFailed(String),

    #[error("API request failed: {0}")]
    ApiError(String),

    #[error("Sandbox not found: {0}")]
    NotFound(String),

    #[error("Execution timeout")]
    Timeout,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type FirecrackerResult<T> = Result<T, FirecrackerError>;

/// Firecracker configuration
#[derive(Debug, Clone)]
pub struct FirecrackerConfig {
    /// Path to firecracker binary
    pub binary_path: PathBuf,
    /// Path to kernel image
    pub kernel_path: PathBuf,
    /// Path to root filesystem
    pub rootfs_path: PathBuf,
    /// Socket directory for API
    pub socket_dir: PathBuf,
    /// Default memory in MB
    pub default_memory_mb: u32,
    /// Default vCPUs
    pub default_vcpus: u32,
}

impl Default for FirecrackerConfig {
    fn default() -> Self {
        Self {
            binary_path: PathBuf::from("/usr/bin/firecracker"),
            kernel_path: PathBuf::from("/var/lib/firecracker/vmlinux"),
            rootfs_path: PathBuf::from("/var/lib/firecracker/rootfs.ext4"),
            socket_dir: PathBuf::from("/tmp/firecracker"),
            default_memory_mb: 512,
            default_vcpus: 1,
        }
    }
}

/// Firecracker VM instance
pub struct FirecrackerInstance {
    pub id: SandboxId,
    pub runtime: Runtime,
    pub status: SandboxStatus,
    pub socket_path: PathBuf,
    process: Option<Child>,
}

impl FirecrackerInstance {
    pub fn new(id: SandboxId, runtime: Runtime, socket_path: PathBuf) -> Self {
        Self {
            id,
            runtime,
            status: SandboxStatus::Creating,
            socket_path,
            process: None,
        }
    }

    pub async fn stop(&mut self) -> FirecrackerResult<()> {
        if let Some(ref mut process) = self.process {
            process.kill().await?;
        }
        self.status = SandboxStatus::Stopped;
        Ok(())
    }
}

/// Firecracker manager
pub struct FirecrackerManager {
    config: FirecrackerConfig,
}

impl FirecrackerManager {
    pub fn new(config: FirecrackerConfig) -> Self {
        Self { config }
    }

    /// Create a new microVM instance
    pub async fn create(
        &self,
        request: CreateSandboxRequest,
    ) -> FirecrackerResult<FirecrackerInstance> {
        let id = SandboxId::new();
        let socket_path = self.config.socket_dir.join(format!("{}.sock", id.as_str()));

        info!(sandbox_id = %id.as_str(), runtime = ?request.runtime, "Creating Firecracker instance");

        // Ensure socket directory exists
        tokio::fs::create_dir_all(&self.config.socket_dir).await?;

        // Remove existing socket if present
        let _ = tokio::fs::remove_file(&socket_path).await;

        let mut instance = FirecrackerInstance::new(id.clone(), request.runtime.clone(), socket_path.clone());

        // Start Firecracker process
        let process = Command::new(&self.config.binary_path)
            .arg("--api-sock")
            .arg(&socket_path)
            .arg("--log-level")
            .arg("Warning")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| FirecrackerError::StartFailed(e.to_string()))?;

        instance.process = Some(process);

        // Wait for socket to be available
        self.wait_for_socket(&socket_path).await?;

        // Configure the VM
        self.configure_vm(&socket_path, &request).await?;

        instance.status = SandboxStatus::Ready;
        info!(sandbox_id = %id.as_str(), "Firecracker instance ready");

        Ok(instance)
    }

    async fn wait_for_socket(&self, socket_path: &PathBuf) -> FirecrackerResult<()> {
        for _ in 0..50 {
            if socket_path.exists() {
                return Ok(());
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        Err(FirecrackerError::StartFailed(
            "Socket not available after timeout".to_string(),
        ))
    }

    async fn configure_vm(
        &self,
        _socket_path: &PathBuf,
        request: &CreateSandboxRequest,
    ) -> FirecrackerResult<()> {
        let memory_mb = request.memory_mb.unwrap_or(self.config.default_memory_mb);

        debug!(
            kernel = %self.config.kernel_path.display(),
            rootfs = %self.config.rootfs_path.display(),
            memory_mb = memory_mb,
            "Configuring VM"
        );

        // In a real implementation, we would:
        // 1. PUT /boot-source with kernel config
        // 2. PUT /drives/rootfs with rootfs config
        // 3. PUT /machine-config with CPU/memory
        // 4. PUT /actions with InstanceStart

        // For now, this is a placeholder that would use HTTP over Unix socket
        // to configure Firecracker via its API

        Ok(())
    }

    /// Execute code in a sandbox
    pub async fn execute(
        &self,
        instance: &FirecrackerInstance,
        request: ExecuteRequest,
    ) -> FirecrackerResult<ExecuteResponse> {
        if instance.status != SandboxStatus::Ready {
            return Err(FirecrackerError::ApiError(format!(
                "Sandbox not ready: {:?}",
                instance.status
            )));
        }

        debug!(
            sandbox_id = %instance.id.as_str(),
            code_len = request.code.len(),
            "Executing code"
        );

        let start = std::time::Instant::now();

        // In a real implementation, we would:
        // 1. Write code to a file in the VM via vsock or SSH
        // 2. Execute the appropriate interpreter based on runtime
        // 3. Capture stdout/stderr
        // 4. Return results

        // Placeholder response
        let response = ExecuteResponse {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: start.elapsed().as_millis() as u64,
            error: None,
        };

        Ok(response)
    }
}

/// Firecracker API types
#[derive(Debug, Serialize, Deserialize)]
pub struct BootSource {
    pub kernel_image_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_args: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Drive {
    pub drive_id: String,
    pub path_on_host: String,
    pub is_root_device: bool,
    pub is_read_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MachineConfig {
    pub vcpu_count: u32,
    pub mem_size_mib: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstanceActionInfo {
    pub action_type: String,
}
