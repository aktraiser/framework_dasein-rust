//! API routes

pub mod chat;
pub mod embeddings;
pub mod filesystem;
pub mod health;
pub mod sandbox;
pub mod vm;

use axum::{response::Html, routing::{get, post}, Router};

use crate::dashboard::create_dashboard_router;
use crate::state::AppState;

/// GET /
async fn index() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html>
<head>
    <title>Agentic Gateway</title>
    <style>
        body { font-family: -apple-system, sans-serif; max-width: 800px; margin: 50px auto; padding: 20px; background: #1a1a2e; color: #eee; }
        h1 { color: #00d9ff; }
        a { color: #00d9ff; }
        code { background: #16213e; padding: 2px 8px; border-radius: 4px; }
        .endpoint { background: #16213e; padding: 15px; margin: 10px 0; border-radius: 8px; border-left: 4px solid #00d9ff; }
        .method { color: #00ff88; font-weight: bold; }
    </style>
</head>
<body>
    <h1>🚀 Agentic Gateway v0.1.0</h1>
    <p>High-performance Rust gateway for LLM providers and Firecracker sandboxes.</p>

    <h2>Endpoints</h2>

    <div class="endpoint">
        <span class="method">GET</span> <a href="/health">/health</a> - Health check
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <a href="/metrics">/metrics</a> - Prometheus metrics
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <a href="/v1/models">/v1/models</a> - List models
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/v1/chat/completions</code> - Chat completion (OpenAI-compatible)
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/v1/embeddings</code> - Generate embeddings
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/v1/sandboxes</code> - Create sandbox
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/v1/execute</code> - Execute code (remote Firecracker)
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/v1/files</code> - Upload file
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/v1/files?path=/path</code> - Download file or list directory
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/v1/vm/start</code> - Start Firecracker VM
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/v1/vm/status</code> - Get VM status
    </div>

    <h2>Example</h2>
    <pre><code>curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "Hello!"}]}'</code></pre>
</body>
</html>"#)
}

/// Create the main API router
pub fn create_router(state: AppState) -> Router {
    // Create dashboard router
    let dashboard_router = create_dashboard_router(state.clone());

    Router::new()
        // Root endpoint
        .route("/", get(index))
        // OpenAI-compatible endpoints
        .route("/v1/chat/completions", post(chat::chat_completions))
        .route("/v1/embeddings", post(embeddings::create_embeddings))
        .route("/v1/models", get(chat::list_models))
        // Sandbox endpoints
        .route("/v1/sandboxes", post(sandbox::create_sandbox))
        .route("/v1/sandboxes/{id}", get(sandbox::get_sandbox))
        .route("/v1/sandboxes/{id}/execute", post(sandbox::execute_code))
        .route(
            "/v1/sandboxes/{id}",
            axum::routing::delete(sandbox::delete_sandbox),
        )
        // Direct code execution (no sandbox management)
        .route("/v1/execute", post(sandbox::execute_direct))
        // Filesystem API
        .route("/v1/files", post(filesystem::upload_file))
        .route("/v1/files", get(filesystem::get_file))
        .route("/v1/files", axum::routing::delete(filesystem::delete_file))
        .route("/v1/files/mkdir", post(filesystem::create_directory))
        .route("/v1/files/stat", get(filesystem::stat_file))
        .route("/v1/files/exists", get(filesystem::file_exists))
        // VM Management API
        .route("/v1/vm/start", post(vm::start_vm))
        .route("/v1/vm/stop", post(vm::stop_vm))
        .route("/v1/vm/restart", post(vm::restart_vm))
        .route("/v1/vm/status", get(vm::vm_status))
        .route("/v1/vm/ensure", post(vm::ensure_vm))
        .route("/v1/vm/exec", post(vm::exec_ssh))
        // Health endpoints
        .route("/health", get(health::health_check))
        .route("/metrics", get(health::metrics))
        // Merge dashboard routes
        .merge(dashboard_router)
        .with_state(state)
}
