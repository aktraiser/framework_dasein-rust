//! Authentication middleware

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use std::collections::HashSet;

/// Authentication configuration
#[derive(Clone)]
pub struct AuthConfig {
    /// Valid API keys
    pub api_keys: HashSet<String>,
    /// Allow unauthenticated requests
    pub allow_anonymous: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            api_keys: HashSet::new(),
            allow_anonymous: true,
        }
    }
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

    pub async fn authenticate(
        &self,
        request: Request,
        next: Next,
    ) -> Result<Response, StatusCode> {
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

/// Middleware function for authentication
pub async fn auth_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Default: allow all requests
    // In production, inject AuthLayer via state
    Ok(next.run(request).await)
}
