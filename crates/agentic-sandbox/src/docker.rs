//! Ephemeral Docker execution with bounded resources and no host mounts.
//!
//! Images must be installed by the operator. Containers run as an unprivileged
//! user with a read-only root, no capabilities and no network. Each execution
//! has a fresh writable workspace. Container isolation is not a VM boundary.

use crate::{ExecutionResult, Sandbox, SandboxError};
use async_trait::async_trait;
use bollard::{
    container::{
        AttachContainerOptions, Config, CreateContainerOptions, ListContainersOptions,
        RemoveContainerOptions, StartContainerOptions, WaitContainerOptions,
    },
    models::{HostConfig, HostConfigLogConfig},
    Docker,
};
use futures::StreamExt;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

/// Docker execution policy, shared by clones of a sandbox.
#[derive(Debug, Clone)]
pub struct DockerSandbox {
    image: String,
    timeout_ms: u64,
    memory_limit: Option<String>,
    cpu_limit: Option<f64>,
    max_output_bytes: usize,
    instance: String,
    active: Arc<Mutex<HashSet<String>>>,
}

impl DockerSandbox {
    /// Configure an execution policy.
    #[must_use]
    pub fn builder() -> DockerSandboxBuilder {
        DockerSandboxBuilder::default()
    }

    /// Image used for every execution.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Label value identifying this sandbox's containers.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance
    }

    /// Remove SDK containers whose execution deadline has expired. Run this
    /// periodically and at service startup to recover after a host process crash.
    /// Only containers carrying both the SDK instance and expiry labels qualify.
    pub async fn reap_expired() -> Result<usize, SandboxError> {
        let docker = Self::client()?;
        let containers = docker
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                filters: HashMap::from([(
                    "label".into(),
                    vec!["dasein.sdk.instance".into(), "dasein.sdk.expires_ms".into()],
                )]),
                ..Default::default()
            }))
            .await
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))?;
        let now = epoch_ms();
        let mut count = 0;
        for container in containers {
            let expires = container
                .labels
                .as_ref()
                .and_then(|labels| labels.get("dasein.sdk.expires_ms"))
                .and_then(|value| value.parse::<u64>().ok());
            if expires.is_some_and(|expires| expires <= now) {
                if let Some(id) = container.id {
                    remove(&docker, &id).await?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    fn client() -> Result<Docker, SandboxError> {
        Docker::connect_with_local_defaults()
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))
    }

    fn host_config(&self) -> Result<HostConfig, SandboxError> {
        let memory = parse_memory(self.memory_limit.as_deref().unwrap_or("512m"))?;
        let cpu = self.cpu_limit.unwrap_or(1.0);
        if !cpu.is_finite()
            || !(0.01..=64.0).contains(&cpu)
            || self.timeout_ms == 0
            || self.max_output_bytes == 0
        {
            return Err(SandboxError::ConfigError(
                "CPU, timeout and output limits must be positive and bounded".into(),
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        let nano_cpus = (cpu * 1_000_000_000.0) as i64;
        Ok(HostConfig {
            network_mode: Some("none".into()),
            readonly_rootfs: Some(true),
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            privileged: Some(false),
            memory: Some(memory),
            memory_swap: Some(memory),
            nano_cpus: Some(nano_cpus),
            pids_limit: Some(64),
            tmpfs: Some(HashMap::from([
                (
                    "/workspace".into(),
                    "rw,nosuid,nodev,size=64m,uid=65534,gid=65534,mode=0700".into(),
                ),
                (
                    "/tmp".into(),
                    "rw,nosuid,nodev,noexec,size=32m,mode=1777".into(),
                ),
            ])),
            log_config: Some(HostConfigLogConfig {
                typ: Some("none".into()),
                config: None,
            }),
            ..Default::default()
        })
    }
}

impl Default for DockerSandbox {
    fn default() -> Self {
        DockerSandboxBuilder::default().build()
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        let docker = Self::client()?;
        let host_config = self.host_config()?;
        docker.inspect_image(&self.image).await.map_err(|e| {
            SandboxError::NotAvailable(format!("Image {} must be installed first: {e}", self.image))
        })?;
        let name = format!("dasein-{}-{}", self.instance, uuid::Uuid::new_v4().simple());
        self.active
            .lock()
            .map_err(|_| SandboxError::CreateFailed("Container registry poisoned".into()))?
            .insert(name.clone());
        let mut cleanup = ContainerCleanup {
            docker: docker.clone(),
            name: name.clone(),
            active: self.active.clone(),
            armed: true,
        };
        let started = std::time::Instant::now();
        // Grace allows normal cleanup to finish before the crash reaper intervenes.
        let expires = epoch_ms()
            .saturating_add(self.timeout_ms)
            .saturating_add(5000)
            .to_string();
        let run = async {
            docker
                .create_container(
                    Some(CreateContainerOptions {
                        name: name.as_str(),
                        platform: None,
                    }),
                    Config {
                        image: Some(self.image.as_str()),
                        user: Some("65534:65534"),
                        working_dir: Some("/workspace"),
                        entrypoint: Some(vec!["/bin/sh", "-c"]),
                        cmd: Some(vec![code]),
                        attach_stdout: Some(true),
                        attach_stderr: Some(true),
                        network_disabled: Some(true),
                        env: Some(vec!["HOME=/workspace", "PYTHONDONTWRITEBYTECODE=1"]),
                        labels: Some(HashMap::from([
                            ("dasein.sdk.instance", self.instance.as_str()),
                            ("dasein.sdk.expires_ms", expires.as_str()),
                        ])),
                        host_config: Some(host_config),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| SandboxError::CreateFailed(e.to_string()))?;
            let attached = docker
                .attach_container(
                    &name,
                    Some(AttachContainerOptions::<String> {
                        stream: Some(true),
                        stdout: Some(true),
                        stderr: Some(true),
                        ..Default::default()
                    }),
                )
                .await
                .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;
            docker
                .start_container(&name, None::<StartContainerOptions<String>>)
                .await
                .map_err(|e| SandboxError::StartFailed(e.to_string()))?;
            let mut logs = attached.output;
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            while let Some(chunk) = logs.next().await {
                let chunk = chunk.map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;
                if stdout
                    .len()
                    .saturating_add(stderr.len())
                    .saturating_add(chunk.as_ref().len())
                    > self.max_output_bytes
                {
                    return Err(SandboxError::ExecutionFailed(
                        "Output limit exceeded".into(),
                    ));
                }
                match chunk {
                    bollard::container::LogOutput::StdErr { message } => {
                        stderr.extend_from_slice(&message);
                    }
                    _ => stdout.extend_from_slice(chunk.as_ref()),
                }
            }
            let exit = docker
                .wait_container(&name, None::<WaitContainerOptions<String>>)
                .next()
                .await
                .ok_or_else(|| {
                    SandboxError::ExecutionFailed("Missing container exit status".into())
                })?;
            let exit_code = match exit {
                Ok(status) => i32::try_from(status.status_code).unwrap_or(-1),
                Err(bollard::errors::Error::DockerContainerWaitError { code, .. }) => {
                    i32::try_from(code).unwrap_or(-1)
                }
                Err(e) => return Err(SandboxError::ExecutionFailed(e.to_string())),
            };
            Ok(ExecutionResult {
                exit_code,
                stdout: String::from_utf8_lossy(&stdout).into(),
                stderr: String::from_utf8_lossy(&stderr).into(),
                execution_time_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                artifacts: vec![],
            })
        };
        let result = tokio::time::timeout(Duration::from_millis(self.timeout_ms), run)
            .await
            .unwrap_or(Err(SandboxError::Timeout));
        cleanup.remove().await?;
        result
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        self.host_config()?;
        let docker = Self::client()?;
        docker
            .ping()
            .await
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))?;
        Ok(docker.inspect_image(&self.image).await.is_ok())
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        let docker = Self::client()?;
        let names: Vec<_> = self
            .active
            .lock()
            .map_err(|_| SandboxError::StopFailed("Container registry poisoned".into()))?
            .iter()
            .cloned()
            .collect();
        for name in names {
            remove(&docker, &name).await?;
            self.active
                .lock()
                .map_err(|_| SandboxError::StopFailed("Container registry poisoned".into()))?
                .remove(&name);
        }
        Ok(())
    }
}

fn epoch_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

async fn remove(docker: &Docker, name: &str) -> Result<(), SandboxError> {
    for _ in 0..30 {
        match docker
            .remove_container(
                name,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(())
            | Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => return Ok(()),
            // Cancellation, explicit stop and the reaper may remove the same
            // container. Wait until Docker finishes the in-flight deletion.
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 409, ..
            }) => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(e) => return Err(SandboxError::StopFailed(e.to_string())),
        }
    }
    Err(SandboxError::StopFailed(
        "Container removal did not finish within 3 seconds".into(),
    ))
}

struct ContainerCleanup {
    docker: Docker,
    name: String,
    active: Arc<Mutex<HashSet<String>>>,
    armed: bool,
}
impl ContainerCleanup {
    async fn remove(&mut self) -> Result<(), SandboxError> {
        remove(&self.docker, &self.name).await?;
        self.armed = false;
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.name);
        }
        Ok(())
    }
}
impl Drop for ContainerCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let docker = self.docker.clone();
        let name = self.name.clone();
        let active = self.active.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                // Retry cancellation cleanup in case creation was in flight.
                for _ in 0..10 {
                    let _ = remove(&docker, &name).await;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                if let Ok(mut active) = active.lock() {
                    active.remove(&name);
                }
            });
        }
    }
}

fn parse_memory(value: &str) -> Result<i64, SandboxError> {
    let value = value.to_ascii_lowercase();
    let (digits, multiplier) = if let Some(s) = value.strip_suffix('m') {
        (s, 1024i64 * 1024)
    } else if let Some(s) = value.strip_suffix('g') {
        (s, 1024i64 * 1024 * 1024)
    } else {
        (value.as_str(), 1)
    };
    digits
        .parse::<i64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| *n >= 6 * 1024 * 1024)
        .ok_or_else(|| {
            SandboxError::ConfigError("Memory limit must be at least 6 MiB (bytes, m or g)".into())
        })
}

/// Builder for ephemeral Docker execution.
#[derive(Debug, Default)]
pub struct DockerSandboxBuilder {
    image: Option<String>,
    timeout_ms: Option<u64>,
    memory_limit: Option<String>,
    cpu_limit: Option<f64>,
    max_output_bytes: Option<usize>,
}
impl DockerSandboxBuilder {
    /// Installed image to execute.
    #[must_use]
    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.image = Some(image.into());
        self
    }
    /// Overall deadline including container creation and execution.
    #[must_use]
    pub const fn timeout_ms(mut self, timeout: u64) -> Self {
        self.timeout_ms = Some(timeout);
        self
    }
    /// Maximum memory in bytes or with m/g suffix.
    #[must_use]
    pub fn memory_limit(mut self, limit: impl Into<String>) -> Self {
        self.memory_limit = Some(limit.into());
        self
    }
    /// CPU quota in cores.
    #[must_use]
    pub const fn cpu_limit(mut self, limit: f64) -> Self {
        self.cpu_limit = Some(limit);
        self
    }
    /// Maximum combined stdout/stderr bytes.
    #[must_use]
    pub const fn max_output_bytes(mut self, limit: usize) -> Self {
        self.max_output_bytes = Some(limit);
        self
    }
    /// Build a policy; validation occurs before execution.
    #[must_use]
    pub fn build(self) -> DockerSandbox {
        DockerSandbox {
            image: self.image.unwrap_or_else(|| "python:3.12-alpine".into()),
            timeout_ms: self.timeout_ms.unwrap_or(30_000),
            memory_limit: self.memory_limit.or_else(|| Some("512m".into())),
            cpu_limit: self.cpu_limit.or(Some(1.0)),
            max_output_bytes: self.max_output_bytes.unwrap_or(1024 * 1024),
            instance: uuid::Uuid::new_v4().simple().to_string(),
            active: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_has_no_host_access() {
        let policy = DockerSandbox::default().host_config().unwrap();
        assert_eq!(policy.network_mode.as_deref(), Some("none"));
        assert_eq!(policy.readonly_rootfs, Some(true));
        assert!(policy.binds.is_none());
        assert_eq!(policy.cap_drop, Some(vec!["ALL".into()]));
    }
    #[test]
    fn rejects_invalid_limits() {
        assert!(DockerSandbox::builder()
            .memory_limit("1m")
            .build()
            .host_config()
            .is_err());
        assert!(DockerSandbox::builder()
            .cpu_limit(f64::NAN)
            .build()
            .host_config()
            .is_err());
    }
}
