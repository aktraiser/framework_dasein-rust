use dasein_agentic_sandbox::{ProcessSandbox, Sandbox};

#[tokio::test]
async fn audit_timeout_stops_process_side_effects() {
    let marker = std::env::temp_dir()
        .join(format!("dasein-audit-timeout-{}", uuid::Uuid::new_v4()))
        .to_string_lossy()
        .into_owned();
    let command = format!("sleep 0.4; printf survived > '{marker}'");
    let result = ProcessSandbox::new()
        .with_timeout(50)
        .execute(&command)
        .await;
    assert!(result.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let exists = std::path::Path::new(&marker).exists();
    let _ = std::fs::remove_file(&marker);
    assert!(
        !exists,
        "command continued after timeout and wrote its marker"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn cancellation_stops_descendants() {
    let marker = std::env::temp_dir().join(format!("dasein-cancel-{}", uuid::Uuid::new_v4()));
    let code = format!("sleep 0.4; printf survived > '{}'", marker.display());
    let task = tokio::spawn(async move { ProcessSandbox::new().execute(&code).await });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    task.abort();
    let _ = task.await;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert!(!marker.exists(), "descendant survived cancellation");
}
