//! Authentication for admin dashboard

use axum::{
    body::Body,
    extract::State,
    http::Request,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use tower_cookies::Cookies;

use crate::state::AppState;

/// Cookie name for admin session
pub const SESSION_COOKIE: &str = "gateway_admin_session";

/// Admin authentication middleware
pub async fn require_auth(
    State(state): State<AppState>,
    Extension(cookies): Extension<Cookies>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // Check for session cookie
    let session_id = cookies.get(SESSION_COOKIE).map(|c| c.value().to_string());

    if let Some(ref session_id) = session_id {
        // Validate session
        if let Some(ref storage) = state.storage {
            if let Ok(valid) = storage.validate_admin_session(session_id).await {
                if valid {
                    return next.run(request).await;
                }
            }
        }
    }

    // Redirect to login
    Redirect::to("/admin/login").into_response()
}

/// Verify admin password using constant-time comparison
pub fn verify_admin_password(password: &str, expected: &str) -> bool {
    if password.len() != expected.len() {
        return false;
    }

    password
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Get admin password from environment
pub fn get_admin_password() -> Option<String> {
    std::env::var("GATEWAY_ADMIN_PASSWORD").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_verification() {
        assert!(verify_admin_password("test123", "test123"));
        assert!(!verify_admin_password("test123", "test124"));
        assert!(!verify_admin_password("test", "test123"));
    }
}
