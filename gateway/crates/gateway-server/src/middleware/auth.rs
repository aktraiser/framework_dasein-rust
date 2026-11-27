//! Authentication middleware

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use std::collections::HashSet;

/// Authentication configuration
#[derive(Clone, Default)]
pub struct AuthConfig {
    /// Valid API keys
    pub api_keys: HashSet<String>,
    /// Allow unauthenticated requests
    pub allow_anonymous: bool,
}

/// Authentication layer
#[derive(Clone)]
pub struct AuthLayer {
    config: AuthConfig,
}

impl AuthLayer {
    pub fn new(config: AuthConfig) -> Self {
        Self { config }
    }

    pub async fn authenticate(&self, request: Request, next: Next) -> Result<Response, StatusCode> {
        // Skip auth for health endpoints
        let path = request.uri().path();
        if path == "/health" || path == "/metrics" {
            return Ok(next.run(request).await);
        }

        // Check for API key
        if let Some(auth_header) = request.headers().get(header::AUTHORIZATION) {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(api_key) = auth_str.strip_prefix("Bearer ") {
                    if self.config.api_keys.contains(api_key) {
                        return Ok(next.run(request).await);
                    }
                }
            }
        }

        // Allow anonymous if configured
        if self.config.allow_anonymous {
            return Ok(next.run(request).await);
        }

        Err(StatusCode::UNAUTHORIZED)
    }
}

#[derive(Clone)]
pub struct Principal(pub String);

/// Middleware function for authentication
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = request.uri().path();
    if path == "/health" || (!path.starts_with("/v1/") && path != "/metrics") {
        return Ok(next.run(request).await);
    }
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    // The operator token is explicitly configured; no anonymous execution fallback.
    if state
        .operator_api_key
        .as_deref()
        .is_some_and(|key| constant_time_eq(key.as_bytes(), token.as_bytes()))
    {
        request
            .extensions_mut()
            .insert(Principal("operator".into()));
        return Ok(next.run(request).await);
    }
    if let Some(storage) = &state.storage {
        if let Ok(key) = storage.validate_api_key(token).await {
            // Ordinary API keys can use LLM endpoints. VM/files/direct execution
            // operate on a shared host and are reserved for its operator.
            if matches!(
                path,
                "/v1/chat/completions" | "/v1/embeddings" | "/v1/models"
            ) || path == "/v1/sandboxes"
                || path.starts_with("/v1/sandboxes/")
            {
                // The key's own limits apply to the calls that cost money.
                if matches!(path, "/v1/chat/completions" | "/v1/embeddings") {
                    // Counted in the database: limits hold across restarts
                    // and across processes sharing it. If the count cannot
                    // be taken, the call is refused rather than let through.
                    match storage
                        .admit_request(&key.id, key.rate_limit_rpm, key.rate_limit_tpm)
                        .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(wait)) => return Ok(rate_limited(wait)),
                        Err(error) => {
                            tracing::error!(%error, "Rate limit check failed");
                            return Err(StatusCode::SERVICE_UNAVAILABLE);
                        }
                    }
                }
                request.extensions_mut().insert(Principal(key.id));
                return Ok(next.run(request).await);
            }
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// Compare two secrets without stopping at the first differing byte, so
/// response time does not reveal how much of a guessed token is correct.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// 429 response telling the client when its window resets.
fn rate_limited(wait: std::time::Duration) -> Response {
    use axum::response::IntoResponse;
    let seconds = wait.as_secs().max(1);
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, seconds.to_string())],
        axum::Json(serde_json::json!({
            "error": {
                "message": format!("Rate limit exceeded for this API key. Retry in {seconds}s."),
                "type": "rate_limit_error",
            }
        })),
    )
        .into_response()
}

impl Principal {
    /// The API key to attribute usage to; `None` for the operator token.
    pub fn api_key_id(&self) -> Option<&str> {
        (self.0 != "operator").then_some(self.0.as_str())
    }
}
