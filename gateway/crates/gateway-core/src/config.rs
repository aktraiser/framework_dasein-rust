//! Gateway configuration types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Main gateway configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub server: ServerConfig,
    pub llm: LLMConfig,
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub cors_origins: Vec<String>,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

/// LLM providers configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,
}

/// Provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

/// Model configuration with routing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Primary provider for this model
    pub provider: String,
    /// Fallback providers in order
    #[serde(default)]
    pub fallbacks: Vec<String>,
    /// Cost per 1K input tokens (USD)
    #[serde(default)]
    pub input_cost_per_1k: f64,
    /// Cost per 1K output tokens (USD)
    #[serde(default)]
    pub output_cost_per_1k: f64,
}

/// Sandbox configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Firecracker binary path
    #[serde(default = "default_firecracker_path")]
    pub firecracker_path: String,
    /// Kernel path
    pub kernel_path: Option<String>,
    /// Root filesystem path
    pub rootfs_path: Option<String>,
    /// Pool configuration
    #[serde(default)]
    pub pool: SandboxPoolConfig,
}

fn default_firecracker_path() -> String {
    "/usr/bin/firecracker".to_string()
}

/// Sandbox pool configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxPoolConfig {
    #[serde(default = "default_min_ready")]
    pub min_ready: usize,
    #[serde(default = "default_max_total")]
    pub max_total: usize,
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_seconds: u64,
}

fn default_min_ready() -> usize {
    2
}

fn default_max_total() -> usize {
    10
}

fn default_idle_timeout() -> u64 {
    300
}

/// Rate limiting configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub requests_per_minute: Option<u32>,
    #[serde(default)]
    pub tokens_per_minute: Option<u32>,
    #[serde(default)]
    pub tokens_per_day: Option<u32>,
}
