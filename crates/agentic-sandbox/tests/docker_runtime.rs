#![cfg(feature = "docker")]
use dasein_agentic_sandbox::{DockerSandbox, Sandbox, SandboxError};
use std::{collections::HashMap, time::Duration};
async fn container_count(sandbox: &DockerSandbox) -> usize {
    let docker = bollard::Docker::connect_with_local_defaults().unwrap();
    docker
        .list_containers(Some(bollard::container::ListContainersOptions::<String> {
            all: true,
            filters: HashMap::from([(
                "label".into(),
                vec![format!("dasein.sdk.instance={}", sandbox.instance_id())],
            )]),
            ..Default::default()
        }))
        .await
        .unwrap()
        .len()
}
#[tokio::test]
#[ignore = "Requires Docker and python:3.12-alpine image"]
async fn docker_exec_isolated_and_cleans_up() {
    let sandbox = DockerSandbox::default();
    assert!(sandbox.is_ready().await.unwrap());
    let result = sandbox
        .execute("python -c 'import os; print(os.getuid()); print(6*7)' ; echo problem >&2; exit 7")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 7);
    assert_eq!(result.stdout, "65534\n42\n");
    assert_eq!(result.stderr, "problem\n");
    let first = sandbox
        .execute("echo secret > /workspace/test; touch /root/forbidden 2>/dev/null; test $? -ne 0")
        .await
        .unwrap();
    assert_eq!(first.exit_code, 0);
    let second = sandbox.execute("test ! -e /workspace/test && python -c 'import socket; s=socket.socket(); s.settimeout(1); assert s.connect_ex((\"1.1.1.1\",443)) != 0'").await.unwrap();
    assert_eq!(second.exit_code, 0, "{second:?}");
    assert_eq!(container_count(&sandbox).await, 0);
}
#[tokio::test]
#[ignore = "Requires Docker and python:3.12-alpine image"]
async fn docker_enforces_deadline_and_output_cap() {
    let sandbox = DockerSandbox::builder().timeout_ms(1500).build();
    assert!(matches!(
        sandbox.execute("sleep 10").await,
        Err(SandboxError::Timeout)
    ));
    assert_eq!(container_count(&sandbox).await, 0);
    let sandbox = DockerSandbox::builder().max_output_bytes(1024).build();
    let error = sandbox
        .execute("python -c 'print(\"x\"*1000000)'")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Output limit"));
    assert_eq!(container_count(&sandbox).await, 0);
}
#[tokio::test]
#[ignore = "Requires Docker and python:3.12-alpine image"]
async fn docker_cancellation_removes_container() {
    let sandbox = DockerSandbox::default();
    let cloned = sandbox.clone();
    let task = tokio::spawn(async move {
        cloned
            .execute("python -c 'import time; time.sleep(30)'")
            .await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while container_count(&sandbox).await == 0 {
        assert!(
            !task.is_finished(),
            "execution finished early: {:?}",
            task.await
        );
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    task.abort();
    let _ = task.await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(container_count(&sandbox).await, 0);
}

#[tokio::test]
#[ignore = "Requires Docker and python:3.12-alpine image"]
async fn docker_reaper_removes_only_expired_sdk_containers() {
    let docker = bollard::Docker::connect_with_local_defaults().unwrap();
    let instance = uuid::Uuid::new_v4().simple().to_string();
    let create = |expiry: &str| bollard::container::Config::<String> {
        image: Some("python:3.12-alpine".into()),
        labels: Some(HashMap::from([
            ("dasein.sdk.instance".into(), instance.clone()),
            ("dasein.sdk.expires_ms".into(), expiry.into()),
        ])),
        ..Default::default()
    };
    let expired = docker
        .create_container(
            None::<bollard::container::CreateContainerOptions<String>>,
            create("0"),
        )
        .await
        .unwrap();
    let future = docker
        .create_container(
            None::<bollard::container::CreateContainerOptions<String>>,
            create(&u64::MAX.to_string()),
        )
        .await
        .unwrap();
    let result = DockerSandbox::reap_expired().await;
    let missing = docker.inspect_container(&expired.id, None).await.is_err();
    let exists = docker.inspect_container(&future.id, None).await.is_ok();
    // Always clean up the test fixture, even if the assertions fail.
    docker.remove_container(&future.id, None).await.unwrap();
    let _ = docker.remove_container(&expired.id, None).await;
    assert!(result.is_ok());
    assert!(missing);
    assert!(exists);
}

#[tokio::test]
#[ignore = "Requires Docker and python:3.12-alpine image"]
async fn docker_memory_limit_is_enforced_by_kernel() {
    let sandbox = DockerSandbox::builder().memory_limit("64m").build();
    let result = sandbox
        .execute("python -c 'x=bytearray(256*1024*1024); print(len(x))'")
        .await
        .unwrap();
    assert_ne!(result.exit_code, 0, "allocation exceeded the memory policy");
    assert!(!result.stdout.contains("268435456"));
    assert_eq!(container_count(&sandbox).await, 0);
}
