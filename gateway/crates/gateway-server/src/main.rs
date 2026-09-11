//! Agentic Gateway - Unified LLM and Sandbox API Server
//!
//! A high-performance gateway providing:
//! - OpenAI-compatible LLM API with multi-provider routing
//! - Python Docker Boxes for controlled laboratory execution
//! - Bounded Box resources and configured provider routing
//! - Admin dashboard for management

use anyhow::Result;
use gateway_core::config::GatewayConfig;
use gateway_core::storage::Storage;
use gateway_server::{create_router, AppState};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .init();

    info!("Starting Agentic Gateway v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = load_config()?;
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let origins = config
        .server
        .cors_origins
        .iter()
        .map(|origin| origin.parse::<axum::http::HeaderValue>())
        .collect::<Result<Vec<_>, _>>()?;
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    // Initialize database
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:gateway.db?mode=rwc".to_string());
    info!("Initializing database at {}", database_url);

    let storage = match Storage::new(&database_url).await {
        Ok(s) => {
            info!("Database initialized successfully");
            Some(s)
        }
        Err(e) => {
            warn!(
                "Failed to initialize database: {}. Dashboard will be limited.",
                e
            );
            None
        }
    };

    // Create application state
    let state = if let Some(storage) = storage {
        AppState::new(config).with_storage(storage)
    } else {
        AppState::new(config)
    };

    // Check admin password configuration
    if std::env::var("GATEWAY_ADMIN_PASSWORD").is_err() {
        warn!("GATEWAY_ADMIN_PASSWORD not set. Dashboard login will be disabled.");
    } else {
        info!("Admin dashboard available at /admin");
    }

    // Recover expired SDK containers left by interrupted gateway processes.
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            interval.tick().await;
            if let Err(error) = dasein_agentic_sandbox::DockerSandbox::reap_expired().await {
                tracing::debug!(%error, "Docker cleanup unavailable");
            }
        }
    });

    // Build router with middleware
    let app = create_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    // Start server
    let addr: SocketAddr = addr.parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!("Listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

fn load_config() -> Result<GatewayConfig> {
    if let Some(path) = std::env::var_os("GATEWAY_CONFIG") {
        let mut settings: GatewayConfig = config::Config::builder()
            .add_source(config::File::from(std::path::PathBuf::from(path)))
            .build()?
            .try_deserialize()?;
        for provider in settings.llm.providers.values_mut().filter(|p| p.enabled) {
            let variable = provider
                .api_key
                .as_deref()
                .and_then(|value| value.strip_prefix("${"))
                .and_then(|value| value.strip_suffix('}'));
            if let Some(variable) = variable {
                provider.api_key = Some(std::env::var(variable).map_err(|_| {
                    anyhow::anyhow!("An enabled provider's API key environment variable is missing")
                })?);
            }
        }
        return Ok(settings);
    }
    // Environment-only mode retains dashboard-managed provider configuration.
    Ok(GatewayConfig {
        server: gateway_core::config::ServerConfig {
            host: std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: std::env::var("GATEWAY_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            cors_origins: vec![],
        },
        llm: gateway_core::config::LLMConfig {
            providers: Default::default(),
            models: Default::default(),
        },
        sandbox: gateway_core::config::SandboxConfig {
            firecracker_path: std::env::var("FIRECRACKER_PATH")
                .unwrap_or_else(|_| "/usr/bin/firecracker".to_string()),
            kernel_path: std::env::var("KERNEL_PATH").ok(),
            rootfs_path: std::env::var("ROOTFS_PATH").ok(),
            pool: Default::default(),
        },
        rate_limit: Default::default(),
    })
}
