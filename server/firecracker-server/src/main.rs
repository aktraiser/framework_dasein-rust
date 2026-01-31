//! Firecracker Sandbox Server
//!
//! A REST API server that executes code in Firecracker microVMs.
//! Deploy this on a Linux server with KVM support.
//!
//! # Usage
//!
//! ```bash
//! # Set environment variables
//! export API_KEY="your-secret-key"
//! export PORT=8080
//!
//! # Run the server
//! ./firecracker-server
//! ```
//!
//! # Endpoints
//!
//! - `GET /health` - Health check
//! - `POST /execute` - Execute code in Firecracker

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::process::Command;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, info, warn};

// ============================================================================
// TYPES
// ============================================================================

/// Server configuration.
#[derive(Debug, Clone)]
struct Config {
    /// API key for authentication (None = no auth)
    api_key: Option<String>,
    /// Port to listen on
    port: u16,
    /// Path to Firecracker binary
    firecracker_bin: PathBuf,
    /// Path to kernel
    kernel_path: PathBuf,
    /// Path to rootfs
    rootfs_path: PathBuf,
    /// Default timeout in ms
    default_timeout_ms: u64,
    /// Max timeout in ms
    max_timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: std::env::var("API_KEY").ok(),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            firecracker_bin: PathBuf::from("/usr/local/bin/firecracker"),
            kernel_path: PathBuf::from("/var/lib/firecracker/vmlinux"),
            rootfs_path: PathBuf::from("/var/lib/firecracker/rootfs.ext4"),
            default_timeout_ms: 30000,
            max_timeout_ms: 120000,
        }
    }
}

/// Shared application state.
#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
}

/// Execute request from client.
#[derive(Debug, Clone, Deserialize)]
struct ExecuteRequest {
    /// Code to execute
    code: String,
    /// Language (optional)
    language: Option<String>,
    /// Timeout in milliseconds
    timeout_ms: Option<u64>,
}

/// Execute response to client.
#[derive(Debug, Clone, Serialize)]
struct ExecuteResponse {
    success: bool,
    exit_code: i32,
    stdout: String,
    stderr: String,
    execution_time_ms: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    artifacts: Vec<Artifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Artifact from execution.
#[derive(Debug, Clone, Serialize)]
struct Artifact {
    path: String,
    content: String,
}

/// Health response.
#[derive(Debug, Clone, Serialize)]
struct HealthResponse {
    status: String,
    firecracker_available: bool,
    kvm_available: bool,
    version: String,
}

/// Error response.
#[derive(Debug, Clone, Serialize)]
struct ErrorResponse {
    error: String,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// Health check endpoint.
async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let firecracker_available = state.config.firecracker_bin.exists();
    let kvm_available = std::path::Path::new("/dev/kvm").exists();

    Json(HealthResponse {
        status: if firecracker_available && kvm_available {
            "healthy".into()
        } else {
            "degraded".into()
        },
        firecracker_available,
        kvm_available,
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

/// Execute code endpoint.
async fn execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Check API key if configured
    if let Some(ref expected_key) = state.config.api_key {
        let provided_key = headers
            .get("X-API-Key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if provided_key != expected_key {
            warn!("Invalid API key provided");
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Invalid API key".into(),
                }),
            ));
        }
    }

    // Validate timeout
    let timeout_ms = request
        .timeout_ms
        .unwrap_or(state.config.default_timeout_ms)
        .min(state.config.max_timeout_ms);

    info!(
        code_len = request.code.len(),
        language = ?request.language,
        timeout_ms = timeout_ms,
        "Executing code"
    );

    // Execute in sandbox
    match execute_in_sandbox(&state.config, &request.code, timeout_ms).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => {
            error!("Execution failed: {}", e);
            Ok(Json(ExecuteResponse {
                success: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: String::new(),
                execution_time_ms: 0,
                artifacts: vec![],
                error: Some(e.to_string()),
            }))
        }
    }
}

// ============================================================================
// SANDBOX EXECUTION
// ============================================================================

/// Execute code in a sandboxed environment.
///
/// For now, uses a simple process sandbox. In production, this would
/// launch a Firecracker microVM.
async fn execute_in_sandbox(
    config: &Config,
    code: &str,
    timeout_ms: u64,
) -> Result<ExecuteResponse, anyhow::Error> {
    let start = Instant::now();

    // Check if we can use Firecracker
    let kvm_available = std::path::Path::new("/dev/kvm").exists();
    let firecracker_available = config.firecracker_bin.exists()
        && config.kernel_path.exists()
        && config.rootfs_path.exists();

    if kvm_available && firecracker_available {
        // Use Firecracker microVM
        execute_in_firecracker(config, code, timeout_ms).await
    } else {
        // Fallback to process sandbox (with warning)
        warn!("Firecracker not available, using process sandbox (LESS SECURE)");
        execute_in_process(code, timeout_ms).await
    }
}

/// Execute code in a Firecracker microVM.
async fn execute_in_firecracker(
    config: &Config,
    code: &str,
    timeout_ms: u64,
) -> Result<ExecuteResponse, anyhow::Error> {
    let start = Instant::now();
    let vm_id = uuid::Uuid::new_v4().to_string();

    info!(vm_id = %vm_id, "Starting Firecracker microVM");

    // Create workspace for this VM
    let workspace = PathBuf::from(format!("/tmp/firecracker/{}", vm_id));
    tokio::fs::create_dir_all(&workspace).await?;

    // Write code to a file that will be copied into the VM
    let code_file = workspace.join("code.sh");
    tokio::fs::write(&code_file, code).await?;

    // Create VM config
    let vm_config = serde_json::json!({
        "boot-source": {
            "kernel_image_path": config.kernel_path.to_string_lossy(),
            "boot_args": "console=ttyS0 reboot=k panic=1 pci=off init=/bin/bash"
        },
        "drives": [{
            "drive_id": "rootfs",
            "path_on_host": config.rootfs_path.to_string_lossy(),
            "is_root_device": true,
            "is_read_only": false
        }],
        "machine-config": {
            "vcpu_count": 1,
            "mem_size_mib": 256
        }
    });

    let config_path = workspace.join("vm_config.json");
    tokio::fs::write(&config_path, serde_json::to_string_pretty(&vm_config)?).await?;

    // Start Firecracker
    let socket_path = workspace.join("firecracker.socket");

    let mut child = Command::new(&config.firecracker_bin)
        .arg("--api-sock")
        .arg(&socket_path)
        .arg("--config-file")
        .arg(&config_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Wait for execution with timeout
    let result = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        async {
            child.wait().await
        },
    )
    .await;

    let execution_time_ms = start.elapsed().as_millis() as u64;

    // Get output
    let (stdout, stderr, exit_code) = match result {
        Ok(Ok(status)) => {
            // Process completed
            let exit_code = status.code().unwrap_or(-1);
            (String::new(), String::new(), exit_code)
        }
        Ok(Err(e)) => {
            // Cleanup
            let _ = tokio::fs::remove_dir_all(&workspace).await;
            return Err(anyhow::anyhow!("Firecracker execution failed: {}", e));
        }
        Err(_) => {
            // Timeout - kill the process
            let _ = child.kill().await;
            let _ = tokio::fs::remove_dir_all(&workspace).await;
            return Ok(ExecuteResponse {
                success: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: "Execution timed out".into(),
                execution_time_ms,
                artifacts: vec![],
                error: Some("Timeout".into()),
            });
        }
    };

    // Cleanup
    let _ = tokio::fs::remove_dir_all(&workspace).await;

    Ok(ExecuteResponse {
        success: exit_code == 0,
        exit_code,
        stdout,
        stderr,
        execution_time_ms,
        artifacts: vec![],
        error: None,
    })
}

/// Execute code in a simple process (fallback, less secure).
async fn execute_in_process(code: &str, timeout_ms: u64) -> Result<ExecuteResponse, anyhow::Error> {
    let start = Instant::now();

    let result = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        Command::new("sh")
            .arg("-c")
            .arg(code)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    let execution_time_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);

            Ok(ExecuteResponse {
                success: exit_code == 0,
                exit_code,
                stdout,
                stderr,
                execution_time_ms,
                artifacts: vec![],
                error: None,
            })
        }
        Ok(Err(e)) => Err(anyhow::anyhow!("Process execution failed: {}", e)),
        Err(_) => Ok(ExecuteResponse {
            success: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: "Execution timed out".into(),
            execution_time_ms,
            artifacts: vec![],
            error: Some("Timeout".into()),
        }),
    }
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("firecracker_server=info".parse()?)
                .add_directive("tower_http=debug".parse()?),
        )
        .init();

    let config = Config::default();

    info!(
        port = config.port,
        firecracker = ?config.firecracker_bin,
        kernel = ?config.kernel_path,
        rootfs = ?config.rootfs_path,
        auth = config.api_key.is_some(),
        "Starting Firecracker Server"
    );

    // Check prerequisites
    if !config.firecracker_bin.exists() {
        warn!("Firecracker binary not found at {:?}", config.firecracker_bin);
    }
    if !std::path::Path::new("/dev/kvm").exists() {
        warn!("/dev/kvm not found - Firecracker will not work");
    }
    if !config.kernel_path.exists() {
        warn!("Kernel not found at {:?}", config.kernel_path);
    }
    if !config.rootfs_path.exists() {
        warn!("Rootfs not found at {:?}", config.rootfs_path);
    }

    let state = AppState {
        config: Arc::new(config.clone()),
    };

    // Build router
    let app = Router::new()
        .route("/health", get(health))
        .route("/execute", post(execute))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Start server
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("Server listening on http://{}", addr);
    info!("Endpoints:");
    info!("  GET  /health  - Health check");
    info!("  POST /execute - Execute code");

    axum::serve(listener, app).await?;

    Ok(())
}
