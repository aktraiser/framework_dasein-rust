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
pub async fn same_origin_only(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !safe && is_cross_site(&request, &state.trusted_proxies) {
        return (StatusCode::FORBIDDEN, "Cross-site request refused").into_response();
    }
    next.run(request).await
}

/// Whether the browser says this request comes from a different origin.
/// Requests with neither header come from non-browser clients, which carry no
/// ambient cookies and so cannot be used for CSRF.
fn is_cross_site(request: &Request<Body>, trusted: &[crate::client_addr::Network]) -> bool {
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok());
    if let Some(site) = header("sec-fetch-site") {
        return !matches!(site, "same-origin" | "none");
    }
    let Some(origin) = header("origin") else {
        return false;
    };
    // `Origin` is `scheme://host[:port]`; anything else ("null") is foreign.
    let Some((_, authority)) = origin.split_once("://") else {
        return true;
    };
    // The host the browser addressed: ours, or the public one a trusted
    // reverse proxy says it received before rewriting `Host`.
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.0.ip());
    let hosts = [
        header("host"),
        request.uri().authority().map(|a| a.as_str()),
        peer.and_then(|peer| crate::client_addr::forwarded_host(peer, request.headers(), trusted)),
    ];
    !hosts
        .into_iter()
        .flatten()
        .any(|host| host.eq_ignore_ascii_case(authority))
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

/// Limits failed dashboard logins, per client and overall.
#[derive(Default)]
pub struct LoginThrottle {
    state: std::sync::Mutex<ThrottleState>,
}

#[derive(Default)]
struct ThrottleState {
    /// Failures per client in its current window.
    clients: std::collections::HashMap<String, (u32, std::time::Instant)>,
    /// Failures from everyone in the current window.
    total: Option<(u32, std::time::Instant)>,
}

impl LoginThrottle {
    /// Failed attempts one client may make per window.
    pub const MAX_FAILURES: u32 = 5;
    /// Failed attempts accepted from all clients together per window. An
    /// attacker with many addresses is held to this rate; while it is
    /// exceeded nobody can log in, the administrator included.
    pub const MAX_TOTAL_FAILURES: u32 = 300;
    /// Length of the counting window, and of the lockout.
    pub const WINDOW: std::time::Duration = std::time::Duration::from_secs(300);
    /// Clients tracked at most; the oldest windows make room for new ones.
    const MAX_CLIENTS: usize = 10_000;

    /// Key under which a client's failures are counted. IPv6 clients are
    /// grouped by /64: one subscriber controls a whole /64 and could
    /// otherwise use a new address for every attempt.
    #[must_use]
    pub fn client_key(ip: std::net::IpAddr) -> String {
        match ip.to_canonical() {
            std::net::IpAddr::V4(v4) => v4.to_string(),
            std::net::IpAddr::V6(v6) => {
                let prefix = u128::from(v6) >> 64 << 64;
                format!("{}/64", std::net::Ipv6Addr::from(prefix))
            }
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ThrottleState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether logins from this client are currently refused.
    pub fn is_blocked(&self, client: &str) -> bool {
        let live = |entry: Option<&(u32, std::time::Instant)>, max: u32| {
            entry.is_some_and(|(count, since)| *count >= max && since.elapsed() < Self::WINDOW)
        };
        let state = self.state();
        live(state.clients.get(client), Self::MAX_FAILURES)
            || live(state.total.as_ref(), Self::MAX_TOTAL_FAILURES)
    }

    /// Count a failed attempt.
    pub fn record_failure(&self, client: &str) {
        let now = std::time::Instant::now();
        let mut state = self.state();
        let total = state.total.get_or_insert((0, now));
        if total.1.elapsed() >= Self::WINDOW {
            *total = (0, now);
        }
        total.0 += 1;

        if !state.clients.contains_key(client) && state.clients.len() >= Self::MAX_CLIENTS {
            state
                .clients
                .retain(|_, (_, since)| since.elapsed() < Self::WINDOW);
            // Still full of live windows: drop the oldest, so the table
            // cannot grow without bound. The overall limit still applies.
            if state.clients.len() >= Self::MAX_CLIENTS {
                let oldest = state
                    .clients
                    .iter()
                    .min_by_key(|(_, (_, since))| *since)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    state.clients.remove(&oldest);
                }
            }
        }
        let entry = state.clients.entry(client.to_owned()).or_insert((0, now));
        if entry.1.elapsed() >= Self::WINDOW {
            *entry = (0, now);
        }
        entry.0 += 1;
    }

    /// Forget a client's failures after a successful login.
    pub fn record_success(&self, client: &str) {
        self.state().clients.remove(client);
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

    fn cross_site(headers: &[(&str, &str)]) -> bool {
        is_cross_site(&post(headers), &[])
    }

    #[test]
    fn cross_site_requests_are_recognised() {
        let host = ("host", "gateway.local:8080");
        // Same origin, by either header.
        assert!(!cross_site(&[host, ("sec-fetch-site", "same-origin")]));
        assert!(!cross_site(&[
            host,
            ("origin", "http://gateway.local:8080")
        ]));
        assert!(!cross_site(&[
            host,
            ("origin", "http://GATEWAY.local:8080")
        ]));
        // Non-browser client: no header at all.
        assert!(!cross_site(&[host]));
        // Another site, a sibling subdomain, a sandboxed frame, another port.
        assert!(cross_site(&[host, ("sec-fetch-site", "cross-site")]));
        assert!(cross_site(&[host, ("sec-fetch-site", "same-site")]));
        assert!(cross_site(&[host, ("origin", "https://evil.example")]));
        assert!(cross_site(&[host, ("origin", "null")]));
        assert!(cross_site(&[host, ("origin", "http://gateway.local:9999")]));
    }

    #[test]
    fn public_host_from_a_trusted_proxy_is_accepted() {
        let trusted = crate::client_addr::parse_networks(&["10.0.0.1".to_string()]);
        let behind = |peer: &str, forwarded_host: &str| {
            let mut request = post(&[
                ("host", "127.0.0.1:8080"),
                ("origin", "http://gateway.lan"),
                ("x-forwarded-host", forwarded_host),
            ]);
            let peer: std::net::SocketAddr = format!("{peer}:4000").parse().unwrap();
            request
                .extensions_mut()
                .insert(axum::extract::ConnectInfo(peer));
            is_cross_site(&request, &trusted)
        };
        // The proxy rewrote Host but reports the public one.
        assert!(!behind("10.0.0.1", "gateway.lan"));
        assert!(behind("10.0.0.1", "other.lan"));
        // Anyone else claiming a forwarded host is not believed.
        assert!(behind("203.0.113.9", "gateway.lan"));
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

    #[test]
    fn login_throttle_resists_address_rotation() {
        // A whole IPv6 /64 is one client.
        let key = |text: &str| LoginThrottle::client_key(text.parse().unwrap());
        assert_eq!(
            key("2001:db8:1:2::1"),
            key("2001:db8:1:2:ffff:ffff:ffff:ffff")
        );
        assert_ne!(key("2001:db8:1:2::1"), key("2001:db8:1:3::1"));
        assert_eq!(key("::ffff:10.0.0.1"), key("10.0.0.1"));

        // Many distinct clients, one failure each: no single one is blocked,
        // but together they hit the overall limit, and the table stays small.
        let throttle = LoginThrottle::default();
        for n in 0..LoginThrottle::MAX_TOTAL_FAILURES {
            assert!(!throttle.is_blocked("fresh"));
            throttle.record_failure(&format!("client-{n}"));
        }
        assert!(throttle.is_blocked("fresh"));
        assert!(throttle.state().clients.len() <= LoginThrottle::MAX_CLIENTS);
    }
}
