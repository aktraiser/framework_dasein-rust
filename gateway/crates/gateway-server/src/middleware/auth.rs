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
        .is_some_and(|key| key == token)
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
                request.extensions_mut().insert(Principal(key.id));
                return Ok(next.run(request).await);
            }
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}
