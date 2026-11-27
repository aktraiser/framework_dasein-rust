//! Authentication for admin dashboard

use axum::{
    body::Body,
    extract::State,
    http::{Method, Request, StatusCode},
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

/// Refuse state-changing dashboard requests that a browser sent from
/// another site (CSRF). The session cookie is `SameSite=Strict` as well; this
/// check also covers browsers and setups where that is not enough.
pub async fn same_origin_only(request: Request<Body>, next: Next) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !safe && is_cross_site(&request) {
        return (StatusCode::FORBIDDEN, "Cross-site request refused").into_response();
    }
    next.run(request).await
}

/// Whether the browser says this request comes from a different origin.
/// Requests with neither header come from non-browser clients, which carry no
/// ambient cookies and so cannot be used for CSRF.
fn is_cross_site(request: &Request<Body>) -> bool {
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok());
    if let Some(site) = header("sec-fetch-site") {
        return !matches!(site, "same-origin" | "none");
    }
    let Some(origin) = header("origin") else {
        return false;
    };
    let host = header("host").or_else(|| request.uri().authority().map(|a| a.as_str()));
    // `Origin` is `scheme://host[:port]`; anything else ("null") is foreign.
    origin.split_once("://").map(|(_, authority)| authority) != host
}

/// Verify the admin password without revealing how much of it matched, or
/// its length: both sides are hashed before the constant-time comparison.
pub fn verify_admin_password(password: &str, expected: &str) -> bool {
    use sha2::{Digest, Sha256};
    Sha256::digest(password.as_bytes())
        .iter()
        .zip(Sha256::digest(expected.as_bytes()).iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Whether the session cookie must be restricted to HTTPS.
///
/// Decided per login: the cookie is `Secure` when the browser is on an HTTPS
/// page (its `Origin` says so) or a trusted proxy reports HTTPS, so it works
/// on `http://localhost` and is protected everywhere else without any
/// setting. `GATEWAY_COOKIE_SECURE=true` or `false` forces the choice.
pub fn cookie_secure(headers: &axum::http::HeaderMap, forwarded_https: bool) -> bool {
    match std::env::var("GATEWAY_COOKIE_SECURE").as_deref() {
        Ok("1" | "true" | "TRUE" | "yes") => return true,
        Ok("0" | "false" | "FALSE" | "no") => return false,
        _ => {}
    }
    forwarded_https
        || headers
            .get("origin")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|origin| origin.starts_with("https://"))
}

/// Limits failed dashboard logins per client address.
#[derive(Default)]
pub struct LoginThrottle {
    failures: std::sync::Mutex<std::collections::HashMap<String, (u32, std::time::Instant)>>,
}

impl LoginThrottle {
    /// Failed attempts allowed per window before logins are refused.
    pub const MAX_FAILURES: u32 = 5;
    /// Length of the counting window, and of the lockout.
    pub const WINDOW: std::time::Duration = std::time::Duration::from_secs(300);
    /// Entries kept at most; older windows are dropped first.
    const MAX_CLIENTS: usize = 10_000;

    fn failures(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, (u32, std::time::Instant)>>
    {
        self.failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether this client has used up its attempts for the current window.
    pub fn is_blocked(&self, client: &str) -> bool {
        self.failures().get(client).is_some_and(|(count, since)| {
            *count >= Self::MAX_FAILURES && since.elapsed() < Self::WINDOW
        })
    }

    /// Count a failed attempt.
    pub fn record_failure(&self, client: &str) {
        let mut failures = self.failures();
        if failures.len() >= Self::MAX_CLIENTS {
            failures.retain(|_, (_, since)| since.elapsed() < Self::WINDOW);
        }
        let entry = failures
            .entry(client.to_owned())
            .or_insert((0, std::time::Instant::now()));
        if entry.1.elapsed() >= Self::WINDOW {
            *entry = (0, std::time::Instant::now());
        }
        entry.0 += 1;
    }

    /// Forget a client's failures after a successful login.
    pub fn record_success(&self, client: &str) {
        self.failures().remove(client);
    }
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

    fn post(headers: &[(&str, &str)]) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/admin/keys/generate");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.body(Body::empty()).unwrap()
    }

    #[test]
    fn cross_site_requests_are_recognised() {
        let host = ("host", "gateway.local:8080");
        // Same origin, by either header.
        assert!(!is_cross_site(&post(&[
            host,
            ("sec-fetch-site", "same-origin")
        ])));
        assert!(!is_cross_site(&post(&[
            host,
            ("origin", "http://gateway.local:8080")
        ])));
        // Non-browser client: no header at all.
        assert!(!is_cross_site(&post(&[host])));
        // Another site, a sibling subdomain, a sandboxed frame, another port.
        assert!(is_cross_site(&post(&[
            host,
            ("sec-fetch-site", "cross-site")
        ])));
        assert!(is_cross_site(&post(&[
            host,
            ("sec-fetch-site", "same-site")
        ])));
        assert!(is_cross_site(&post(&[
            host,
            ("origin", "https://evil.example")
        ])));
        assert!(is_cross_site(&post(&[host, ("origin", "null")])));
        assert!(is_cross_site(&post(&[
            host,
            ("origin", "http://gateway.local:9999")
        ])));
    }

    #[test]
    fn login_throttle_blocks_after_repeated_failures() {
        let throttle = LoginThrottle::default();
        for _ in 0..LoginThrottle::MAX_FAILURES {
            assert!(!throttle.is_blocked("10.0.0.1"));
            throttle.record_failure("10.0.0.1");
        }
        assert!(throttle.is_blocked("10.0.0.1"));
        assert!(
            !throttle.is_blocked("10.0.0.2"),
            "other clients are unaffected"
        );
        throttle.record_success("10.0.0.1");
        assert!(!throttle.is_blocked("10.0.0.1"));
    }
}
