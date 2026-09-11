//! Explicit Linux/KVM integration test. Never falls back to a host shell.
#![cfg(target_os = "linux")]
use dasein_agentic_sandbox::{FirecrackerConfig, FirecrackerSandbox, Sandbox, SandboxError};
use std::{path::PathBuf, time::Duration};
fn config() -> FirecrackerConfig {
    FirecrackerConfig {
        firecracker_bin: PathBuf::from(std::env::var("FIRECRACKER_BIN").expect("FIRECRACKER_BIN")),
        kernel_path: PathBuf::from(std::env::var("KERNEL_PATH").expect("KERNEL_PATH")),
        rootfs_path: PathBuf::from(std::env::var("ROOTFS_PATH").expect("ROOTFS_PATH")),
        workspace: PathBuf::from(format!("/tmp/fc-test-{}", std::process::id())),
        vcpu_count: 1,
        mem_size_mib: 128,
        timeout: Duration::from_secs(15),
        max_concurrent: 2,
        ..Default::default()
    }
}
async fn empty(workspace: &std::path::Path) {
    for _ in 0..100 {
        if std::fs::read_dir(workspace).unwrap().next().is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("microVM workspace was not cleaned");
}
#[tokio::test]
#[ignore = "requires Linux/KVM and a prepared guest image; see server/firecracker-guest/README.md"]
async fn real_microvm_lifecycle() {
    let cfg = config();
    let sandbox = FirecrackerSandbox::with_config(cfg.clone());
    sandbox.check_prerequisites().unwrap();
    assert!(sandbox.is_ready().await.unwrap());
    let result = sandbox
        .execute("id -u; uname -r; printf 'err' >&2; echo yes > /workspace/marker")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.starts_with("65534\n"));
    assert_eq!(result.stderr, "err");
    println!("identity/kernel: {}", result.stdout.trim());
    let result=sandbox.execute("test ! -e /workspace/marker && ! touch /etc/cannot-write && test $(ls /sys/class/net | wc -l) -eq 1 && python3 -c 'import socket; s=socket.socket(); s.settimeout(1); assert s.connect_ex((\"1.1.1.1\",443)) != 0'").await.unwrap();
    assert_eq!(result.exit_code, 0, "{:?}", result);
    println!("PASS fresh filesystem, readonly root and no network");
    let result = sandbox.execute("printf 'é🙂'; exit 7").await.unwrap();
    assert_eq!(result.exit_code, 7);
    assert_eq!(result.stdout, "é🙂");
    let (a, b) = tokio::join!(sandbox.execute("echo alpha"), sandbox.execute("echo beta"));
    assert_eq!(a.unwrap().stdout, "alpha\n");
    assert_eq!(b.unwrap().stdout, "beta\n");
    println!("PASS concurrent response correlation");
    let bounded = FirecrackerSandbox::with_config(FirecrackerConfig {
        max_output_bytes: 1024,
        ..cfg.clone()
    });
    assert!(matches!(
        bounded.execute("python3 -c 'print(\"x\"*100000)' ").await,
        Err(SandboxError::ExecutionFailed(_))
    ));
    println!("PASS output limit");
    let timed = FirecrackerSandbox::with_config(FirecrackerConfig {
        timeout: Duration::from_secs(5),
        ..cfg.clone()
    });
    assert!(matches!(
        timed.execute("sleep 30").await,
        Err(SandboxError::Timeout)
    ));
    println!("PASS timeout");
    empty(&cfg.workspace).await;
    let clone = sandbox.clone();
    let task = tokio::spawn(async move { clone.execute("sleep 30").await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    task.abort();
    let _ = task.await;
    empty(&cfg.workspace).await;
    println!("PASS future cancellation cleanup");
    let clone = sandbox.clone();
    let task = tokio::spawn(async move { clone.execute("sleep 30").await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    sandbox.stop().await.unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(SandboxError::StopFailed(_))
    ));
    empty(&cfg.workspace).await;
    println!("PASS stop cleanup");
    let bad = FirecrackerSandbox::with_config(FirecrackerConfig {
        kernel_path: cfg.rootfs_path.clone(),
        ..cfg.clone()
    });
    assert!(matches!(
        bad.execute("echo impossible").await,
        Err(SandboxError::StartFailed(_))
    ));
    empty(&cfg.workspace).await;
    println!("PASS boot failure cleanup");
    let limited = FirecrackerSandbox::with_config(FirecrackerConfig {
        max_concurrent: 1,
        ..cfg.clone()
    });
    let clone = limited.clone();
    let task = tokio::spawn(async move { clone.execute("sleep 30").await });
    for _ in 0..100 {
        if std::fs::read_dir(&cfg.workspace).unwrap().next().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(matches!(
        limited.execute("echo overload").await,
        Err(SandboxError::NotReady(_))
    ));
    limited.stop().await.unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(SandboxError::StopFailed(_))
    ));
    empty(&cfg.workspace).await;
    println!("PASS shared concurrency capacity");
    std::fs::remove_dir(&cfg.workspace).unwrap();
}
