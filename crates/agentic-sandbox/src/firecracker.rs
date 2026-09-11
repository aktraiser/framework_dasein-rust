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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, Semaphore};
#[cfg(unix)]
#[path = "firecracker_runtime.rs"]
mod runtime;
use tracing::instrument;

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
    /// Combined stdout/stderr cap, at most 1 MiB.
    pub max_output_bytes: usize,
    /// Maximum simultaneous microVMs per sandbox instance (shared by clones).
    pub max_concurrent: usize,
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
            workspace: PathBuf::from("/tmp/dasein-fc"),
            max_output_bytes: 1024 * 1024,
            max_concurrent: 4,
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
    pub const fn vcpus(mut self, count: u8) -> Self {
        self.vcpu_count = count;
        self
    }

    /// Set memory size in MiB.
    #[must_use]
    pub const fn memory(mut self, mib: u32) -> Self {
        self.mem_size_mib = mib;
        self
    }

    /// Set timeout.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
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

/// Firecracker `MicroVM` sandbox.
///
/// Provides strong isolation using Firecracker microVMs.
/// Each execution spawns a new microVM, executes code, and destroys it.
#[derive(Clone)]
pub struct FirecrackerSandbox {
    config: FirecrackerConfig,
    active: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    permits: Arc<Semaphore>,
}

impl FirecrackerSandbox {
    /// Create a new Firecracker sandbox with default config.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(FirecrackerConfig::default())
    }

    /// Create with custom config.
    #[must_use]
    pub fn with_config(config: FirecrackerConfig) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(config.max_concurrent.min(16))),
            config,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Builder pattern for configuration.
    #[must_use]
    pub fn builder() -> FirecrackerSandboxBuilder {
        FirecrackerSandboxBuilder::new()
    }

    /// Check if Firecracker is available on the system.
    ///
    /// # Errors
    ///
    /// Returns an error if Firecracker, KVM, kernel or rootfs is not available.
    pub fn check_prerequisites(&self) -> Result<(), SandboxError> {
        self.validate_config()?;
        if !cfg!(target_os = "linux") {
            return Err(SandboxError::NotAvailable("Firecracker requires Linux/KVM; on supported Macs use a Linux VM with nested virtualization".into()));
        }
        for (name, path) in [
            ("Firecracker", &self.config.firecracker_bin),
            ("kernel", &self.config.kernel_path),
            ("rootfs", &self.config.rootfs_path),
        ] {
            if !path.is_file() {
                return Err(SandboxError::NotAvailable(format!(
                    "{name} file not found: {}",
                    path.display()
                )));
            }
            std::fs::File::open(path)
                .map_err(|e| SandboxError::NotAvailable(format!("Cannot read {name}: {e}")))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{FileTypeExt, PermissionsExt};
            if self.config.firecracker_bin.metadata()?.permissions().mode() & 0o111 == 0 {
                return Err(SandboxError::NotAvailable(
                    "Firecracker binary is not executable".into(),
                ));
            }
            let kvm = std::path::Path::new("/dev/kvm");
            if !kvm.metadata().is_ok_and(|m| m.file_type().is_char_device()) {
                return Err(SandboxError::NotAvailable(
                    "/dev/kvm is not a character device".into(),
                ));
            }
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(kvm)
                .map_err(|e| {
                    SandboxError::NotAvailable(format!("KVM read/write access unavailable: {e}"))
                })?;
            if !std::path::Path::new("/usr/bin/timeout").is_file() {
                return Err(SandboxError::NotAvailable(
                    "Coreutils-compatible /usr/bin/timeout is required for a host-crash deadline"
                        .into(),
                ));
            }
        }
        Ok(())
    }
    fn validate_config(&self) -> Result<(), SandboxError> {
        if !(1..=8).contains(&self.config.vcpu_count)
            || !(32..=2048).contains(&self.config.mem_size_mib)
            || self.config.timeout.is_zero()
            || self.config.timeout > Duration::from_secs(120)
            || !(1..=1024 * 1024).contains(&self.config.max_output_bytes)
            || !(1..=16).contains(&self.config.max_concurrent)
        {
            return Err(SandboxError::ConfigError("Firecracker limits: 1..8 vCPU, 32..2048 MiB, 0 < timeout <= 120s, 1..1048576 output bytes, 1..16 concurrent VMs".into()));
        }
        Ok(())
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
        self.check_prerequisites()?;
        if code.len() > 256 * 1024 {
            return Err(SandboxError::ConfigError("Code exceeds 256 KiB".into()));
        }
        let _permit = self.permits.try_acquire().map_err(|_| {
            SandboxError::NotReady("Firecracker concurrency capacity reached".into())
        })?;
        #[cfg(unix)]
        {
            runtime::execute(self, code).await
        }
        #[cfg(not(unix))]
        {
            Err(SandboxError::NotAvailable(
                "Firecracker requires Linux".into(),
            ))
        }
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        Ok(self.check_prerequisites().is_ok())
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        let active = self
            .active
            .lock()
            .map_err(|_| SandboxError::StopFailed("VM registry poisoned".into()))?;
        for signal in active.values() {
            signal.send_replace(true);
        }
        Ok(())
    }
}

/// Builder for `FirecrackerSandbox`.
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
    pub const fn vcpus(mut self, count: u8) -> Self {
        self.config.vcpu_count = count;
        self
    }

    /// Set memory in MiB.
    #[must_use]
    pub const fn memory(mut self, mib: u32) -> Self {
        self.config.mem_size_mib = mib;
        self
    }

    /// Set timeout.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
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

    #[test]
    fn test_prerequisites_missing() {
        let sandbox = FirecrackerSandbox::builder()
            .firecracker_bin("/nonexistent/firecracker")
            .build();

        let result = sandbox.check_prerequisites();
        assert!(result.is_err());
    }
}
