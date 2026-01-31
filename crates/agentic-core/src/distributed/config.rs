//! Configuration types for distributed agents.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Capabilities an agent can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Can generate code
    CodeGeneration,
    /// Can execute code
    CodeExecution,
    /// Can read files
    FileRead,
    /// Can write files
    FileWrite,
    /// Can make network requests
    Network,
    /// Can spawn child agents
    SpawnChild,
    /// Can delegate to other agents
    Delegate,
}

/// Role in the distributed architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Supervisor,
    Executor,
    Validator,
}

/// LLM provider configuration.
#[derive(Debug, Clone)]
pub struct LLMConfig {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    pub temperature: f32,
    pub max_tokens: u32,
}

impl LLMConfig {
    /// Create Gemini config with sensible defaults.
    /// Temperature is low (0.2) for deterministic code generation.
    pub fn gemini(model: &str) -> Self {
        Self {
            provider: "gemini".into(),
            model: model.into(),
            api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            temperature: 0.2,  // Low for reliable code generation
            max_tokens: 8192,  // Increased for larger outputs
        }
    }

    /// Create OpenAI config with sensible defaults.
    pub fn openai(model: &str) -> Self {
        Self {
            provider: "openai".into(),
            model: model.into(),
            api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            temperature: 0.7,
            max_tokens: 4096,
        }
    }

    /// Create Anthropic/Claude config with sensible defaults.
    pub fn anthropic(model: &str) -> Self {
        Self {
            provider: "anthropic".into(),
            model: model.into(),
            api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            temperature: 0.2,  // Low for reliable code generation
            max_tokens: 8192,  // Claude supports large outputs
        }
    }

    /// Set temperature.
    pub fn temperature(mut self, temp: f32) -> Self {
        self.temperature = temp;
        self
    }

    /// Set max tokens.
    pub fn max_tokens(mut self, tokens: u32) -> Self {
        self.max_tokens = tokens;
        self
    }

    /// Set API key.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = key.into();
        self
    }
}

/// Sandbox configuration.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub sandbox_type: SandboxType,
    pub timeout_ms: u64,
    pub memory_limit: Option<u64>,
    pub cpu_limit: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxType {
    None,
    Process,
    Docker,
}

impl SandboxConfig {
    /// No sandbox (generation only).
    pub fn none() -> Self {
        Self {
            sandbox_type: SandboxType::None,
            timeout_ms: 0,
            memory_limit: None,
            cpu_limit: None,
        }
    }

    /// Process-based sandbox (fast, less isolated).
    pub fn process() -> Self {
        Self {
            sandbox_type: SandboxType::Process,
            timeout_ms: 30_000,
            memory_limit: None,
            cpu_limit: None,
        }
    }

    /// Docker-based sandbox (slower, fully isolated).
    pub fn docker() -> Self {
        Self {
            sandbox_type: SandboxType::Docker,
            timeout_ms: 60_000,
            memory_limit: Some(512 * 1024 * 1024), // 512MB
            cpu_limit: Some(1.0),
        }
    }

    /// Set timeout.
    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Set memory limit.
    pub fn memory_limit(mut self, bytes: u64) -> Self {
        self.memory_limit = Some(bytes);
        self
    }
}

/// Supervisor configuration.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub id: String,
    pub domain: String,
    pub executor_count: usize,
    pub validator_count: usize,
    pub llm: LLMConfig,
    pub sandbox: SandboxConfig,
    pub capabilities: HashSet<Capability>,
    pub allow_lending: bool,
    pub allow_borrowing: bool,
    pub max_borrowed: usize,
    pub max_queue_depth: usize,
    pub task_timeout_ms: u64,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            domain: "default".into(),
            executor_count: 2,
            validator_count: 1,
            llm: LLMConfig::gemini("gemini-2.0-flash"),
            sandbox: SandboxConfig::process(),
            capabilities: HashSet::from([Capability::CodeGeneration]),
            allow_lending: true,
            allow_borrowing: true,
            max_borrowed: 4,
            max_queue_depth: 100,
            task_timeout_ms: 60_000,
        }
    }
}

/// Executor configuration.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub id: String,
    pub owner_supervisor: String,
    pub llm: LLMConfig,
    pub sandbox: SandboxConfig,
    pub capabilities: HashSet<Capability>,
}

/// Validator configuration.
#[derive(Debug, Clone)]
pub struct ValidatorConfig {
    pub id: String,
    pub supervisor: String,
    pub rules: Vec<String>,
    pub max_retries: u32,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            supervisor: String::new(),
            rules: vec!["output_not_empty".into()],
            max_retries: 2,
        }
    }
}
