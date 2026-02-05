//! Askama templates for the admin dashboard

use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use gateway_core::storage::{ApiKey, Provider, UsageLog, UsageStats};

/// Helper function to convert Askama templates to Axum responses
fn render_template<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("Template rendering error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Template error: {}", e),
            )
                .into_response()
        }
    }
}

/// Base layout data shared by all templates
#[derive(Debug, Clone)]
pub struct LayoutData {
    pub title: String,
    pub active_page: String,
}

impl Default for LayoutData {
    fn default() -> Self {
        Self {
            title: "Gateway Admin".to_string(),
            active_page: "dashboard".to_string(),
        }
    }
}

/// Login page template
#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

impl IntoResponse for LoginTemplate {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// Provider stats for dashboard
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    pub available: usize,
    pub in_use: usize,
    pub max_total: usize,
}

/// Main dashboard template
#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTemplate {
    pub layout: LayoutData,
    pub stats: UsageStats,
    pub pool_stats: PoolStats,
    pub logs: Vec<UsageLog>,
}

impl IntoResponse for DashboardTemplate {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// Providers page template
#[derive(Template)]
#[template(path = "providers.html")]
pub struct ProvidersTemplate {
    pub layout: LayoutData,
    pub providers: Vec<Provider>,
    pub message: Option<String>,
}

impl IntoResponse for ProvidersTemplate {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// API keys page template
#[derive(Template)]
#[template(path = "api_keys.html")]
pub struct ApiKeysTemplate {
    pub layout: LayoutData,
    pub api_keys: Vec<ApiKey>,
    pub new_key: Option<String>,
    pub message: Option<String>,
}

impl IntoResponse for ApiKeysTemplate {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// VM Status for dashboard display
#[derive(Debug, Clone, Default)]
pub struct VmStatusDisplay {
    pub firecracker_running: bool,
    pub agent_healthy: bool,
    pub server_uptime: String,
}

/// Runtime info for dashboard display
#[derive(Debug, Clone)]
pub struct RuntimeInfo {
    pub name: String,
    pub version: String,
}

/// Test execution result
#[derive(Debug, Clone)]
pub struct TestResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

/// Sandboxes config page template (VM-focused)
#[derive(Template)]
#[template(path = "sandboxes.html")]
pub struct SandboxesTemplate {
    pub layout: LayoutData,
    pub vm_host: String,
    pub vm_status: VmStatusDisplay,
    pub runtimes: Vec<RuntimeInfo>,
    pub orchestrator_enabled: bool,
    pub message: Option<String>,
    pub test_result: Option<TestResult>,
}

impl IntoResponse for SandboxesTemplate {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// Usage logs page template
#[derive(Template)]
#[template(path = "usage.html")]
pub struct UsageTemplate {
    pub layout: LayoutData,
    pub stats: UsageStats,
    pub logs: Vec<UsageLog>,
}

impl IntoResponse for UsageTemplate {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// Partial: Stats cards for HTMX refresh
#[derive(Template)]
#[template(path = "partials/stats_cards.html")]
pub struct StatsCardsPartial {
    pub stats: UsageStats,
}

impl IntoResponse for StatsCardsPartial {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// Partial: Pool stats for HTMX refresh
#[derive(Template)]
#[template(path = "partials/pool_stats.html")]
pub struct PoolStatsPartial {
    pub pool_stats: PoolStats,
}

impl IntoResponse for PoolStatsPartial {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// Partial: Provider row for HTMX updates
#[derive(Template)]
#[template(path = "partials/provider_row.html")]
pub struct ProviderRowPartial {
    pub provider: Provider,
}

impl IntoResponse for ProviderRowPartial {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// Partial: API key row for HTMX updates
#[derive(Template)]
#[template(path = "partials/api_key_row.html")]
pub struct ApiKeyRowPartial {
    pub key: ApiKey,
}

impl IntoResponse for ApiKeyRowPartial {
    fn into_response(self) -> Response {
        render_template(self)
    }
}

/// Partial: Recent logs table for HTMX refresh
#[derive(Template)]
#[template(path = "partials/recent_logs.html")]
pub struct RecentLogsPartial {
    pub logs: Vec<UsageLog>,
}

impl IntoResponse for RecentLogsPartial {
    fn into_response(self) -> Response {
        render_template(self)
    }
}
