//! Dashboard routes

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tower_cookies::CookieManagerLayer;
use tower_http::services::ServeDir;

use super::auth::require_auth;
use super::handlers;
use crate::state::AppState;

/// Create the dashboard router
pub fn create_dashboard_router(state: AppState) -> Router<AppState> {
    // Static files router (no auth needed)
    let static_router = Router::new().nest_service(
        "/admin/static",
        ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/static")),
    );

    // Protected dashboard routes (require auth)
    let protected_routes = Router::new()
        // Main dashboard
        .route("/admin", get(handlers::dashboard))
        // HTMX partials
        .route("/admin/stats", get(handlers::stats_partial))
        .route("/admin/pool-stats", get(handlers::pool_stats_partial))
        .route("/admin/recent-logs", get(handlers::recent_logs_partial))
        // Providers
        .route("/admin/providers", get(handlers::providers))
        .route("/admin/providers/add", post(handlers::add_provider))
        .route(
            "/admin/providers/{id}/toggle",
            post(handlers::toggle_provider),
        )
        .route(
            "/admin/providers/{id}/delete",
            post(handlers::delete_provider),
        )
        // API Keys
        .route("/admin/keys", get(handlers::api_keys))
        .route("/admin/keys/generate", post(handlers::generate_key))
        .route("/admin/keys/{id}/toggle", post(handlers::toggle_key))
        .route("/admin/keys/{id}/revoke", post(handlers::revoke_key))
        // Sandboxes (Remote VM)
        .route("/admin/sandboxes", get(handlers::sandboxes))
        .route("/admin/sandboxes/vm/start", post(handlers::vm_start))
        .route("/admin/sandboxes/vm/stop", post(handlers::vm_stop))
        .route("/admin/sandboxes/vm/restart", post(handlers::vm_restart))
        .route("/admin/sandboxes/test", post(handlers::test_code))
        // Usage
        .route("/admin/usage", get(handlers::usage))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    // Public routes (login/logout)
    let public_routes = Router::new()
        .route(
            "/admin/login",
            get(handlers::login_page).post(handlers::login_submit),
        )
        .route("/admin/logout", post(handlers::logout));

    // Merge all routers with cookie layer
    Router::new()
        .merge(static_router)
        .merge(protected_routes)
        .merge(public_routes)
        .layer(CookieManagerLayer::new())
}
