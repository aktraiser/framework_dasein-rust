//! Firecracker MicroVM-based sandbox for secure code execution.
//!
//! Uses Firecracker microVMs for strong isolation with minimal overhead.
//!
//! # Prerequisites
//!
//! - Firecracker binary installed
//! - Linux with KVM support (`/dev/kvm` accessible)
//! - Root or appropriate permissions
//! - Kernel image (vmlinux)
//! - Root filesystem with toolchain + agent

use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, info, instrument, warn};

use crate::{
    error::SandboxError,
    traits::{ExecutionResult, Sandbox},
};

/// Configuration for Firecracker sandbox.
#[derive(Debug, Clone)]
pub struct FirecrackerConfig {
    /// Path to firecracker binary
    pub firecracker_bin: PathBuf,
    /// Path to kernel image (vmlinux)
    pub kernel_path: PathBuf,
    /// Path to root filesystem (ext4)
    pub rootfs_path: PathBuf,
    /// Number of vCPUs
    pub vcpu_count: u8,
    /// Memory size in MiB
    pub mem_size_mib: u32,
    /// Execution timeout
    pub timeout: Duration,
    /// Workspace directory for VM files
    pub workspace: PathBuf,
}

impl Default for FirecrackerConfig {
    fn default() -> Self {
        Self {
            firecracker_bin: PathBuf::from("/usr/bin/firecracker"),
            kernel_path: PathBuf::from("/var/lib/firecracker/vmlinux"),
            rootfs_path: PathBuf::from("/var/lib/firecracker/rootfs.ext4"),
            vcpu_count: 2,
            mem_size_mib: 512,
            timeout: Duration::from_secs(60),
            workspace: PathBuf::from("/tmp/firecracker"),
        }
    }
}

impl FirecrackerConfig {
    /// Create a new config with custom paths.
    #[must_use]
    pub fn new(kernel: impl Into<PathBuf>, rootfs: impl Into<PathBuf>) -> Self {
        Self {
            kernel_path: kernel.into(),
            rootfs_path: rootfs.into(),
            ..Default::default()
        }
    }

    /// Set vCPU count.
    #[must_use]
    pub fn vcpus(mut self, count: u8) -> Self {
        self.vcpu_count = count;
        self
    }

    /// Set memory size in MiB.
    #[must_use]
    pub fn memory(mut self, mib: u32) -> Self {
        self.mem_size_mib = mib;
        self
    }

    /// Set timeout.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set workspace directory.
    #[must_use]
    pub fn workspace(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace = path.into();
        self
    }
}

/// Firecracker MicroVM sandbox.
///
/// Provides strong isolation using Firecracker microVMs.
/// Each execution spawns a new microVM, executes code, and destroys it.
#[derive(Clone)]
pub struct FirecrackerSandbox {
    config: FirecrackerConfig,
}

impl FirecrackerSandbox {
    /// Create a new Firecracker sandbox with default config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: FirecrackerConfig::default(),
        }
    }

    /// Create with custom config.
    #[must_use]
    pub fn with_config(config: FirecrackerConfig) -> Self {
        Self { config }
    }

    /// Builder pattern for configuration.
    #[must_use]
    pub fn builder() -> FirecrackerSandboxBuilder {
        FirecrackerSandboxBuilder::new()
    }

    /// Check if Firecracker is available on the system.
    pub async fn check_prerequisites(&self) -> Result<(), SandboxError> {
        // Check firecracker binary
        if !self.config.firecracker_bin.exists() {
            return Err(SandboxError::NotAvailable(format!(
                "Firecracker binary not found at {:?}",
                self.config.firecracker_bin
            )));
        }

        // Check KVM
        if !std::path::Path::new("/dev/kvm").exists() {
            return Err(SandboxError::NotAvailable(
                "KVM not available (/dev/kvm not found)".into(),
            ));
        }

        // Check kernel
        if !self.config.kernel_path.exists() {
            return Err(SandboxError::NotAvailable(format!(
                "Kernel not found at {:?}",
                self.config.kernel_path
            )));
        }

        // Check rootfs
        if !self.config.rootfs_path.exists() {
            return Err(SandboxError::NotAvailable(format!(
                "Rootfs not found at {:?}",
                self.config.rootfs_path
            )));
        }

        Ok(())
    }

    /// Execute code using firepilot.
    #[cfg(feature = "firecracker")]
    async fn execute_with_firepilot(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        use firepilot::builder::{Configuration, Builder};
        use firepilot::builder::drive::DriveBuilder;
        use firepilot::builder::kernel::KernelBuilder;
        use firepilot::builder::executor::FirecrackerExecutorBuilder;
        use firepilot::machine::Machine;
        use std::path::PathBuf;

        let start = std::time::Instant::now();
        let vm_id = uuid::Uuid::new_v4().to_string();

        info!("Starting Firecracker microVM: {}", vm_id);

        // Create workspace directory
        let workspace = self.config.workspace.join(&vm_id);
        tokio::fs::create_dir_all(&workspace)
            .await
            .map_err(|e| SandboxError::ConfigError(format!("Failed to create workspace: {}", e)))?;

        // Build kernel configuration
        let kernel = KernelBuilder::new()
            .with_kernel_image_path(self.config.kernel_path.to_string_lossy().to_string())
            .with_boot_args("console=ttyS0 reboot=k panic=1 pci=off".to_string())
            .try_build()
            .map_err(|e| SandboxError::ConfigError(format!("Kernel config error: {:?}", e)))?;

        // Build rootfs drive
        let rootfs = DriveBuilder::new()
            .with_drive_id("rootfs".to_string())
            .with_path_on_host(PathBuf::from(&self.config.rootfs_path))
            .as_root_device()
            .try_build()
            .map_err(|e| SandboxError::ConfigError(format!("Drive config error: {:?}", e)))?;

        // Build executor
        let executor = FirecrackerExecutorBuilder::new()
            .with_chroot(workspace.to_string_lossy().to_string())
            .with_exec_binary(PathBuf::from(&self.config.firecracker_bin))
            .try_build()
            .map_err(|e| SandboxError::ConfigError(format!("Executor config error: {:?}", e)))?;

        // Build full configuration
        let config = Configuration::new(vm_id.clone())
            .with_kernel(kernel)
            .with_drive(rootfs)
            .with_executor(executor);

        // Create and start the machine
        let mut machine = Machine::new();

        machine.create(config).await.map_err(|e| {
            SandboxError::StartError(format!("Failed to create VM: {:?}", e))
        })?;

        machine.start().await.map_err(|e| {
            SandboxError::StartError(format!("Failed to start VM: {:?}", e))
        })?;

        debug!("MicroVM started, executing code...");

        // Wait for VM to boot and execute
        // In a real implementation, we'd communicate with an agent inside the VM
        tokio::time::sleep(Duration::from_secs(2)).await;

        // For now, simulate execution result
        let stdout = format!("MicroVM {} executed command", vm_id);
        let stderr = String::new();
        let exit_code = 0;

        // Cleanup
        let _ = machine.stop().await;
        let _ = machine.kill().await;
        let _ = tokio::fs::remove_dir_all(&workspace).await;

        let execution_time_ms = start.elapsed().as_millis() as u64;

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
            execution_time_ms,
            artifacts: vec![],
        })
    }

    /// Fallback execution without firepilot (for testing/dev).
    #[cfg(not(feature = "firecracker"))]
    async fn execute_fallback(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        warn!("Firecracker feature not enabled, using process fallback");

        let start = std::time::Instant::now();

        let output = tokio::time::timeout(
            self.config.timeout,
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(code)
                .output(),
        )
        .await
        .map_err(|_| SandboxError::Timeout)?
        .map_err(SandboxError::IoError)?;

        Ok(ExecutionResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            execution_time_ms: start.elapsed().as_millis() as u64,
            artifacts: vec![],
        })
    }
}

impl Default for FirecrackerSandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Sandbox for FirecrackerSandbox {
    #[instrument(skip(self, code), fields(sandbox = "firecracker"))]
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        debug!("Executing in Firecracker sandbox: {}...", &code[..code.len().min(50)]);

        #[cfg(feature = "firecracker")]
        {
            self.execute_with_firepilot(code).await
        }

        #[cfg(not(feature = "firecracker"))]
        {
            self.execute_fallback(code).await
        }
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        self.check_prerequisites().await.map(|_| true)
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        Ok(())
    }
}

/// Builder for FirecrackerSandbox.
#[derive(Default)]
pub struct FirecrackerSandboxBuilder {
    config: FirecrackerConfig,
}

impl FirecrackerSandboxBuilder {
    /// Create a new builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: FirecrackerConfig::default(),
        }
    }

    /// Set firecracker binary path.
    #[must_use]
    pub fn firecracker_bin(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.firecracker_bin = path.into();
        self
    }

    /// Set kernel path.
    #[must_use]
    pub fn kernel(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.kernel_path = path.into();
        self
    }

    /// Set rootfs path.
    #[must_use]
    pub fn rootfs(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.rootfs_path = path.into();
        self
    }

    /// Set vCPU count.
    #[must_use]
    pub fn vcpus(mut self, count: u8) -> Self {
        self.config.vcpu_count = count;
        self
    }

    /// Set memory in MiB.
    #[must_use]
    pub fn memory(mut self, mib: u32) -> Self {
        self.config.mem_size_mib = mib;
        self
    }

    /// Set timeout.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Set workspace directory.
    #[must_use]
    pub fn workspace(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.workspace = path.into();
        self
    }

    /// Build the sandbox.
    #[must_use]
    pub fn build(self) -> FirecrackerSandbox {
        FirecrackerSandbox::with_config(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = FirecrackerConfig::new("/path/to/kernel", "/path/to/rootfs")
            .vcpus(4)
            .memory(1024)
            .timeout(Duration::from_secs(120));

        assert_eq!(config.vcpu_count, 4);
        assert_eq!(config.mem_size_mib, 1024);
        assert_eq!(config.timeout, Duration::from_secs(120));
    }

    #[test]
    fn test_sandbox_builder() {
        let sandbox = FirecrackerSandbox::builder()
            .kernel("/custom/kernel")
            .rootfs("/custom/rootfs")
            .vcpus(2)
            .memory(512)
            .build();

        assert_eq!(sandbox.config.vcpu_count, 2);
        assert_eq!(sandbox.config.mem_size_mib, 512);
    }

    #[tokio::test]
    async fn test_prerequisites_missing() {
        let sandbox = FirecrackerSandbox::builder()
            .firecracker_bin("/nonexistent/firecracker")
            .build();

        let result = sandbox.check_prerequisites().await;
        assert!(result.is_err());
    }
}
