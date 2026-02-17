//! Health and metrics endpoints

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use crate::state::AppState;

/// GET /health
pub async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    let pool_stats = state.sandbox_pool.stats().await;

    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        sandbox_pool: PoolHealth {
            available: pool_stats.available,
            in_use: pool_stats.in_use,
            max_total: pool_stats.max_total,
        },
    })
}

/// GET /metrics
pub async fn metrics(State(state): State<AppState>) -> String {
    let pool_stats = state.sandbox_pool.stats().await;

    // Prometheus format
    format!(
        r#"# HELP gateway_sandbox_available Number of available sandboxes
# TYPE gateway_sandbox_available gauge
gateway_sandbox_available {}

# HELP gateway_sandbox_in_use Number of sandboxes in use
# TYPE gateway_sandbox_in_use gauge
gateway_sandbox_in_use {}

# HELP gateway_sandbox_max Maximum sandbox capacity
# TYPE gateway_sandbox_max gauge
gateway_sandbox_max {}
"#,
        pool_stats.available, pool_stats.in_use, pool_stats.max_total
    )
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub sandbox_pool: PoolHealth,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PoolHealth {
    pub available: usize,
    pub in_use: usize,
    pub max_total: usize,
}
