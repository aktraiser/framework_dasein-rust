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

/// Subdirectories of `directory`, or none if it does not exist.
fn entries(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// Wait until nothing of any microVM is left on the host.
async fn nothing_left(chroots: &std::path::Path, cgroups: &std::path::Path) {
    for _ in 0..200 {
        if entries(chroots).is_empty() && entries(cgroups).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("left behind: {:?} {:?}", entries(chroots), entries(cgroups));
}

/// The same microVMs through the jailer. Run as root, with `JAILER_BIN` and
/// `JAILER_CHROOT_BASE` (a root-owned directory on the filesystem of the
/// kernel and rootfs) in addition to the variables of the test above.
#[tokio::test]
#[ignore = "requires root, Linux/KVM, cgroup v2 and the jailer; see server/firecracker-guest/README.md"]
async fn real_microvm_lifecycle_jailed() {
    use dasein_agentic_sandbox::JailerConfig;
    use std::os::unix::fs::MetadataExt;

    let jailer = JailerConfig {
        jailer_bin: PathBuf::from(std::env::var("JAILER_BIN").expect("JAILER_BIN")),
        chroot_base: PathBuf::from(
            std::env::var("JAILER_CHROOT_BASE").expect("JAILER_CHROOT_BASE"),
        ),
        parent_cgroup: format!("dasein-test-{}", std::process::id()),
        uid: 65534,
        gid: 65534,
        ..Default::default()
    };
    let cfg = FirecrackerConfig {
        jailer: Some(jailer.clone()),
        vcpu_count: 1,
        mem_size_mib: 128,
        ..config()
    };
    let chroots = jailer.chroot_base.join(
        cfg.firecracker_bin
            .canonicalize()
            .unwrap()
            .file_name()
            .unwrap(),
    );
    let cgroups = std::path::Path::new("/sys/fs/cgroup").join(&jailer.parent_cgroup);
    let links_before = cfg.rootfs_path.metadata().unwrap().nlink();
    let sandbox = FirecrackerSandbox::with_config(cfg.clone());
    sandbox.check_prerequisites().unwrap();

    // The guest behaves as without the jailer.
    let result = sandbox
        .execute("id -u; printf 'err' >&2; echo yes > /workspace/marker")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0, "{result:?}");
    assert_eq!(result.stdout, "65534\n");
    assert_eq!(result.stderr, "err");
    nothing_left(&chroots, &cgroups).await;
    println!("PASS jailed execution and cleanup");

    // What the host sees of a running microVM.
    let clone = sandbox.clone();
    let task = tokio::spawn(async move { clone.execute("sleep 30").await });
    let mut cgroup = None;
    for _ in 0..200 {
        cgroup = entries(&cgroups).into_iter().find(|cgroup| {
            std::fs::read_to_string(cgroup.join("cgroup.procs"))
                .is_ok_and(|processes| !processes.trim().is_empty())
        });
        if cgroup.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let cgroup = cgroup.expect("the microVM has no cgroup");
    // Let the jailer finish dropping privileges and start Firecracker.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let read = |file: &str| {
        std::fs::read_to_string(cgroup.join(file))
            .unwrap()
            .trim()
            .to_owned()
    };
    assert_eq!(read("cpu.max"), "100000 100000");
    assert_eq!(
        read("memory.max"),
        ((128 + u64::from(jailer.memory_overhead_mib)) * 1024 * 1024).to_string()
    );
    assert_eq!(read("memory.swap.max"), "0");
    assert_eq!(read("pids.max"), jailer.pids_max.to_string());
    let processes = read("cgroup.procs");
    assert_eq!(processes.lines().count(), 1, "{processes}");
    let process = std::path::Path::new("/proc").join(processes.trim());
    let status = std::fs::read_to_string(process.join("status")).unwrap();
    let ids = |field: &str| {
        status
            .lines()
            .find(|line| line.starts_with(field))
            .unwrap_or_else(|| panic!("no {field} in {status}"))
            .split_whitespace()
            .skip(1)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    // Real, effective, saved and filesystem IDs: none is root.
    assert_eq!(ids("Uid:"), vec!["65534"; 4]);
    assert_eq!(ids("Gid:"), vec!["65534"; 4]);
    // Its own mount namespace, whose root is the chroot: it holds what was
    // put there for it and nothing of the host.
    assert_ne!(
        std::fs::read_link(process.join("ns/mnt")).unwrap(),
        std::fs::read_link("/proc/self/ns/mnt").unwrap()
    );
    let seen: Vec<String> = std::fs::read_dir(process.join("root"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    for ours in [
        "config.json",
        "firecracker",
        "rootfs.ext4",
        "v.sock",
        "vmlinux",
    ] {
        assert!(seen.iter().any(|name| name == ours), "{ours} in {seen:?}");
    }
    for of_the_host in ["etc", "home", "usr", "bin", "root", "var", "proc"] {
        assert!(!seen.iter().any(|name| name == of_the_host), "{seen:?}");
    }
    assert!(std::fs::read_link(process.join("exe"))
        .unwrap()
        .ends_with("firecracker"));
    println!("PASS unprivileged, chrooted and limited on the host");

    sandbox.stop().await.unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(SandboxError::StopFailed(_))
    ));
    nothing_left(&chroots, &cgroups).await;
    println!("PASS stop cleanup");

    let (a, b) = tokio::join!(sandbox.execute("echo alpha"), sandbox.execute("echo beta"));
    assert_eq!(a.unwrap().stdout, "alpha\n");
    assert_eq!(b.unwrap().stdout, "beta\n");
    println!("PASS concurrent jailed microVMs");

    let timed = FirecrackerSandbox::with_config(FirecrackerConfig {
        timeout: Duration::from_secs(5),
        ..cfg.clone()
    });
    assert!(matches!(
        timed.execute("sleep 30").await,
        Err(SandboxError::Timeout)
    ));
    nothing_left(&chroots, &cgroups).await;
    println!("PASS timeout cleanup");

    // What a killed service leaves behind is removed, but only when it is
    // old and has no process.
    let stale = chroots.join("0123456789abcdef0123456789abcdef");
    let recent = chroots.join("fedcba9876543210fedcba9876543210");
    let foreign = chroots.join("not-one-of-ours");
    for jail in [&stale, &recent, &foreign] {
        std::fs::create_dir_all(jail.join("root")).unwrap();
        std::fs::hard_link(&cfg.rootfs_path, jail.join("root/rootfs.ext4")).unwrap();
    }
    let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    for jail in [&stale, &foreign] {
        std::fs::File::open(jail)
            .unwrap()
            .set_modified(hour_ago)
            .unwrap();
    }
    assert_eq!(sandbox.remove_stale_jails().unwrap(), 1);
    assert!(!stale.exists() && recent.exists() && foreign.exists());
    std::fs::remove_dir_all(&recent).unwrap();
    std::fs::remove_dir_all(&foreign).unwrap();
    println!("PASS stale jails removed, others kept");

    // The shared image is untouched and no chroot still links to it.
    assert_eq!(cfg.rootfs_path.metadata().unwrap().nlink(), links_before);
    let result = sandbox.execute("echo still-works").await.unwrap();
    assert_eq!(result.stdout, "still-works\n");
    let _ = std::fs::remove_dir(&cgroups);
    let _ = std::fs::remove_dir(&chroots);
    std::fs::remove_dir(&cfg.workspace).ok();
}
