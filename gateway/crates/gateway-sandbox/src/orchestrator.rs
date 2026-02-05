//! Remote Firecracker orchestrator via SSH
//!
//! Manages the Firecracker VM lifecycle on a remote server

use async_trait::async_trait;
use russh::*;
use russh_keys::key::PublicKey;
use std::sync::Arc;
use tokio::sync::Mutex;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum OrchestratorError {
    #[error("SSH connection failed: {0}")]
    ConnectionFailed(String),

    #[error("SSH authentication failed: {0}")]
    AuthFailed(String),

    #[error("Command execution failed: {0}")]
    CommandFailed(String),

    #[error("Firecracker not ready after {0} attempts")]
    StartupTimeout(u32),

    #[error("VM health check failed: {0}")]
    HealthCheckFailed(String),
}

pub type OrchestratorResult<T> = Result<T, OrchestratorError>;

/// Configuration for the remote Firecracker server
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// SSH host (IP or hostname)
    pub host: String,
    /// SSH port (default: 22)
    pub port: u16,
    /// SSH username
    pub username: String,
    /// SSH password
    pub password: String,
    /// fc-agent URL (internal VM address)
    pub agent_internal_url: String,
    /// fc-agent URL (external, for port forwarding check)
    pub agent_external_url: String,
    /// Firecracker config file path on server
    pub vm_config_path: String,
    /// Max attempts to wait for VM startup
    pub startup_max_attempts: u32,
    /// Delay between startup checks (ms)
    pub startup_check_delay_ms: u64,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            host: std::env::var("FIRECRACKER_SSH_HOST")
                .unwrap_or_else(|_| "65.108.230.227".to_string()),
            port: std::env::var("FIRECRACKER_SSH_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(22),
            username: std::env::var("FIRECRACKER_SSH_USER")
                .unwrap_or_else(|_| "root".to_string()),
            password: std::env::var("FIRECRACKER_SSH_PASSWORD")
                .unwrap_or_default(),
            agent_internal_url: "http://172.16.0.2:8080".to_string(),
            agent_external_url: std::env::var("FIRECRACKER_REMOTE_URL")
                .unwrap_or_else(|_| "http://65.108.230.227:8080".to_string()),
            vm_config_path: "/var/lib/firecracker/vm-config.json".to_string(),
            startup_max_attempts: 30,
            startup_check_delay_ms: 1000,
        }
    }
}

/// SSH client handler
struct SshHandler;

#[async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        // Accept all server keys (in production, verify against known hosts)
        Ok(true)
    }
}

/// Remote Firecracker orchestrator
pub struct FirecrackerOrchestrator {
    config: OrchestratorConfig,
    session: Mutex<Option<client::Handle<SshHandler>>>,
}

impl FirecrackerOrchestrator {
    pub fn new(config: OrchestratorConfig) -> Self {
        Self {
            config,
            session: Mutex::new(None),
        }
    }

    /// Create with default configuration from environment
    pub fn from_env() -> Self {
        Self::new(OrchestratorConfig::default())
    }

    /// Connect to the remote server via SSH
    pub async fn connect(&self) -> OrchestratorResult<()> {
        info!(
            host = %self.config.host,
            port = self.config.port,
            user = %self.config.username,
            "Connecting to Firecracker server via SSH"
        );

        let config = client::Config::default();
        let config = Arc::new(config);
        let handler = SshHandler;

        let mut session = client::connect(
            config,
            (self.config.host.as_str(), self.config.port),
            handler,
        )
        .await
        .map_err(|e| OrchestratorError::ConnectionFailed(e.to_string()))?;

        // Authenticate with password
        let auth_result = session
            .authenticate_password(&self.config.username, &self.config.password)
            .await
            .map_err(|e| OrchestratorError::AuthFailed(e.to_string()))?;

        if !auth_result {
            return Err(OrchestratorError::AuthFailed("Password rejected".to_string()));
        }

        info!("SSH connection established");

        let mut guard = self.session.lock().await;
        *guard = Some(session);

        Ok(())
    }

    /// Execute a command on the remote server
    pub async fn exec(&self, command: &str) -> OrchestratorResult<(i32, String, String)> {
        let guard = self.session.lock().await;
        let session = guard.as_ref()
            .ok_or_else(|| OrchestratorError::ConnectionFailed("Not connected".to_string()))?;

        debug!(command = %command, "Executing remote command");

        let mut channel = session
            .channel_open_session()
            .await
            .map_err(|e| OrchestratorError::CommandFailed(e.to_string()))?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| OrchestratorError::CommandFailed(e.to_string()))?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0i32;

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    stdout.push_str(&String::from_utf8_lossy(&data));
                }
                Some(ChannelMsg::ExtendedData { data, ext }) => {
                    if ext == 1 {
                        stderr.push_str(&String::from_utf8_lossy(&data));
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = exit_status as i32;
                }
                Some(ChannelMsg::Eof) | None => break,
                _ => {}
            }
        }

        debug!(exit_code = exit_code, "Command completed");

        Ok((exit_code, stdout, stderr))
    }

    /// Check if Firecracker VM is running and fc-agent is healthy
    pub async fn is_vm_healthy(&self) -> bool {
        // Check via SSH if fc-agent responds
        match self.exec(&format!("curl -s --connect-timeout 2 {}/health", self.config.agent_internal_url)).await {
            Ok((0, stdout, _)) => {
                stdout.contains("healthy")
            }
            _ => false,
        }
    }

    /// Check if Firecracker process is running
    pub async fn is_firecracker_running(&self) -> OrchestratorResult<bool> {
        let (code, stdout, _) = self.exec("pgrep -x firecracker").await?;
        Ok(code == 0 && !stdout.trim().is_empty())
    }

    /// Setup the TAP network interface
    pub async fn setup_tap(&self) -> OrchestratorResult<()> {
        info!("Setting up TAP interface");

        let commands = [
            "ip link delete tap0 2>/dev/null || true",
            "ip tuntap add dev tap0 mode tap",
            "ip addr add 172.16.0.1/24 dev tap0",
            "ip link set tap0 up",
        ];

        for cmd in commands {
            let (code, _, stderr) = self.exec(cmd).await?;
            if code != 0 && !stderr.is_empty() {
                warn!(cmd = %cmd, stderr = %stderr, "TAP setup command warning");
            }
        }

        // Enable IP forwarding
        self.exec("echo 1 > /proc/sys/net/ipv4/ip_forward").await?;

        // Setup port forwarding (external 8080 -> VM 8080)
        let iptables_cmds = [
            "iptables -t nat -D PREROUTING -p tcp --dport 8080 -j DNAT --to-destination 172.16.0.2:8080 2>/dev/null || true",
            "iptables -t nat -A PREROUTING -p tcp --dport 8080 -j DNAT --to-destination 172.16.0.2:8080",
            "iptables -t nat -D POSTROUTING -p tcp -d 172.16.0.2 --dport 8080 -j MASQUERADE 2>/dev/null || true",
            "iptables -t nat -A POSTROUTING -p tcp -d 172.16.0.2 --dport 8080 -j MASQUERADE",
        ];

        for cmd in iptables_cmds {
            let _ = self.exec(cmd).await;
        }

        info!("TAP interface configured");
        Ok(())
    }

    /// Start Firecracker VM
    pub async fn start_vm(&self) -> OrchestratorResult<()> {
        info!("Starting Firecracker VM");

        // Kill any existing Firecracker process
        let _ = self.exec("pkill -9 firecracker 2>/dev/null || true").await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Remove old socket
        self.exec("rm -f /tmp/firecracker.socket").await?;

        // Setup TAP
        self.setup_tap().await?;

        // Start Firecracker in background
        let start_cmd = format!(
            "nohup firecracker --api-sock /tmp/firecracker.socket --config-file {} > /var/log/firecracker.log 2>&1 &",
            self.config.vm_config_path
        );

        self.exec(&start_cmd).await?;

        // Wait for VM to be ready
        info!("Waiting for VM to become healthy...");

        for attempt in 1..=self.config.startup_max_attempts {
            tokio::time::sleep(tokio::time::Duration::from_millis(
                self.config.startup_check_delay_ms,
            ))
            .await;

            if self.is_vm_healthy().await {
                info!(attempts = attempt, "Firecracker VM is healthy");
                return Ok(());
            }

            debug!(attempt = attempt, max = self.config.startup_max_attempts, "VM not ready yet");
        }

        Err(OrchestratorError::StartupTimeout(self.config.startup_max_attempts))
    }

    /// Stop Firecracker VM
    pub async fn stop_vm(&self) -> OrchestratorResult<()> {
        info!("Stopping Firecracker VM");

        self.exec("pkill -9 firecracker 2>/dev/null || true").await?;
        self.exec("rm -f /tmp/firecracker.socket").await?;

        info!("Firecracker VM stopped");
        Ok(())
    }

    /// Restart Firecracker VM
    pub async fn restart_vm(&self) -> OrchestratorResult<()> {
        self.stop_vm().await?;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        self.start_vm().await
    }

    /// Ensure VM is running (start if not)
    pub async fn ensure_running(&self) -> OrchestratorResult<()> {
        if self.is_vm_healthy().await {
            info!("VM already healthy");
            return Ok(());
        }

        info!("VM not healthy, starting...");
        self.start_vm().await
    }

    /// Get VM status
    pub async fn status(&self) -> OrchestratorResult<VmStatus> {
        let firecracker_running = self.is_firecracker_running().await?;
        let agent_healthy = self.is_vm_healthy().await;

        let (_, uptime, _) = self.exec("uptime -p 2>/dev/null || uptime").await?;

        Ok(VmStatus {
            firecracker_running,
            agent_healthy,
            server_uptime: uptime.trim().to_string(),
        })
    }

    /// Disconnect SSH session
    pub async fn disconnect(&self) {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.take() {
            let _ = session
                .disconnect(Disconnect::ByApplication, "Goodbye", "en")
                .await;
        }
        info!("SSH disconnected");
    }
}

/// VM status information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VmStatus {
    pub firecracker_running: bool,
    pub agent_healthy: bool,
    pub server_uptime: String,
}

impl Drop for FirecrackerOrchestrator {
    fn drop(&mut self) {
        // Note: Can't do async cleanup in Drop
        // Session will be cleaned up when dropped
    }
}
