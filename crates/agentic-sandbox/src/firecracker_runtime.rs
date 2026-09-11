//! Linux microVM launcher and bounded, correlated guest protocol over virtio-vsock.
use super::{ExecutionResult, FirecrackerSandbox, SandboxError};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    process::{Child, Command},
    sync::watch,
};

#[derive(Serialize)]
struct Request<'a> {
    protocol: u8,
    request_id: &'a str,
    code: &'a str,
    timeout_ms: u64,
    max_output_bytes: usize,
}
#[derive(Deserialize)]
struct Response {
    protocol: u8,
    request_id: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
    error: Option<String>,
}

fn machine_config(
    sandbox: &FirecrackerSandbox,
    vsock: &Path,
) -> Result<serde_json::Value, SandboxError> {
    Ok(serde_json::json!({
        "boot-source": {"kernel_image_path":sandbox.config.kernel_path.canonicalize()?, "boot_args":"console=ttyS0 keep_bootcon reboot=k panic=1 pci=off ro root=/dev/vda init=/sbin/dasein-init"},
        "drives":[{"drive_id":"rootfs","path_on_host":sandbox.config.rootfs_path.canonicalize()?,"is_root_device":true,"is_read_only":true}],
        "machine-config":{"vcpu_count":sandbox.config.vcpu_count,"mem_size_mib":sandbox.config.mem_size_mib},
        "vsock":{"guest_cid":3,"uds_path":vsock},
        "network-interfaces":[]
    }))
}
fn private_workspace(root: &Path) -> Result<PathBuf, SandboxError> {
    match std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(root)
    {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(SandboxError::ConfigError("VM workspace must be a private directory owned by the service user (0700), not a symlink".into()));
    }
    Ok(root.canonicalize()?)
}

pub(super) async fn execute(
    sandbox: &FirecrackerSandbox,
    code: &str,
) -> Result<ExecutionResult, SandboxError> {
    let root = private_workspace(&sandbox.config.workspace)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let directory = root.join(&id);
    std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    let (cancel, mut cancelled) = watch::channel(false);
    let mut vm = VmGuard {
        directory,
        child: None,
        id: id.clone(),
        active: sandbox.active.clone(),
        logs: Arc::new(Mutex::new(Vec::new())),
    };
    sandbox
        .active
        .lock()
        .map_err(|_| SandboxError::CreateFailed("VM registry poisoned".into()))?
        .insert(id.clone(), cancel);
    let vsock = vm.directory.join("v.sock");
    if vsock.as_os_str().as_encoded_bytes().len() > 100 {
        return Err(SandboxError::ConfigError(
            "VM workspace path is too long for a Unix socket".into(),
        ));
    }
    let config = machine_config(sandbox, &vsock)?;
    let config_path = vm.directory.join("config.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec(&config).map_err(|e| SandboxError::ConfigError(e.to_string()))?,
    )?;
    let started = Instant::now();
    // The coreutils timeout survives an unexpected SDK/server process death. The entire
    // VM process group is also killed synchronously by VmGuard on cancellation.
    let mut command = Command::new("/usr/bin/timeout");
    command
        .args([
            "--signal=KILL",
            "--kill-after=1s",
            &format!("{}s", sandbox.config.timeout.as_secs_f64() + 2.0),
        ])
        .arg(sandbox.config.firecracker_bin.canonicalize()?)
        .args(["--no-api", "--config-file"])
        .arg(&config_path)
        .current_dir(&vm.directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| SandboxError::StartFailed(e.to_string()))?;
    if let Some(stdout) = child.stdout.take() {
        drain(stdout, vm.logs.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        drain(stderr, vm.logs.clone());
    }
    vm.child = Some(child);
    let result = tokio::select! {
        result=tokio::time::timeout(sandbox.config.timeout, exchange(&mut vm,&vsock,&id,code,sandbox.config.timeout,sandbox.config.max_output_bytes))=>result.unwrap_or(Err(SandboxError::Timeout)),
        _=cancelled.changed()=>Err(SandboxError::StopFailed("Firecracker execution cancelled".into())),
    };
    vm.cleanup().await?;
    result.map(|response| ExecutionResult {
        exit_code: response.exit_code,
        stdout: response.stdout,
        stderr: response.stderr,
        execution_time_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        artifacts: vec![],
    })
}

async fn exchange(
    vm: &mut VmGuard,
    path: &Path,
    id: &str,
    code: &str,
    timeout: Duration,
    max_output: usize,
) -> Result<Response, SandboxError> {
    let mut stream = loop {
        if let Some(status) = vm.child.as_mut().expect("spawned above").try_wait()? {
            tokio::task::yield_now().await;
            let logs = vm
                .logs
                .lock()
                .map(|v| String::from_utf8_lossy(&v).into_owned())
                .unwrap_or_default();
            return Err(SandboxError::StartFailed(format!(
                "Firecracker exited {status}: {logs}"
            )));
        }
        if let Ok(mut candidate) = UnixStream::connect(path).await {
            let connect = async {
                candidate.write_all(b"CONNECT 5000\n").await?;
                let line = read_line(&mut candidate, 64).await?;
                if !line
                    .strip_prefix("OK ")
                    .is_some_and(|port| port.trim().parse::<u32>().is_ok())
                {
                    return Err(std::io::Error::other("Invalid vsock handshake"));
                }
                Ok::<_, std::io::Error>(candidate)
            };
            if let Ok(Ok(stream)) = tokio::time::timeout(Duration::from_millis(200), connect).await
            {
                break stream;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let request = Request {
        protocol: 1,
        request_id: id,
        code,
        timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        max_output_bytes: max_output,
    };
    let data =
        serde_json::to_vec(&request).map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;
    stream
        .write_u32(
            u32::try_from(data.len())
                .map_err(|_| SandboxError::ConfigError("Request too large".into()))?,
        )
        .await?;
    stream.write_all(&data).await?;
    let size = stream.read_u32().await? as usize;
    if size > 8 * 1024 * 1024 {
        return Err(SandboxError::ExecutionFailed(
            "Guest response exceeds 8 MiB".into(),
        ));
    }
    let mut data = vec![0; size];
    stream.read_exact(&mut data).await?;
    decode_response(&data, id, max_output)
}
fn decode_response(data: &[u8], id: &str, max_output: usize) -> Result<Response, SandboxError> {
    let response: Response = serde_json::from_slice(data)
        .map_err(|e| SandboxError::ExecutionFailed(format!("Invalid guest response: {e}")))?;
    if response.protocol != 1 || response.request_id != id {
        return Err(SandboxError::ExecutionFailed(
            "Guest response protocol or correlation mismatch".into(),
        ));
    }
    if response.stdout.len().saturating_add(response.stderr.len()) > max_output {
        return Err(SandboxError::ExecutionFailed(
            "Guest output limit exceeded".into(),
        ));
    }
    if let Some(error) = &response.error {
        return Err(if error == "timeout" {
            SandboxError::Timeout
        } else {
            SandboxError::ExecutionFailed(error.clone())
        });
    }
    Ok(response)
}
async fn read_line(stream: &mut UnixStream, maximum: usize) -> std::io::Result<String> {
    let mut data = Vec::new();
    for _ in 0..maximum {
        let byte = stream.read_u8().await?;
        if byte == b'\n' {
            return String::from_utf8(data).map_err(std::io::Error::other);
        }
        data.push(byte);
    }
    Err(std::io::Error::other("Overlong vsock handshake"))
}
fn drain(
    mut stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    logs: Arc<Mutex<Vec<u8>>>,
) {
    tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        while let Ok(n) = stream.read(&mut buffer).await {
            if n == 0 {
                break;
            }
            if let Ok(mut logs) = logs.lock() {
                logs.extend_from_slice(&buffer[..n]);
                let excess = logs.len().saturating_sub(8192);
                logs.drain(..excess);
            }
        }
    });
}
struct VmGuard {
    directory: PathBuf,
    child: Option<Child>,
    id: String,
    active: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    logs: Arc<Mutex<Vec<u8>>>,
}
impl VmGuard {
    fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if let Some(pid) = child.id().and_then(|id| i32::try_from(id).ok()) {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            let _ = child.start_kill();
        }
    }
    async fn cleanup(&mut self) -> Result<(), SandboxError> {
        self.kill();
        if let Some(mut child) = self.child.take() {
            child.wait().await?;
        }
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.id);
        }
        std::fs::remove_dir_all(&self.directory)?;
        Ok(())
    }
}
impl Drop for VmGuard {
    fn drop(&mut self) {
        self.kill();
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.id);
        }
        let directory = self.directory.clone();
        if let Some(mut child) = self.child.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                    let _ = std::fs::remove_dir_all(directory);
                });
            }
        } else {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_foreign_or_excessive_guest_responses() {
        let response = serde_json::json!({"protocol":1,"request_id":"other","exit_code":0,"stdout":"12345","stderr":"","error":null});
        let data = serde_json::to_vec(&response).unwrap();
        assert!(decode_response(&data, "expected", 10).is_err());
        assert!(decode_response(&data, "other", 2).is_err());
        assert!(decode_response(&data, "other", 10).is_ok());
    }
    #[test]
    fn rejects_zero_limits() {
        let config = super::super::FirecrackerConfig {
            max_concurrent: 0,
            ..Default::default()
        };
        assert!(FirecrackerSandbox::with_config(config)
            .validate_config()
            .is_err());
    }
}
