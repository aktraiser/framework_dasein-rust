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

/// The microVM definition, with the paths as the Firecracker process sees
/// them: on the host, or inside its chroot.
fn machine_config(
    sandbox: &FirecrackerSandbox,
    kernel: &Path,
    rootfs: &Path,
    vsock: &Path,
) -> serde_json::Value {
    serde_json::json!({
        "boot-source": {"kernel_image_path":kernel, "boot_args":"console=ttyS0 keep_bootcon reboot=k panic=1 pci=off ro root=/dev/vda init=/sbin/dasein-init"},
        "drives":[{"drive_id":"rootfs","path_on_host":rootfs,"is_root_device":true,"is_read_only":true}],
        "machine-config":{"vcpu_count":sandbox.config.vcpu_count,"mem_size_mib":sandbox.config.mem_size_mib},
        "vsock":{"guest_cid":3,"uds_path":vsock},
        "network-interfaces":[]
    })
}

/// What it takes to start one microVM and to clean up after it.
struct Launch {
    /// Removed, with everything in it, when the microVM is gone.
    directory: PathBuf,
    /// The vsock socket, as seen from this process.
    vsock: PathBuf,
    /// The cgroup made for the microVM, if any.
    cgroup: Option<PathBuf>,
    /// Program and arguments, run under the deadline of `timeout`.
    program: PathBuf,
    arguments: Vec<std::ffi::OsString>,
}

fn write_config(path: &Path, config: &serde_json::Value) -> Result<(), SandboxError> {
    std::fs::write(
        path,
        serde_json::to_vec(config).map_err(|e| SandboxError::ConfigError(e.to_string()))?,
    )?;
    Ok(())
}

fn socket_path_fits(vsock: &Path) -> Result<(), SandboxError> {
    if vsock.as_os_str().as_encoded_bytes().len() > 100 {
        return Err(SandboxError::ConfigError(
            "VM workspace path is too long for a Unix socket".into(),
        ));
    }
    Ok(())
}

/// Firecracker started directly, as the calling user, in a private directory.
fn direct_launch(sandbox: &FirecrackerSandbox, id: &str) -> Result<Launch, SandboxError> {
    let program = sandbox.config.firecracker_bin.canonicalize()?;
    let directory = private_workspace(&sandbox.config.workspace)?.join(id);
    std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    let launch = Launch {
        vsock: directory.join("v.sock"),
        cgroup: None,
        program,
        arguments: vec![
            "--no-api".into(),
            "--config-file".into(),
            directory.join("config.json").into(),
        ],
        directory,
    };
    // From here on the caller removes the directory, also on failure.
    Ok(launch)
}

fn prepare_direct(sandbox: &FirecrackerSandbox, launch: &Launch) -> Result<(), SandboxError> {
    socket_path_fits(&launch.vsock)?;
    let config = machine_config(
        sandbox,
        &sandbox.config.kernel_path.canonicalize()?,
        &sandbox.config.rootfs_path.canonicalize()?,
        &launch.vsock,
    );
    write_config(&launch.directory.join("config.json"), &config)
}

/// Firecracker started by the jailer: chrooted, unprivileged, in a cgroup.
fn jailed_launch(
    sandbox: &FirecrackerSandbox,
    jailer: &super::JailerConfig,
    id: &str,
) -> Result<Launch, SandboxError> {
    let firecracker = sandbox.config.firecracker_bin.canonicalize()?;
    // Where the jailer builds the chroot of this microVM.
    let directory = jails_of(&sandbox.config, jailer)?.join(id);
    let memory_max = (u64::from(sandbox.config.mem_size_mib)
        + u64::from(jailer.memory_overhead_mib))
        * 1024
        * 1024;
    let mut arguments: Vec<std::ffi::OsString> = vec![
        "--id".into(),
        id.into(),
        "--exec-file".into(),
        firecracker.into(),
        "--uid".into(),
        jailer.uid.to_string().into(),
        "--gid".into(),
        jailer.gid.to_string().into(),
        "--chroot-base-dir".into(),
        jailer.chroot_base.canonicalize()?.into(),
        "--cgroup-version".into(),
        "2".into(),
        "--parent-cgroup".into(),
        (&jailer.parent_cgroup).into(),
    ];
    for limit in [
        // One full CPU per vCPU, no more, whatever the guest does.
        format!(
            "cpu.max={} 100000",
            u32::from(sandbox.config.vcpu_count) * 100_000
        ),
        format!("memory.max={memory_max}"),
        "memory.swap.max=0".into(),
        format!("pids.max={}", jailer.pids_max),
    ] {
        arguments.push("--cgroup".into());
        arguments.push(limit.into());
    }
    arguments.extend([
        "--resource-limit".into(),
        format!("no-file={}", jailer.open_files_max).into(),
        "--".into(),
        "--no-api".into(),
        "--config-file".into(),
        "/config.json".into(),
    ]);
    Ok(Launch {
        vsock: directory.join("root/v.sock"),
        cgroup: Some(
            Path::new("/sys/fs/cgroup")
                .join(&jailer.parent_cgroup)
                .join(id),
        ),
        program: jailer.jailer_bin.canonicalize()?,
        arguments,
        directory,
    })
}

/// Where the jailer puts the chroots of `firecracker`'s microVMs.
fn jails_of(
    config: &super::FirecrackerConfig,
    jailer: &super::JailerConfig,
) -> Result<PathBuf, SandboxError> {
    let firecracker = config.firecracker_bin.canonicalize()?;
    let name = firecracker
        .file_name()
        .ok_or_else(|| SandboxError::ConfigError("Firecracker path has no file name".into()))?;
    Ok(jailer.chroot_base.canonicalize()?.join(name))
}

pub(super) fn remove_stale_jails(
    config: &super::FirecrackerConfig,
    jailer: &super::JailerConfig,
    older_than: Duration,
) -> Result<usize, SandboxError> {
    let jails = jails_of(config, jailer)?;
    let entries = match std::fs::read_dir(&jails) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let id = entry.file_name();
        // Only what this sandbox names its microVMs.
        let ours = id.len() == 32 && id.as_encoded_bytes().iter().all(u8::is_ascii_hexdigit);
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > older_than);
        if !ours || !old || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let cgroup = Path::new("/sys/fs/cgroup")
            .join(&jailer.parent_cgroup)
            .join(&id);
        let running = std::fs::read_to_string(cgroup.join("cgroup.procs"))
            .is_ok_and(|processes| !processes.trim().is_empty());
        if running {
            continue;
        }
        VmGuard::remove(&entry.path(), Some(&cgroup));
        if !entry.path().exists() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn prepare_jail(sandbox: &FirecrackerSandbox, launch: &Launch) -> Result<(), SandboxError> {
    socket_path_fits(&launch.vsock)?;
    let root = launch.directory.join("root");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root)?;
    // Hard links: the same read-only files in every chroot, without a copy.
    for (source, name) in [
        (&sandbox.config.kernel_path, "vmlinux"),
        (&sandbox.config.rootfs_path, "rootfs.ext4"),
    ] {
        std::fs::hard_link(source.canonicalize()?, root.join(name)).map_err(|e| {
            SandboxError::ConfigError(format!(
                "Cannot link {} into the chroot ({e}): it must be on the same filesystem \
                 as the jailer chroot base",
                source.display()
            ))
        })?;
    }
    let config = machine_config(
        sandbox,
        Path::new("/vmlinux"),
        Path::new("/rootfs.ext4"),
        Path::new("/v.sock"),
    );
    write_config(&root.join("config.json"), &config)
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
    let id = uuid::Uuid::new_v4().simple().to_string();
    let launch = match &sandbox.config.jailer {
        Some(jailer) => jailed_launch(sandbox, jailer, &id)?,
        None => direct_launch(sandbox, &id)?,
    };
    let (cancel, mut cancelled) = watch::channel(false);
    // Owns the cleanup from here on, whatever happens next.
    let mut vm = VmGuard {
        directory: launch.directory.clone(),
        cgroup: launch.cgroup.clone(),
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
    if sandbox.config.jailer.is_some() {
        prepare_jail(sandbox, &launch)?;
    } else {
        prepare_direct(sandbox, &launch)?;
    }
    let vsock = launch.vsock.clone();
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
        .arg(&launch.program)
        .args(&launch.arguments)
        .current_dir(if sandbox.config.jailer.is_some() {
            Path::new("/")
        } else {
            launch.directory.as_path()
        })
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
    /// The cgroup of a jailed microVM.
    cgroup: Option<PathBuf>,
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
        // Everything the jailer put in the cgroup, whatever its process
        // group became.
        if let Some(cgroup) = &self.cgroup {
            let _ = std::fs::write(cgroup.join("cgroup.kill"), "1");
        }
    }
    /// Remove what the microVM leaves on the host.
    fn remove(directory: &Path, cgroup: Option<&Path>) {
        let _ = std::fs::remove_dir_all(directory);
        if let Some(cgroup) = cgroup {
            // Empty once its processes are reaped, which may take a moment.
            for _ in 0..50 {
                match std::fs::remove_dir(cgroup) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    _ => break,
                }
            }
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
        let result = std::fs::remove_dir_all(&self.directory);
        Self::remove(&self.directory, self.cgroup.as_deref());
        match result {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}
impl Drop for VmGuard {
    fn drop(&mut self) {
        self.kill();
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.id);
        }
        let directory = self.directory.clone();
        let cgroup = self.cgroup.clone();
        if let Some(mut child) = self.child.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                    Self::remove(&directory, cgroup.as_deref());
                });
            }
        } else {
            Self::remove(&directory, cgroup.as_deref());
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
