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
    /// Run each microVM through Firecracker's jailer: in a chroot, as an
    /// unprivileged user, inside a cgroup with CPU, memory and process
    /// limits. `None` starts Firecracker directly, as the calling user.
    pub jailer: Option<JailerConfig>,
}

/// How the jailer confines the Firecracker process of each microVM.
///
/// The process that uses the sandbox must then run as root: the jailer
/// needs it to build the chroot and the cgroup, and drops Firecracker itself
/// to `uid`/`gid` before it starts. Requires cgroup v2.
#[derive(Debug, Clone)]
pub struct JailerConfig {
    /// Path to the jailer binary, of the same release as Firecracker.
    pub jailer_bin: PathBuf,
    /// User Firecracker runs as. Not root.
    pub uid: u32,
    /// Group Firecracker runs as. Not root.
    pub gid: u32,
    /// Directory the chroots are made in, owned by root and writable by
    /// nobody else. It must be on the same filesystem as the kernel and the
    /// rootfs, which are hard-linked into each chroot, and short: the path
    /// of the vsock socket inside a chroot has to fit in 100 bytes.
    pub chroot_base: PathBuf,
    /// Cgroup the microVM cgroups are made under, below `/sys/fs/cgroup`.
    pub parent_cgroup: String,
    /// Host memory the Firecracker process may use on top of the guest's
    /// memory, in MiB. Beyond it the kernel kills the microVM.
    pub memory_overhead_mib: u32,
    /// Processes and threads the Firecracker process may have.
    pub pids_max: u32,
    /// Open files the Firecracker process may have.
    pub open_files_max: u32,
}

impl Default for JailerConfig {
    fn default() -> Self {
        Self {
            jailer_bin: PathBuf::from("/usr/bin/jailer"),
            uid: 65534,
            gid: 65534,
            chroot_base: PathBuf::from("/srv/jailer"),
            parent_cgroup: "dasein".into(),
            memory_overhead_mib: 128,
            pids_max: 64,
            open_files_max: 256,
        }
    }
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
            jailer: None,
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

    /// Confine each microVM with the jailer.
    #[must_use]
    pub fn jailer(mut self, jailer: JailerConfig) -> Self {
        self.jailer = Some(jailer);
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
            if let Some(jailer) = &self.config.jailer {
                Self::check_jailer(jailer)?;
            }
            if !std::path::Path::new("/usr/bin/timeout").is_file() {
                return Err(SandboxError::NotAvailable(
                    "Coreutils-compatible /usr/bin/timeout is required for a host-crash deadline"
                        .into(),
                ));
            }
        }
        Ok(())
    }
    /// Remove the chroots and cgroups left by microVMs whose process died
    /// without cleaning up, for instance when the service was killed. Call
    /// it when the service starts. Returns how many were removed.
    ///
    /// Only jails older than ten minutes with no process left in their
    /// cgroup are removed, so that microVMs being started or run by another
    /// instance are not touched. Does nothing without the jailer.
    ///
    /// # Errors
    ///
    /// Returns an error if the jails cannot be listed.
    pub fn remove_stale_jails(&self) -> Result<usize, SandboxError> {
        #[cfg(unix)]
        if let Some(jailer) = &self.config.jailer {
            return runtime::remove_stale_jails(&self.config, jailer, Duration::from_secs(600));
        }
        Ok(0)
    }

    #[cfg(unix)]
    fn check_jailer(jailer: &JailerConfig) -> Result<(), SandboxError> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let unavailable = |reason: String| Err(SandboxError::NotAvailable(reason));
        if !nix::unistd::geteuid().is_root() {
            return unavailable(
                "The jailer needs the calling process to run as root; it drops Firecracker \
                 to the configured user"
                    .into(),
            );
        }
        if jailer.uid == 0 || jailer.gid == 0 {
            return Err(SandboxError::ConfigError(
                "The jailer must drop Firecracker to a user and group other than root".into(),
            ));
        }
        if !jailer
            .jailer_bin
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        {
            return unavailable(format!(
                "Jailer binary is not an executable file: {}",
                jailer.jailer_bin.display()
            ));
        }
        let valid_cgroup = !jailer.parent_cgroup.is_empty()
            && jailer.parent_cgroup.len() <= 64
            && jailer
                .parent_cgroup
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !valid_cgroup
            || jailer.pids_max == 0
            || jailer.open_files_max < 16
            || jailer.memory_overhead_mib == 0
        {
            return Err(SandboxError::ConfigError(
                "Jailer limits must be positive (at least 16 open files) and the parent \
                 cgroup a single name of letters, digits, underscores or hyphens"
                    .into(),
            ));
        }
        if !std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").is_file() {
            return unavailable("The jailer limits need cgroup v2 at /sys/fs/cgroup".into());
        }
        // Root builds chroots there: nobody else may be able to plant
        // anything in it.
        let base = jailer.chroot_base.metadata().map_err(|e| {
            SandboxError::NotAvailable(format!(
                "Jailer chroot base {}: {e}",
                jailer.chroot_base.display()
            ))
        })?;
        if !jailer.chroot_base.is_absolute()
            || !base.is_dir()
            || base.uid() != 0
            || base.permissions().mode() & 0o022 != 0
        {
            return Err(SandboxError::ConfigError(
                "Jailer chroot base must be an absolute directory owned by root and \
                 writable by root only"
                    .into(),
            ));
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
    fn is_isolated(&self) -> bool {
        true
    }

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
