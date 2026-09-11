//! Authenticated HTTP adapter for the SDK's real Linux/KVM microVM backend.
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use dasein_agentic_sandbox::{FirecrackerConfig, FirecrackerSandbox, Sandbox, SandboxError};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct AppState {
    key: Arc<String>,
    config: FirecrackerConfig,
    capacity: Arc<Semaphore>,
}
#[derive(Deserialize)]
struct ExecuteRequest {
    code: String,
    language: Option<String>,
    timeout_ms: Option<u64>,
}
#[derive(Serialize)]
struct ExecuteResponse {
    success: bool,
    exit_code: i32,
    stdout: String,
    stderr: String,
    execution_time_ms: u64,
}
type ApiError = (StatusCode, Json<serde_json::Value>);
fn error(status: StatusCode, message: impl ToString) -> ApiError {
    (
        status,
        Json(serde_json::json!({"error":message.to_string()})),
    )
}
fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.get("X-API-Key").and_then(|v| v.to_str().ok()) != Some(state.key.as_str()) {
        return Err(error(StatusCode::UNAUTHORIZED, "Invalid API key"));
    }
    Ok(())
}
async fn health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let ready = FirecrackerSandbox::with_config(state.config.clone())
        .check_prerequisites()
        .is_ok();
    Ok(Json(
        serde_json::json!({"status":if ready {"ok"} else {"unavailable"},"firecracker_available":ready,"kvm_available":ready,"version":env!("CARGO_PKG_VERSION"),"readiness":"prerequisites; each execution verifies guest boot"}),
    ))
}
fn guest_command(code: &str, language: Option<&str>) -> Result<String, ApiError> {
    if code.len() > 256 * 1024 {
        return Err(error(StatusCode::BAD_REQUEST, "Code exceeds 256 KiB"));
    }
    match language {
        None | Some("sh" | "shell") => Ok(code.to_owned()),
        Some("python" | "python3") => Ok(format!("python3 -c '{}'", code.replace('\'', "'\\''"))),
        _ => Err(error(
            StatusCode::BAD_REQUEST,
            "Supported languages: sh, shell, python, python3",
        )),
    }
}
async fn execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    authorize(&state, &headers)?;
    let command = guest_command(&request.code, request.language.as_deref())?;
    let timeout = request.timeout_ms.unwrap_or(30_000);
    if !(1..=120_000).contains(&timeout) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "timeout_ms must be 1..120000",
        ));
    }
    let _permit = state
        .capacity
        .try_acquire()
        .map_err(|_| error(StatusCode::TOO_MANY_REQUESTS, "MicroVM capacity reached"))?;
    let sandbox = FirecrackerSandbox::with_config(FirecrackerConfig {
        timeout: Duration::from_millis(timeout),
        ..state.config
    });
    let result = sandbox.execute(&command).await.map_err(|e| {
        let status = match &e {
            SandboxError::Timeout => StatusCode::REQUEST_TIMEOUT,
            SandboxError::ConfigError(_) => StatusCode::BAD_REQUEST,
            SandboxError::NotAvailable(_) | SandboxError::NotReady(_) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            _ => StatusCode::BAD_GATEWAY,
        };
        tracing::warn!(error=%e,"MicroVM execution failed");
        error(
            status,
            match status {
                StatusCode::REQUEST_TIMEOUT => "Execution timed out",
                StatusCode::BAD_REQUEST => "Invalid execution configuration",
                StatusCode::SERVICE_UNAVAILABLE => "MicroVM backend unavailable",
                _ => "MicroVM execution failed",
            },
        )
    })?;
    Ok(Json(ExecuteResponse {
        success: result.exit_code == 0,
        exit_code: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
        execution_time_ms: result.execution_time_ms,
    }))
}
fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/execute", post(execute))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(state)
}
fn asset(variable: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(variable).map_or(default, PathBuf::from)
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let key = std::env::var("API_KEY").map_err(|_| anyhow::anyhow!("API_KEY is required"))?;
    anyhow::ensure!(!key.trim().is_empty(), "API_KEY cannot be empty");
    let defaults = FirecrackerConfig::default();
    let config = FirecrackerConfig {
        firecracker_bin: asset("FIRECRACKER_BIN", defaults.firecracker_bin),
        kernel_path: asset("KERNEL_PATH", defaults.kernel_path),
        rootfs_path: asset("ROOTFS_PATH", defaults.rootfs_path),
        workspace: asset("WORKSPACE", defaults.workspace),
        ..defaults
    };
    FirecrackerSandbox::with_config(config.clone()).check_prerequisites()?;
    let address = std::env::var("BIND").unwrap_or_else(|_| {
        format!(
            "127.0.0.1:{}",
            std::env::var("PORT").unwrap_or_else(|_| "8080".into())
        )
    });
    let state = AppState {
        key: Arc::new(key),
        capacity: Arc::new(Semaphore::new(config.max_concurrent)),
        config,
    };
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(%address,"Firecracker adapter ready");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use tower::ServiceExt;
    #[test]
    fn quotes_python_and_rejects_unsupported_languages() {
        assert_eq!(
            guest_command("print('ok')", Some("python")).unwrap(),
            r"python3 -c 'print('\''ok'\'')'"
        );
        assert!(guest_command("x", Some("rust")).is_err());
    }
    #[tokio::test]
    async fn auth_and_validation_precede_vm_launch() {
        let state = AppState {
            key: Arc::new("test-secret".into()),
            config: FirecrackerConfig::default(),
            capacity: Arc::new(Semaphore::new(1)),
        };
        let app = router(state);
        for (key, body, expected) in [
            ("wrong", r#"{"code":"echo test"}"#, 401),
            ("test-secret", r#"{"code":"echo test","timeout_ms":0}"#, 400),
            ("test-secret", r#"{"code":"x","language":"ruby"}"#, 400),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/execute")
                        .header("content-type", "application/json")
                        .header("X-API-Key", key)
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected);
        }
        let response = app
            .oneshot(
                Request::get("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
}
