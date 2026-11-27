//! Dashboard HTTP handlers

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
    Extension, Form,
};
use serde::Deserialize;
use tower_cookies::{Cookie, Cookies};

use super::auth::{cookie_secure, get_admin_password, verify_admin_password, SESSION_COOKIE};
use super::templates::{
    ApiKeysTemplate, DashboardTemplate, LayoutData, LoginTemplate, PoolStats, ProvidersTemplate,
    SandboxesTemplate, UsageTemplate,
};
use crate::state::AppState;
use gateway_core::storage::UsageStats;

// ==================== Login ====================

/// GET /admin/login - Show login page
pub async fn login_page() -> impl IntoResponse {
    LoginTemplate { error: None }
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub password: String,
}

/// POST /admin/login - Process login
pub async fn login_submit(
    State(state): State<AppState>,
    Extension(cookies): Extension<Cookies>,
    extensions: axum::http::Extensions,
    headers: axum::http::HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    // Failed logins are counted per client address: the peer, or the address
    // a trusted reverse proxy reports for it.
    let peer = extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.0.ip());
    let client = peer.map_or_else(
        || "unknown".to_string(),
        |peer| crate::client_addr::client_ip(peer, &headers, &state.trusted_proxies).to_string(),
    );
    let forwarded_https = peer.is_some_and(|peer| {
        crate::client_addr::forwarded_https(peer, &headers, &state.trusted_proxies)
    });
    if state.login_throttle.is_blocked(&client) {
        tracing::warn!(%client, "Dashboard login refused: too many failed attempts");
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            LoginTemplate {
                error: Some("Too many failed attempts. Try again in a few minutes.".to_string()),
            },
        )
            .into_response();
    }

    // Check if admin password is configured
    let Some(expected_password) = get_admin_password() else {
        return LoginTemplate {
            error: Some("Admin password not configured. Set GATEWAY_ADMIN_PASSWORD.".to_string()),
        }
        .into_response();
    };

    // Verify password
    if !verify_admin_password(&form.password, &expected_password) {
        state.login_throttle.record_failure(&client);
        return LoginTemplate {
            error: Some("Invalid password".to_string()),
        }
        .into_response();
    }

    // Create session
    if let Some(ref storage) = state.storage {
        match storage.create_admin_session().await {
            Ok(session) => {
                let cookie = Cookie::build((SESSION_COOKIE, session.id))
                    .path("/admin")
                    .http_only(true)
                    // Never sent on requests started by another site.
                    .same_site(tower_cookies::cookie::SameSite::Strict)
                    .secure(cookie_secure(&headers, forwarded_https))
                    .build();
                cookies.add(cookie);
                state.login_throttle.record_success(&client);
                return Redirect::to("/admin").into_response();
            }
            Err(e) => {
                tracing::error!("Failed to create session: {}", e);
                return LoginTemplate {
                    error: Some("Internal error".to_string()),
                }
                .into_response();
            }
        }
    }

    LoginTemplate {
        error: Some("Storage not configured".to_string()),
    }
    .into_response()
}

/// POST /admin/logout - Logout
pub async fn logout(
    State(state): State<AppState>,
    Extension(cookies): Extension<Cookies>,
) -> impl IntoResponse {
    if let Some(session_cookie) = cookies.get(SESSION_COOKIE) {
        if let Some(ref storage) = state.storage {
            let _ = storage.delete_admin_session(session_cookie.value()).await;
        }
        // Same path as when it was set, or the browser keeps it.
        cookies.remove(Cookie::build(SESSION_COOKIE).path("/admin").build());
    }
    Redirect::to("/admin/login")
}

// ==================== Dashboard ====================

/// GET /admin - Main dashboard
pub async fn dashboard(State(state): State<AppState>) -> impl IntoResponse {
    let storage = match &state.storage {
        Some(s) => s,
        None => {
            return DashboardTemplate {
                layout: LayoutData {
                    title: "Dashboard".to_string(),
                    active_page: "dashboard".to_string(),
                },
                stats: UsageStats::default(),
                pool_stats: PoolStats::default(),
                logs: vec![],
            }
        }
    };

    // Get usage stats for last 24 hours
    let stats = storage.get_usage_stats(24).await.unwrap_or_default();

    // Get recent logs
    let logs = storage.get_usage_logs(10).await.unwrap_or_default();

    // Get pool stats from sandbox pool
    let pool_stats = {
        let stats = state.sandbox_pool.stats().await;
        PoolStats {
            available: stats.available,
            in_use: stats.in_use,
            max_total: stats.max_total,
        }
    };

    DashboardTemplate {
        layout: LayoutData {
            title: "Dashboard".to_string(),
            active_page: "dashboard".to_string(),
        },
        stats,
        pool_stats,
        logs,
    }
}

/// GET /admin/stats - HTMX partial for stats refresh
pub async fn stats_partial(State(state): State<AppState>) -> impl IntoResponse {
    let stats = if let Some(ref storage) = state.storage {
        storage.get_usage_stats(24).await.unwrap_or_default()
    } else {
        UsageStats::default()
    };

    super::templates::StatsCardsPartial { stats }
}

/// GET /admin/pool-stats - HTMX partial for pool stats refresh
pub async fn pool_stats_partial(State(state): State<AppState>) -> impl IntoResponse {
    let stats = state.sandbox_pool.stats().await;
    super::templates::PoolStatsPartial {
        pool_stats: PoolStats {
            available: stats.available,
            in_use: stats.in_use,
            max_total: stats.max_total,
        },
    }
}

// ==================== Providers ====================

/// GET /admin/providers - List providers
pub async fn providers(State(state): State<AppState>) -> impl IntoResponse {
    let providers = if let Some(ref storage) = state.storage {
        storage.list_providers().await.unwrap_or_default()
    } else {
        vec![]
    };

    ProvidersTemplate {
        layout: LayoutData {
            title: "Providers".to_string(),
            active_page: "providers".to_string(),
        },
        providers,
        message: None,
    }
}

#[derive(Debug, Deserialize)]
pub struct AddProviderForm {
    pub name: String,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
}

/// POST /admin/providers/add - Add provider
pub async fn add_provider(
    State(state): State<AppState>,
    Form(form): Form<AddProviderForm>,
) -> impl IntoResponse {
    let message = if let Some(ref storage) = state.storage {
        match storage
            .create_provider(
                &form.name,
                form.api_key.as_deref(),
                form.api_base.as_deref(),
            )
            .await
        {
            Ok(_) => Some("Provider added successfully".to_string()),
            Err(e) => Some(format!("Error: {}", e)),
        }
    } else {
        Some("Storage not configured".to_string())
    };

    let providers = if let Some(ref storage) = state.storage {
        storage.list_providers().await.unwrap_or_default()
    } else {
        vec![]
    };

    ProvidersTemplate {
        layout: LayoutData {
            title: "Providers".to_string(),
            active_page: "providers".to_string(),
        },
        providers,
        message,
    }
}

/// POST /admin/providers/{id}/toggle - Toggle provider
pub async fn toggle_provider(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if let Some(ref storage) = state.storage {
        let _ = storage.toggle_provider(&id).await;
        if let Ok(provider) = storage.get_provider(&id).await {
            return super::templates::ProviderRowPartial { provider }.into_response();
        }
    }
    Html("Provider not found").into_response()
}

/// POST /admin/providers/{id}/delete - Delete provider
pub async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(ref storage) = state.storage {
        let _ = storage.delete_provider(&id).await;
    }
    Html("") // Return empty to remove row
}

// ==================== API Keys ====================

/// GET /admin/keys - List API keys
pub async fn api_keys(State(state): State<AppState>) -> impl IntoResponse {
    let api_keys = if let Some(ref storage) = state.storage {
        storage.list_api_keys().await.unwrap_or_default()
    } else {
        vec![]
    };

    ApiKeysTemplate {
        layout: LayoutData {
            title: "API Keys".to_string(),
            active_page: "keys".to_string(),
        },
        api_keys,
        new_key: None,
        message: None,
    }
}

#[derive(Debug, Deserialize)]
pub struct GenerateKeyForm {
    pub name: String,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
}

/// POST /admin/keys/generate - Generate new API key
pub async fn generate_key(
    State(state): State<AppState>,
    Form(form): Form<GenerateKeyForm>,
) -> impl IntoResponse {
    let (new_key, message) = if let Some(ref storage) = state.storage {
        match storage
            .create_api_key(&form.name, form.rate_limit_rpm, form.rate_limit_tpm)
            .await
        {
            Ok((_, raw_key)) => (
                Some(raw_key),
                Some("API key generated! Copy it now, it won't be shown again.".to_string()),
            ),
            Err(e) => (None, Some(format!("Error: {}", e))),
        }
    } else {
        (None, Some("Storage not configured".to_string()))
    };

    let api_keys = if let Some(ref storage) = state.storage {
        storage.list_api_keys().await.unwrap_or_default()
    } else {
        vec![]
    };

    ApiKeysTemplate {
        layout: LayoutData {
            title: "API Keys".to_string(),
            active_page: "keys".to_string(),
        },
        api_keys,
        new_key,
        message,
    }
}

/// POST /admin/keys/{id}/toggle - Toggle API key
pub async fn toggle_key(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if let Some(ref storage) = state.storage {
        let _ = storage.toggle_api_key(&id).await;
        if let Ok(key) = storage.get_api_key(&id).await {
            return super::templates::ApiKeyRowPartial { key }.into_response();
        }
    }
    Html("Key not found").into_response()
}

/// POST /admin/keys/{id}/revoke - Revoke API key
pub async fn revoke_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(ref storage) = state.storage {
        let _ = storage.revoke_api_key(&id).await;
    }
    Html("") // Return empty to remove row
}

// ==================== Sandboxes (Remote VM) ====================

use super::templates::{RuntimeInfo, TestResult, VmStatusDisplay};
use gateway_core::sandbox::Runtime;

/// GET /admin/sandboxes - VM Status and Control
pub async fn sandboxes(State(state): State<AppState>) -> impl IntoResponse {
    let orchestrator_enabled = state.orchestrator.is_some();
    let vm_host = std::env::var("FIRECRACKER_REMOTE_URL")
        .unwrap_or_else(|_| "(FIRECRACKER_REMOTE_URL not set)".to_string());

    // Get VM status if orchestrator is enabled
    let vm_status = if let Some(ref orchestrator) = state.orchestrator {
        if orchestrator.connect().await.is_ok() {
            orchestrator
                .status()
                .await
                .map(|s| VmStatusDisplay {
                    firecracker_running: s.firecracker_running,
                    agent_healthy: s.agent_healthy,
                    server_uptime: s.server_uptime,
                })
                .unwrap_or_default()
        } else {
            VmStatusDisplay::default()
        }
    } else {
        // Check via code executor if remote is configured
        let healthy = state.code_executor.is_ready().await;
        VmStatusDisplay {
            firecracker_running: healthy,
            agent_healthy: healthy,
            server_uptime: "Unknown".to_string(),
        }
    };

    // Available runtimes
    let runtimes = vec![
        RuntimeInfo {
            name: "python".to_string(),
            version: "3.10".to_string(),
        },
        RuntimeInfo {
            name: "node".to_string(),
            version: "20.x".to_string(),
        },
        RuntimeInfo {
            name: "go".to_string(),
            version: "1.21".to_string(),
        },
        RuntimeInfo {
            name: "rust".to_string(),
            version: "1.93".to_string(),
        },
        RuntimeInfo {
            name: "bash".to_string(),
            version: "5.x".to_string(),
        },
        RuntimeInfo {
            name: "typescript".to_string(),
            version: "via Node".to_string(),
        },
    ];

    SandboxesTemplate {
        layout: LayoutData {
            title: "Sandboxes".to_string(),
            active_page: "sandboxes".to_string(),
        },
        vm_host,
        vm_status,
        runtimes,
        orchestrator_enabled,
        message: None,
        test_result: None,
    }
}

/// POST /admin/sandboxes/vm/start - Start VM
pub async fn vm_start(State(state): State<AppState>) -> Response {
    if let Some(ref orchestrator) = state.orchestrator {
        if orchestrator.connect().await.is_ok() {
            let _ = orchestrator.start_vm().await;
        }
    }
    Redirect::to("/admin/sandboxes").into_response()
}

/// POST /admin/sandboxes/vm/stop - Stop VM
pub async fn vm_stop(State(state): State<AppState>) -> Response {
    if let Some(ref orchestrator) = state.orchestrator {
        if orchestrator.connect().await.is_ok() {
            let _ = orchestrator.stop_vm().await;
        }
    }
    Redirect::to("/admin/sandboxes").into_response()
}

/// POST /admin/sandboxes/vm/restart - Restart VM
pub async fn vm_restart(State(state): State<AppState>) -> Response {
    if let Some(ref orchestrator) = state.orchestrator {
        if orchestrator.connect().await.is_ok() {
            let _ = orchestrator.restart_vm().await;
        }
    }
    Redirect::to("/admin/sandboxes").into_response()
}

#[derive(Debug, Deserialize)]
pub struct TestCodeForm {
    pub runtime: String,
    pub code: String,
}

/// POST /admin/sandboxes/test - Execute test code
pub async fn test_code(
    State(state): State<AppState>,
    Form(form): Form<TestCodeForm>,
) -> impl IntoResponse {
    let orchestrator_enabled = state.orchestrator.is_some();
    let vm_host = std::env::var("FIRECRACKER_REMOTE_URL")
        .unwrap_or_else(|_| "(FIRECRACKER_REMOTE_URL not set)".to_string());

    // Parse runtime
    let runtime = match form.runtime.as_str() {
        "python" => Runtime::Python,
        "node" => Runtime::Node,
        "bash" => Runtime::Bash,
        "go" => Runtime::Go,
        "rust" => Runtime::Rust,
        "typescript" => Runtime::TypeScript,
        _ => Runtime::Python,
    };

    // The submitted code itself stays out of the logs.
    tracing::info!(
        runtime = ?runtime,
        code_len = form.code.len(),
        "Dashboard test_code executing"
    );

    // Execute code
    let test_result = match state
        .code_executor
        .execute(runtime.clone(), form.code, vec![], Some(30))
        .await
    {
        Ok(result) => Some(TestResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
            duration_ms: result.duration_ms,
        }),
        Err(e) => Some(TestResult {
            stdout: String::new(),
            stderr: format!("Error: {}", e),
            exit_code: -1,
            duration_ms: 0,
        }),
    };

    // Get VM status
    let vm_status = if let Some(ref orchestrator) = state.orchestrator {
        if orchestrator.connect().await.is_ok() {
            orchestrator
                .status()
                .await
                .map(|s| VmStatusDisplay {
                    firecracker_running: s.firecracker_running,
                    agent_healthy: s.agent_healthy,
                    server_uptime: s.server_uptime,
                })
                .unwrap_or_default()
        } else {
            VmStatusDisplay::default()
        }
    } else {
        let healthy = state.code_executor.is_ready().await;
        VmStatusDisplay {
            firecracker_running: healthy,
            agent_healthy: healthy,
            server_uptime: "Unknown".to_string(),
        }
    };

    let runtimes = vec![
        RuntimeInfo {
            name: "python".to_string(),
            version: "3.10".to_string(),
        },
        RuntimeInfo {
            name: "node".to_string(),
            version: "20.x".to_string(),
        },
        RuntimeInfo {
            name: "go".to_string(),
            version: "1.21".to_string(),
        },
        RuntimeInfo {
            name: "rust".to_string(),
            version: "1.93".to_string(),
        },
        RuntimeInfo {
            name: "bash".to_string(),
            version: "5.x".to_string(),
        },
        RuntimeInfo {
            name: "typescript".to_string(),
            version: "via Node".to_string(),
        },
    ];

    SandboxesTemplate {
        layout: LayoutData {
            title: "Sandboxes".to_string(),
            active_page: "sandboxes".to_string(),
        },
        vm_host,
        vm_status,
        runtimes,
        orchestrator_enabled,
        message: None,
        test_result,
    }
}

// ==================== Usage ====================

/// GET /admin/usage - Usage logs
pub async fn usage(State(state): State<AppState>) -> impl IntoResponse {
    let (stats, logs) = if let Some(ref storage) = state.storage {
        (
            storage.get_usage_stats(24).await.unwrap_or_default(),
            storage.get_usage_logs(100).await.unwrap_or_default(),
        )
    } else {
        (UsageStats::default(), vec![])
    };

    UsageTemplate {
        layout: LayoutData {
            title: "Usage".to_string(),
            active_page: "usage".to_string(),
        },
        stats,
        logs,
    }
}

/// GET /admin/recent-logs - HTMX partial for recent logs
pub async fn recent_logs_partial(State(state): State<AppState>) -> impl IntoResponse {
    let logs = if let Some(ref storage) = state.storage {
        storage.get_usage_logs(10).await.unwrap_or_default()
    } else {
        vec![]
    };

    super::templates::RecentLogsPartial { logs }
}
