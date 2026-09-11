//! Firecracker microVM management

use gateway_core::sandbox::{
    CreateSandboxRequest, ExecuteRequest, ExecuteResponse, Runtime, SandboxId, SandboxStatus,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tokio::process::Child;

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

    /// Effective configuration, retained for future guest-agent integration.
    pub fn config(&self) -> &FirecrackerConfig {
        &self.config
    }

    /// VM boot/configuration is unavailable until the guest protocol is implemented.
    pub async fn create(
        &self,
        _request: CreateSandboxRequest,
    ) -> FirecrackerResult<FirecrackerInstance> {
        Err(FirecrackerError::ApiError(
            "Local VM provisioning and guest protocol are not implemented".into(),
        ))
    }

    /// Reject execution until a real guest transport is available.
    pub async fn execute(
        &self,
        _instance: &FirecrackerInstance,
        _request: ExecuteRequest,
    ) -> FirecrackerResult<ExecuteResponse> {
        Err(FirecrackerError::ApiError(
            "Local guest execution is not implemented".into(),
        ))
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
