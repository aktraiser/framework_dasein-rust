# Feuille de Route - Agentic Framework Rust

> **Vision:** Framework Rust haute-performance pour systèmes multi-agents IA distribués

---

## Vue d'Ensemble

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           AGENTIC-RS ROADMAP                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Phase 0        Phase 1        Phase 2        Phase 3        Phase 4        │
│  Foundation     Core Agent     Coordination   Intelligence   Production     │
│  ─────────     ──────────     ────────────   ────────────   ──────────     │
│  [2 sem]       [3 sem]        [3 sem]        [3 sem]        [3 sem]        │
│                                                                              │
│  ┌─────┐       ┌─────┐        ┌─────┐        ┌─────┐        ┌─────┐        │
│  │Setup│ ───▶  │Agent│  ───▶  │Multi│  ───▶  │ RAG │  ───▶  │Prod │        │
│  │Crate│       │ LLM │        │Agent│        │ MCP │        │Ready│        │
│  │Arch │       │Sand │        │NATS │        │Memory│       │Perf │        │
│  └─────┘       └─────┘        └─────┘        └─────┘        └─────┘        │
│                                                                              │
│  Total estimé: 14 semaines (~3.5 mois)                                      │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Phase 0: Foundation (2 semaines)

### Objectif
Mettre en place l'architecture du projet, les conventions, et les outils de développement.

### 0.1 Structure du Workspace Cargo

```
agentic-rs/
├── Cargo.toml                    # Workspace root
├── rust-toolchain.toml           # Rust version pinning
├── .cargo/
│   └── config.toml               # Build config
├── deny.toml                     # cargo-deny (security)
├── clippy.toml                   # Linting rules
│
├── crates/
│   ├── agentic-core/             # Agent, Protocol, Types
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── agent/
│   │       │   ├── mod.rs
│   │       │   ├── agent.rs
│   │       │   ├── state.rs
│   │       │   └── metrics.rs
│   │       ├── protocol/
│   │       │   ├── mod.rs
│   │       │   ├── message.rs
│   │       │   └── validation.rs
│   │       ├── error.rs
│   │       └── types.rs
│   │
│   ├── agentic-llm/              # LLM Adapters
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs         # LLMAdapter trait
│   │       ├── openai.rs
│   │       ├── anthropic.rs
│   │       ├── gemini.rs
│   │       ├── ollama.rs
│   │       └── factory.rs
│   │
│   ├── agentic-sandbox/          # Code Execution
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs         # Sandbox trait
│   │       ├── docker.rs
│   │       └── process.rs        # Fallback local
│   │
│   ├── agentic-mcp/              # MCP Client
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── client.rs
│   │       ├── transport.rs
│   │       └── executor.rs
│   │
│   ├── agentic-bus/              # NATS Message Bus
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── nats.rs
│   │       └── patterns.rs
│   │
│   ├── agentic-storage/          # Redis + Qdrant
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── redis.rs
│   │       ├── qdrant.rs
│   │       └── traits.rs
│   │
│   └── agentic-orchestrator/     # Multi-agent coordination
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── orchestrator.rs
│           └── workflow.rs
│
├── examples/
│   ├── single_agent.rs
│   ├── multi_agent.rs
│   └── mcp_agent.rs
│
├── tests/
│   └── integration/
│
└── benches/
    └── agent_benchmark.rs
```

### 0.2 Dépendances Core

```toml
# Cargo.toml (workspace)
[workspace]
resolver = "2"
members = [
    "crates/agentic-core",
    "crates/agentic-llm",
    "crates/agentic-sandbox",
    "crates/agentic-mcp",
    "crates/agentic-bus",
    "crates/agentic-storage",
    "crates/agentic-orchestrator",
]

[workspace.dependencies]
# Async runtime
tokio = { version = "1.43", features = ["full"] }
async-trait = "0.1"
futures = "0.3"

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Error handling
thiserror = "2.0"
anyhow = "1.0"

# Logging/Tracing
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Validation
validator = { version = "0.19", features = ["derive"] }

# UUID
uuid = { version = "1.11", features = ["v4", "serde"] }

# Time
chrono = { version = "0.4", features = ["serde"] }

# HTTP client (for LLM APIs)
reqwest = { version = "0.12", features = ["json", "stream"] }

# LLM SDKs
async-openai = "0.27"
# rust-genai ou custom pour Anthropic/Gemini

# MCP
rmcp = { version = "0.8", features = ["client", "transport-stdio"] }

# Message Bus
async-nats = "0.38"

# Storage
redis = { version = "0.27", features = ["tokio-comp", "connection-manager"] }
qdrant-client = "1.13"

# Docker
bollard = "0.18"

# Testing
tokio-test = "0.4"
mockall = "0.13"
wiremock = "0.6"
```

### 0.3 Tâches Phase 0

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 0.1 | Initialiser workspace Cargo | `Cargo.toml` workspace |
| 0.2 | Configurer CI/CD (GitHub Actions) | `.github/workflows/` |
| 0.3 | Setup linting (clippy, rustfmt) | `clippy.toml`, `.rustfmt.toml` |
| 0.4 | Setup security audit (cargo-deny) | `deny.toml` |
| 0.5 | Créer structure crates vides | 7 crates avec `lib.rs` |
| 0.6 | Configurer tests + benchmarks | `tests/`, `benches/` |
| 0.7 | Documentation setup (rustdoc) | `README.md` par crate |

### Critères de Validation Phase 0
- [ ] `cargo build` compile sans erreur
- [ ] `cargo clippy` passe sans warning
- [ ] `cargo test` exécute (même si vide)
- [ ] CI GitHub Actions fonctionne
- [ ] Documentation générée avec `cargo doc`

---

## Phase 1: Core Agent (3 semaines)

### Objectif
Implémenter l'Agent de base avec support LLM multi-provider.

### 1.1 Types et Protocol (Semaine 1)

```rust
// crates/agentic-core/src/types.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Mode d'exécution du code généré
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Always,
    Never,
    Auto,
    Task,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Task
    }
}

/// État de l'agent
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Busy,
    Error,
    Stopped,
}

/// Configuration de l'agent
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub execution_mode: ExecutionMode,
    pub timeout_ms: u64,
    pub max_retries: u32,
}

/// Payload d'une tâche
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPayload {
    pub action: String,
    pub spec: serde_json::Value,
    #[serde(default)]
    pub inputs: Option<serde_json::Value>,
    #[serde(default)]
    pub constraints: Vec<String>,
}

/// Résultat d'une tâche
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultPayload {
    pub status: ResultStatus,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    pub metrics: ResultMetrics,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Success,
    Failure,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_type: ArtifactType,
    pub path: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    Code,
    File,
    Data,
    Log,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResultMetrics {
    pub execution_time_ms: u64,
    #[serde(default)]
    pub tokens_used: Option<u32>,
}

/// Métriques de l'agent
#[derive(Debug, Clone, Default)]
pub struct AgentMetrics {
    pub total_tasks: u64,
    pub successful_tasks: u64,
    pub failed_tasks: u64,
    pub average_execution_time_ms: f64,
    pub total_tokens_used: u64,
    pub uptime_ms: u64,
}

/// État interne de l'agent
#[derive(Debug, Clone)]
pub struct AgentState {
    pub status: AgentStatus,
    pub current_task_id: Option<String>,
    pub last_activity_at: DateTime<Utc>,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            status: AgentStatus::Idle,
            current_task_id: None,
            last_activity_at: Utc::now(),
            tasks_completed: 0,
            tasks_failed: 0,
        }
    }
}
```

### 1.2 LLM Adapter Trait (Semaine 1)

```rust
// crates/agentic-llm/src/traits.rs

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use crate::error::LLMError;

/// Message pour le LLM
#[derive(Debug, Clone)]
pub struct LLMMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl LLMMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

/// Réponse du LLM
#[derive(Debug, Clone)]
pub struct LLMResponse {
    pub content: String,
    pub tokens_used: TokenUsage,
    pub finish_reason: FinishReason,
    pub model: String,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    Error,
}

/// Chunk de streaming
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub content: String,
    pub done: bool,
    pub tokens_used: Option<TokenUsage>,
    pub finish_reason: Option<FinishReason>,
}

/// Trait principal pour les adapters LLM
#[async_trait]
pub trait LLMAdapter: Send + Sync {
    /// Nom du provider
    fn provider(&self) -> &str;

    /// Modèle utilisé
    fn model(&self) -> &str;

    /// Génère une completion
    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError>;

    /// Génère en streaming
    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>>;

    /// Health check
    async fn health_check(&self) -> Result<bool, LLMError>;
}
```

### 1.3 OpenAI Adapter (Semaine 2)

```rust
// crates/agentic-llm/src/openai.rs

use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
        ChatCompletionRequestUserMessage, ChatCompletionRequestAssistantMessage,
        CreateChatCompletionRequest, CreateChatCompletionStreamResponse,
    },
    Client,
};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;

use crate::{
    traits::{LLMAdapter, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage, FinishReason},
    error::LLMError,
};

pub struct OpenAIAdapter {
    client: Client<OpenAIConfig>,
    model: String,
    temperature: f32,
    max_tokens: Option<u32>,
}

impl OpenAIAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        Self {
            client: Client::with_config(config),
            model: model.into(),
            temperature: 0.7,
            max_tokens: None,
        }
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    fn convert_messages(&self, messages: &[LLMMessage]) -> Vec<ChatCompletionRequestMessage> {
        messages.iter().map(|msg| {
            match msg.role {
                Role::System => ChatCompletionRequestMessage::System(
                    ChatCompletionRequestSystemMessage {
                        content: msg.content.clone().into(),
                        ..Default::default()
                    }
                ),
                Role::User => ChatCompletionRequestMessage::User(
                    ChatCompletionRequestUserMessage {
                        content: msg.content.clone().into(),
                        ..Default::default()
                    }
                ),
                Role::Assistant => ChatCompletionRequestMessage::Assistant(
                    ChatCompletionRequestAssistantMessage {
                        content: Some(msg.content.clone()),
                        ..Default::default()
                    }
                ),
            }
        }).collect()
    }
}

#[async_trait]
impl LLMAdapter for OpenAIAdapter {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        let request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: self.convert_messages(messages),
            temperature: Some(self.temperature),
            max_tokens: self.max_tokens,
            ..Default::default()
        };

        let response = self.client
            .chat()
            .create(request)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;

        let choice = response.choices.first()
            .ok_or_else(|| LLMError::EmptyResponse)?;

        let content = choice.message.content.clone()
            .unwrap_or_default();

        let usage = response.usage.as_ref();

        Ok(LLMResponse {
            content,
            tokens_used: TokenUsage {
                prompt: usage.map(|u| u.prompt_tokens).unwrap_or(0),
                completion: usage.map(|u| u.completion_tokens).unwrap_or(0),
                total: usage.map(|u| u.total_tokens).unwrap_or(0),
            },
            finish_reason: match choice.finish_reason {
                Some(async_openai::types::FinishReason::Stop) => FinishReason::Stop,
                Some(async_openai::types::FinishReason::Length) => FinishReason::Length,
                _ => FinishReason::Stop,
            },
            model: response.model,
        })
    }

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        let request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: self.convert_messages(messages),
            temperature: Some(self.temperature),
            max_tokens: self.max_tokens,
            stream: Some(true),
            ..Default::default()
        };

        Box::pin(async_stream::try_stream! {
            let mut stream = self.client
                .chat()
                .create_stream(request)
                .await
                .map_err(|e| LLMError::ApiError(e.to_string()))?;

            while let Some(result) = stream.next().await {
                let response = result.map_err(|e| LLMError::ApiError(e.to_string()))?;

                if let Some(choice) = response.choices.first() {
                    let content = choice.delta.content.clone().unwrap_or_default();
                    let done = choice.finish_reason.is_some();

                    yield StreamChunk {
                        content,
                        done,
                        tokens_used: None,
                        finish_reason: choice.finish_reason.map(|r| match r {
                            async_openai::types::FinishReason::Stop => FinishReason::Stop,
                            async_openai::types::FinishReason::Length => FinishReason::Length,
                            _ => FinishReason::Stop,
                        }),
                    };
                }
            }
        })
    }

    async fn health_check(&self) -> Result<bool, LLMError> {
        // Simple check: list models
        let _ = self.client.models().list().await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;
        Ok(true)
    }
}
```

### 1.4 Agent Implementation (Semaine 2-3)

```rust
// crates/agentic-core/src/agent/agent.rs

use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::Utc;
use tracing::{info, error, instrument};

use crate::{
    types::{
        AgentConfig, AgentState, AgentStatus, AgentMetrics,
        TaskPayload, ResultPayload, ResultStatus, ResultMetrics,
        ExecutionMode,
    },
    error::AgentError,
};
use dasein_agentic_llm::{LLMAdapter, LLMMessage};
use dasein_agentic_sandbox::Sandbox;

/// Actions qui déclenchent l'exécution par défaut
const EXECUTABLE_ACTIONS: &[&str] = &[
    "execute", "run", "test", "build", "deploy", "install", "validate"
];

/// Actions qui ne déclenchent PAS l'exécution par défaut
const NON_EXECUTABLE_ACTIONS: &[&str] = &[
    "review", "analyze", "explain", "document", "generate", "refactor", "suggest", "plan"
];

/// Agent - Entité autonome avec LLM + Sandbox
pub struct Agent<L: LLMAdapter, S: Sandbox> {
    id: String,
    config: AgentConfig,
    llm: Arc<L>,
    sandbox: Option<Arc<S>>,
    state: Arc<RwLock<AgentState>>,
    metrics: Arc<RwLock<AgentMetrics>>,
    start_time: Arc<RwLock<Option<std::time::Instant>>>,
}

impl<L: LLMAdapter, S: Sandbox> Agent<L, S> {
    /// Crée un nouvel agent
    pub fn new(config: AgentConfig, llm: L, sandbox: Option<S>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            config,
            llm: Arc::new(llm),
            sandbox: sandbox.map(Arc::new),
            state: Arc::new(RwLock::new(AgentState::default())),
            metrics: Arc::new(RwLock::new(AgentMetrics::default())),
            start_time: Arc::new(RwLock::new(None)),
        }
    }

    /// Retourne l'ID de l'agent
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Retourne le nom de l'agent
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Démarre l'agent
    #[instrument(skip(self), fields(agent_name = %self.config.name))]
    pub async fn start(&self) -> Result<(), AgentError> {
        // Vérifier le LLM
        if !self.llm.health_check().await.unwrap_or(false) {
            return Err(AgentError::LLMNotAvailable);
        }

        // Vérifier le sandbox si configuré et mode != never
        if self.config.execution_mode != ExecutionMode::Never {
            if let Some(ref sandbox) = self.sandbox {
                if !sandbox.is_ready().await? {
                    return Err(AgentError::SandboxNotReady);
                }
            }
        }

        // Mettre à jour l'état
        {
            let mut state = self.state.write().await;
            state.status = AgentStatus::Idle;
        }

        // Enregistrer le temps de démarrage
        {
            let mut start_time = self.start_time.write().await;
            *start_time = Some(std::time::Instant::now());
        }

        info!(agent = %self.config.name, "Agent started");
        Ok(())
    }

    /// Arrête l'agent
    #[instrument(skip(self), fields(agent_name = %self.config.name))]
    pub async fn stop(&self) -> Result<(), AgentError> {
        if let Some(ref sandbox) = self.sandbox {
            sandbox.stop().await?;
        }

        {
            let mut state = self.state.write().await;
            state.status = AgentStatus::Stopped;
        }

        info!(agent = %self.config.name, "Agent stopped");
        Ok(())
    }

    /// Exécute une tâche
    #[instrument(skip(self, task), fields(agent_name = %self.config.name, action = %task.action))]
    pub async fn run(&self, task: TaskPayload) -> Result<ResultPayload, AgentError> {
        let start = std::time::Instant::now();
        let task_id = uuid::Uuid::new_v4().to_string();

        // Mettre à jour l'état
        {
            let mut state = self.state.write().await;
            state.status = AgentStatus::Busy;
            state.current_task_id = Some(task_id.clone());
        }

        // Incrémenter le compteur de tâches
        {
            let mut metrics = self.metrics.write().await;
            metrics.total_tasks += 1;
        }

        info!(task_id = %task_id, action = %task.action, "Task started");

        let result = self.execute_task(&task, &task_id, start).await;

        // Mettre à jour l'état et les métriques
        {
            let mut state = self.state.write().await;
            state.current_task_id = None;
            state.last_activity_at = Utc::now();

            match &result {
                Ok(r) if r.status == ResultStatus::Success => {
                    state.status = AgentStatus::Idle;
                    state.tasks_completed += 1;
                }
                _ => {
                    state.status = AgentStatus::Error;
                    state.tasks_failed += 1;
                }
            }
        }

        // Mettre à jour les métriques
        {
            let mut metrics = self.metrics.write().await;
            let elapsed_ms = start.elapsed().as_millis() as u64;

            match &result {
                Ok(r) => {
                    if r.status == ResultStatus::Success {
                        metrics.successful_tasks += 1;
                    } else {
                        metrics.failed_tasks += 1;
                    }

                    // Moyenne mobile du temps d'exécution
                    let n = metrics.total_tasks as f64;
                    metrics.average_execution_time_ms =
                        (metrics.average_execution_time_ms * (n - 1.0) + elapsed_ms as f64) / n;

                    if let Some(tokens) = r.metrics.tokens_used {
                        metrics.total_tokens_used += tokens as u64;
                    }
                }
                Err(_) => {
                    metrics.failed_tasks += 1;
                }
            }
        }

        result
    }

    /// Exécution interne de la tâche
    async fn execute_task(
        &self,
        task: &TaskPayload,
        task_id: &str,
        start: std::time::Instant,
    ) -> Result<ResultPayload, AgentError> {
        // 1. THINK: Demander au LLM
        let messages = self.build_messages(task);
        let llm_response = self.llm.generate(&messages).await
            .map_err(|e| AgentError::LLMError(e.to_string()))?;

        // 2. EXTRACT: Extraire le code
        let code = self.extract_code(&llm_response.content);

        // 3. DECIDE: Exécuter ou non
        let should_execute = self.should_execute(task, &llm_response.content);

        if !should_execute || code.is_none() {
            // Réponse directe ou code non exécuté
            return Ok(ResultPayload {
                status: ResultStatus::Success,
                data: Some(serde_json::json!({
                    "output": llm_response.content,
                    "code": code,
                    "executed": false,
                })),
                artifacts: vec![],
                metrics: ResultMetrics {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    tokens_used: Some(llm_response.tokens_used.total),
                },
                error: None,
            });
        }

        // 4. EXECUTE: Exécuter dans le sandbox
        let sandbox = self.sandbox.as_ref()
            .ok_or(AgentError::SandboxRequired)?;

        let exec_result = sandbox.execute(&code.unwrap()).await?;

        // 5. RESPOND: Construire le résultat
        Ok(ResultPayload {
            status: if exec_result.exit_code == 0 {
                ResultStatus::Success
            } else {
                ResultStatus::Failure
            },
            data: Some(serde_json::json!({
                "output": exec_result.stdout,
                "error": exec_result.stderr,
                "executed": true,
            })),
            artifacts: exec_result.artifacts.into_iter().map(|a| {
                crate::types::Artifact {
                    artifact_type: crate::types::ArtifactType::File,
                    path: Some(a.path),
                    content: Some(a.content),
                }
            }).collect(),
            metrics: ResultMetrics {
                execution_time_ms: start.elapsed().as_millis() as u64,
                tokens_used: Some(llm_response.tokens_used.total),
            },
            error: if exec_result.exit_code != 0 {
                Some(exec_result.stderr)
            } else {
                None
            },
        })
    }

    /// Construit les messages pour le LLM
    fn build_messages(&self, task: &TaskPayload) -> Vec<LLMMessage> {
        let system_prompt = self.build_system_prompt();
        let user_prompt = self.build_user_prompt(task);

        vec![
            LLMMessage::system(system_prompt),
            LLMMessage::user(user_prompt),
        ]
    }

    /// Construit le system prompt
    fn build_system_prompt(&self) -> String {
        let execution_instructions = match self.config.execution_mode {
            ExecutionMode::Always => "Le code que tu génères sera TOUJOURS exécuté.",
            ExecutionMode::Never => "Le code que tu génères ne sera PAS exécuté.",
            ExecutionMode::Auto => "Utilise [EXECUTE] ou [NO_EXECUTE] pour indiquer si le code doit être exécuté.",
            ExecutionMode::Task => "L'exécution dépend du type d'action demandée.",
        };

        format!(
            "{}\n\n## Instructions\n\n\
            Tu es un agent qui génère du CODE pour accomplir des tâches.\n\n\
            RÈGLES:\n\
            1. Réponds avec du code exécutable quand approprié\n\
            2. Le code doit être complet et fonctionnel\n\
            3. Utilise println!() ou console.log() pour les résultats\n\
            4. Gère les erreurs\n\n\
            {}\n\n\
            FORMAT:\n\
            ```rust\n// ou typescript\n// Ton code ici\n```",
            self.config.system_prompt,
            execution_instructions
        )
    }

    /// Construit le user prompt
    fn build_user_prompt(&self, task: &TaskPayload) -> String {
        let mut parts = vec![
            format!("## Action: {}", task.action),
            format!("## Specification:\n{}", serde_json::to_string_pretty(&task.spec).unwrap_or_default()),
        ];

        if let Some(inputs) = &task.inputs {
            parts.push(format!("## Inputs:\n{}", serde_json::to_string_pretty(inputs).unwrap_or_default()));
        }

        if !task.constraints.is_empty() {
            parts.push(format!("## Constraints:\n- {}", task.constraints.join("\n- ")));
        }

        parts.join("\n\n")
    }

    /// Extrait le code des blocs markdown
    fn extract_code(&self, content: &str) -> Option<String> {
        let re = regex::Regex::new(r"```(?:rust|typescript|javascript)\n([\s\S]*?)```").ok()?;
        re.captures(content)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// Détermine si le code doit être exécuté
    fn should_execute(&self, task: &TaskPayload, content: &str) -> bool {
        match self.config.execution_mode {
            ExecutionMode::Always => true,
            ExecutionMode::Never => false,
            ExecutionMode::Auto => {
                // Parser les marqueurs
                if content.contains("[NO_EXECUTE]") || content.contains("<!-- NO_EXECUTE -->") {
                    return false;
                }
                if content.contains("[EXECUTE]") || content.contains("<!-- EXECUTE -->") {
                    return true;
                }
                false
            }
            ExecutionMode::Task => {
                let action = task.action.to_lowercase();

                if EXECUTABLE_ACTIONS.iter().any(|a| action.contains(a)) {
                    return true;
                }
                if NON_EXECUTABLE_ACTIONS.iter().any(|a| action.contains(a)) {
                    return false;
                }

                // Par défaut, ne pas exécuter (principe de moindre privilège)
                false
            }
        }
    }

    /// Retourne l'état actuel
    pub async fn get_state(&self) -> AgentState {
        self.state.read().await.clone()
    }

    /// Retourne les métriques
    pub async fn get_metrics(&self) -> AgentMetrics {
        let mut metrics = self.metrics.read().await.clone();

        // Calculer l'uptime
        if let Some(start) = *self.start_time.read().await {
            metrics.uptime_ms = start.elapsed().as_millis() as u64;
        }

        metrics
    }
}
```

### 1.5 Tâches Phase 1

| Tâche | Semaine | Description | Livrable |
|-------|---------|-------------|----------|
| 1.1 | S1 | Types et Protocol | `agentic-core/src/types.rs` |
| 1.2 | S1 | Error types | `agentic-core/src/error.rs` |
| 1.3 | S1 | LLMAdapter trait | `agentic-llm/src/traits.rs` |
| 1.4 | S2 | OpenAI adapter | `agentic-llm/src/openai.rs` |
| 1.5 | S2 | Ollama adapter | `agentic-llm/src/ollama.rs` |
| 1.6 | S2 | Sandbox trait | `agentic-sandbox/src/traits.rs` |
| 1.7 | S2 | Process sandbox (local) | `agentic-sandbox/src/process.rs` |
| 1.8 | S3 | Agent implementation | `agentic-core/src/agent/` |
| 1.9 | S3 | Tests unitaires Agent | `tests/agent_test.rs` |
| 1.10 | S3 | Example single_agent | `examples/single_agent.rs` |

### Critères de Validation Phase 1
- [ ] Agent compile et démarre
- [ ] Génération LLM fonctionne (OpenAI + Ollama)
- [ ] Extraction code markdown fonctionne
- [ ] Mode execution `task` fonctionne
- [ ] Tests unitaires passent (>80% coverage)
- [ ] Example `single_agent` exécutable

---

## Phase 2: Coordination (3 semaines)

### Objectif
Implémenter la communication multi-agents via NATS et l'orchestration.

### 2.1 NATS Message Bus

```rust
// crates/agentic-bus/src/nats.rs

use async_nats::{Client, Subscriber, Message as NatsMessage};
use async_trait::async_trait;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::{
    traits::{MessageBus, Channel, MessageHandler},
    error::BusError,
};
use dasein_agentic_core::protocol::Message;

pub struct NatsBus {
    client: Client,
    url: String,
}

impl NatsBus {
    pub async fn connect(url: &str) -> Result<Self, BusError> {
        let client = async_nats::connect(url).await
            .map_err(|e| BusError::ConnectionFailed(e.to_string()))?;

        Ok(Self {
            client,
            url: url.to_string(),
        })
    }
}

#[async_trait]
impl MessageBus for NatsBus {
    async fn disconnect(&self) -> Result<(), BusError> {
        self.client.drain().await
            .map_err(|e| BusError::DisconnectFailed(e.to_string()))
    }

    fn channel(&self, subject: &str) -> Box<dyn Channel + Send + Sync> {
        Box::new(NatsChannel {
            client: self.client.clone(),
            subject: subject.to_string(),
        })
    }

    fn is_connected(&self) -> bool {
        self.client.connection_state() == async_nats::connection::State::Connected
    }
}

pub struct NatsChannel {
    client: Client,
    subject: String,
}

#[async_trait]
impl Channel for NatsChannel {
    async fn publish(&self, message: &Message) -> Result<(), BusError> {
        let payload = serde_json::to_vec(message)
            .map_err(|e| BusError::SerializationError(e.to_string()))?;

        self.client.publish(self.subject.clone(), payload.into()).await
            .map_err(|e| BusError::PublishFailed(e.to_string()))
    }

    async fn subscribe(&self) -> Result<Box<dyn futures::Stream<Item = Message> + Send + Unpin>, BusError> {
        let subscriber = self.client.subscribe(self.subject.clone()).await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))?;

        let stream = subscriber.filter_map(|msg| async move {
            serde_json::from_slice::<Message>(&msg.payload).ok()
        });

        Ok(Box::new(stream))
    }

    async fn request(&self, message: &Message, timeout_ms: u64) -> Result<Message, BusError> {
        let payload = serde_json::to_vec(message)
            .map_err(|e| BusError::SerializationError(e.to_string()))?;

        let response = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            self.client.request(self.subject.clone(), payload.into())
        ).await
            .map_err(|_| BusError::Timeout)?
            .map_err(|e| BusError::RequestFailed(e.to_string()))?;

        serde_json::from_slice(&response.payload)
            .map_err(|e| BusError::DeserializationError(e.to_string()))
    }
}

/// Subject patterns standards
pub struct SubjectPatterns;

impl SubjectPatterns {
    pub fn agent_inbox(agent_id: &str) -> String {
        format!("agents.{}.inbox", agent_id)
    }

    pub fn agent_outbox(agent_id: &str) -> String {
        format!("agents.{}.outbox", agent_id)
    }

    pub fn agent_events(agent_id: &str) -> String {
        format!("agents.{}.events", agent_id)
    }

    pub fn orchestrator() -> &'static str {
        "orchestrator.inbox"
    }

    pub fn system_events() -> &'static str {
        "system.events"
    }
}
```

### 2.2 Orchestrator

```rust
// crates/agentic-orchestrator/src/orchestrator.rs

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error, instrument};

use dasein_agentic_core::{
    types::{TaskPayload, ResultPayload, ResultStatus},
    agent::Agent,
};
use dasein_agentic_llm::LLMAdapter;
use dasein_agentic_sandbox::Sandbox;

use crate::{
    workflow::{Workflow, WorkflowStep, WorkflowResult},
    error::OrchestratorError,
};

/// Référence à un agent enregistré
pub struct AgentRef {
    pub id: String,
    pub name: String,
}

/// Orchestrateur multi-agents
pub struct Orchestrator<L: LLMAdapter, S: Sandbox> {
    agents: Arc<RwLock<HashMap<String, Arc<Agent<L, S>>>>>,
    default_timeout_ms: u64,
    max_parallel_tasks: usize,
}

impl<L: LLMAdapter + 'static, S: Sandbox + 'static> Orchestrator<L, S> {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            default_timeout_ms: 60000,
            max_parallel_tasks: 10,
        }
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.default_timeout_ms = timeout_ms;
        self
    }

    pub fn with_max_parallel(mut self, max: usize) -> Self {
        self.max_parallel_tasks = max;
        self
    }

    /// Enregistre un agent
    pub async fn register_agent(&self, agent: Agent<L, S>) {
        let id = agent.id().to_string();
        let name = agent.name().to_string();

        self.agents.write().await.insert(id.clone(), Arc::new(agent));
        info!(agent_id = %id, agent_name = %name, "Agent registered");
    }

    /// Supprime un agent
    pub async fn unregister_agent(&self, agent_id: &str) -> Option<Arc<Agent<L, S>>> {
        self.agents.write().await.remove(agent_id)
    }

    /// Liste les agents
    pub async fn list_agents(&self) -> Vec<AgentRef> {
        self.agents.read().await.iter().map(|(id, agent)| {
            AgentRef {
                id: id.clone(),
                name: agent.name().to_string(),
            }
        }).collect()
    }

    /// Envoie une tâche à un agent
    #[instrument(skip(self, task), fields(agent_id = %agent_id))]
    pub async fn send_task(
        &self,
        agent_id: &str,
        task: TaskPayload,
    ) -> Result<ResultPayload, OrchestratorError> {
        let agents = self.agents.read().await;
        let agent = agents.get(agent_id)
            .ok_or_else(|| OrchestratorError::AgentNotFound(agent_id.to_string()))?;

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(self.default_timeout_ms),
            agent.run(task)
        ).await
            .map_err(|_| OrchestratorError::Timeout)?
            .map_err(|e| OrchestratorError::AgentError(e.to_string()))?;

        Ok(result)
    }

    /// Exécute un workflow
    #[instrument(skip(self, workflow), fields(workflow_name = %workflow.name))]
    pub async fn execute(&self, workflow: Workflow) -> Result<WorkflowResult, OrchestratorError> {
        let start = std::time::Instant::now();
        let workflow_id = uuid::Uuid::new_v4().to_string();

        info!(workflow_id = %workflow_id, "Workflow started");

        let mut results: HashMap<String, ResultPayload> = HashMap::new();
        let mut pending_steps: Vec<&WorkflowStep> = workflow.steps.iter().collect();

        while !pending_steps.is_empty() {
            // Trouver les steps prêts (dépendances satisfaites)
            let ready_steps: Vec<_> = pending_steps.iter()
                .filter(|step| {
                    step.depends_on.iter().all(|dep| results.contains_key(dep))
                })
                .cloned()
                .collect();

            if ready_steps.is_empty() && !pending_steps.is_empty() {
                return Err(OrchestratorError::CircularDependency);
            }

            // Exécuter les steps prêts en parallèle
            let mut handles = vec![];

            for step in &ready_steps {
                let step_id = format!("{}:{}", step.agent, step.action);
                let task = TaskPayload {
                    action: step.action.clone(),
                    spec: step.inputs.clone().unwrap_or(serde_json::json!({})),
                    inputs: None,
                    constraints: vec![],
                };

                let agent_id = self.find_agent_by_name(&step.agent).await
                    .ok_or_else(|| OrchestratorError::AgentNotFound(step.agent.clone()))?;

                let agents = self.agents.clone();
                let timeout = self.default_timeout_ms;

                handles.push(tokio::spawn(async move {
                    let agents = agents.read().await;
                    let agent = agents.get(&agent_id).unwrap();

                    let result = tokio::time::timeout(
                        std::time::Duration::from_millis(timeout),
                        agent.run(task)
                    ).await;

                    (step_id, result)
                }));
            }

            // Collecter les résultats
            for handle in handles {
                let (step_id, result) = handle.await
                    .map_err(|e| OrchestratorError::JoinError(e.to_string()))?;

                let result = result
                    .map_err(|_| OrchestratorError::Timeout)?
                    .map_err(|e| OrchestratorError::AgentError(e.to_string()))?;

                results.insert(step_id, result);
            }

            // Retirer les steps exécutés
            pending_steps.retain(|step| {
                let step_id = format!("{}:{}", step.agent, step.action);
                !results.contains_key(&step_id)
            });
        }

        // Déterminer le statut global
        let all_success = results.values().all(|r| r.status == ResultStatus::Success);
        let any_success = results.values().any(|r| r.status == ResultStatus::Success);

        let status = if all_success {
            ResultStatus::Success
        } else if any_success {
            ResultStatus::Partial
        } else {
            ResultStatus::Failure
        };

        info!(
            workflow_id = %workflow_id,
            status = ?status,
            duration_ms = %start.elapsed().as_millis(),
            "Workflow completed"
        );

        Ok(WorkflowResult {
            workflow_id,
            status,
            results,
            total_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Trouve un agent par son nom
    async fn find_agent_by_name(&self, name: &str) -> Option<String> {
        self.agents.read().await.iter()
            .find(|(_, agent)| agent.name() == name)
            .map(|(id, _)| id.clone())
    }
}
```

### 2.3 Tâches Phase 2

| Tâche | Semaine | Description | Livrable |
|-------|---------|-------------|----------|
| 2.1 | S1 | MessageBus trait | `agentic-bus/src/traits.rs` |
| 2.2 | S1 | NATS implementation | `agentic-bus/src/nats.rs` |
| 2.3 | S1 | Subject patterns | `agentic-bus/src/patterns.rs` |
| 2.4 | S2 | Workflow types | `agentic-orchestrator/src/workflow.rs` |
| 2.5 | S2 | Orchestrator impl | `agentic-orchestrator/src/orchestrator.rs` |
| 2.6 | S2 | DAG dependency resolution | Dans orchestrator |
| 2.7 | S3 | Tests intégration NATS | `tests/nats_integration.rs` |
| 2.8 | S3 | Tests workflow | `tests/workflow_test.rs` |
| 2.9 | S3 | Example multi_agent | `examples/multi_agent.rs` |

### Critères de Validation Phase 2
- [ ] Connexion NATS fonctionne
- [ ] Publish/Subscribe fonctionne
- [ ] Request/Reply fonctionne
- [ ] Orchestrator enregistre des agents
- [ ] Workflow avec dépendances s'exécute
- [ ] Parallélisation des steps indépendants

---

## Phase 3: Intelligence (3 semaines)

### Objectif
Ajouter MCP, storage (Redis/Qdrant), et les fonctionnalités avancées.

### 3.1 MCP Client

```rust
// crates/agentic-mcp/src/client.rs

use rmcp::{
    ServiceExt,
    model::{CallToolResult, Tool},
    service::RunningService,
    transport::TokioChildProcess,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::MCPError;

/// Configuration d'un serveur MCP
#[derive(Debug, Clone)]
pub struct MCPServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// Client MCP
pub struct MCPClient {
    servers: Arc<RwLock<HashMap<String, RunningService<TokioChildProcess, ()>>>>,
    tools: Arc<RwLock<HashMap<String, Vec<Tool>>>>,
}

impl MCPClient {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Connecte à un serveur MCP
    pub async fn connect_server(&self, config: MCPServerConfig) -> Result<(), MCPError> {
        let transport = TokioChildProcess::new(&config.command, &config.args)
            .map_err(|e| MCPError::TransportError(e.to_string()))?;

        let service = ().serve(transport).await
            .map_err(|e| MCPError::ConnectionFailed(e.to_string()))?;

        // Découvrir les tools
        let tools_list = service.list_tools().await
            .map_err(|e| MCPError::DiscoveryFailed(e.to_string()))?;

        // Stocker
        {
            let mut servers = self.servers.write().await;
            servers.insert(config.name.clone(), service);
        }
        {
            let mut tools = self.tools.write().await;
            tools.insert(config.name.clone(), tools_list.tools);
        }

        Ok(())
    }

    /// Appelle un tool
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<CallToolResult, MCPError> {
        let servers = self.servers.read().await;
        let service = servers.get(server)
            .ok_or_else(|| MCPError::ServerNotFound(server.to_string()))?;

        service.call_tool(tool, args).await
            .map_err(|e| MCPError::CallFailed(e.to_string()))
    }

    /// Liste tous les tools disponibles
    pub async fn get_all_tools(&self) -> HashMap<String, Vec<Tool>> {
        self.tools.read().await.clone()
    }

    /// Génère la description des tools pour le LLM
    pub async fn describe_tools_for_llm(&self) -> String {
        let tools = self.tools.read().await;
        let mut description = String::new();

        for (server, tool_list) in tools.iter() {
            description.push_str(&format!("### Server: {}\n\n", server));

            for tool in tool_list {
                description.push_str(&format!(
                    "- `mcp.{}.{}(args)`: {}\n",
                    server,
                    tool.name,
                    tool.description.as_deref().unwrap_or("No description")
                ));
            }
            description.push('\n');
        }

        description
    }
}
```

### 3.2 Redis Storage

```rust
// crates/agentic-storage/src/redis.rs

use redis::{AsyncCommands, Client, aio::ConnectionManager};
use async_trait::async_trait;

use crate::{
    traits::StateStore,
    error::StorageError,
};

pub struct RedisStore {
    conn: ConnectionManager,
}

impl RedisStore {
    pub async fn connect(url: &str) -> Result<Self, StorageError> {
        let client = Client::open(url)
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        let conn = ConnectionManager::new(client).await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        Ok(Self { conn })
    }
}

#[async_trait]
impl StateStore for RedisStore {
    async fn get<T: serde::de::DeserializeOwned + Send>(
        &self,
        key: &str,
    ) -> Result<Option<T>, StorageError> {
        let mut conn = self.conn.clone();
        let value: Option<String> = conn.get(key).await
            .map_err(|e| StorageError::GetFailed(e.to_string()))?;

        match value {
            Some(v) => {
                let parsed = serde_json::from_str(&v)
                    .map_err(|e| StorageError::DeserializationError(e.to_string()))?;
                Ok(Some(parsed))
            }
            None => Ok(None),
        }
    }

    async fn set<T: serde::Serialize + Send + Sync>(
        &self,
        key: &str,
        value: &T,
        ttl_ms: Option<u64>,
    ) -> Result<(), StorageError> {
        let mut conn = self.conn.clone();
        let serialized = serde_json::to_string(value)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        if let Some(ttl) = ttl_ms {
            conn.set_ex(key, &serialized, ttl / 1000).await
        } else {
            conn.set(key, &serialized).await
        }.map_err(|e| StorageError::SetFailed(e.to_string()))
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let mut conn = self.conn.clone();
        conn.del(key).await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let mut conn = self.conn.clone();
        conn.exists(key).await
            .map_err(|e| StorageError::QueryFailed(e.to_string()))
    }
}

/// Patterns de clés standards
pub struct KeyPatterns;

impl KeyPatterns {
    pub fn agent_state(agent_id: &str) -> String {
        format!("agents:{}:state", agent_id)
    }

    pub fn agent_metrics(agent_id: &str) -> String {
        format!("agents:{}:metrics", agent_id)
    }

    pub fn task_state(task_id: &str) -> String {
        format!("tasks:{}:state", task_id)
    }

    pub fn session(session_id: &str) -> String {
        format!("sessions:{}", session_id)
    }

    pub fn cache(key: &str) -> String {
        format!("cache:{}", key)
    }
}
```

### 3.3 Qdrant Vector Store

```rust
// crates/agentic-storage/src/qdrant.rs

use qdrant_client::{
    Qdrant,
    qdrant::{
        CreateCollectionBuilder, Distance, PointStruct,
        SearchPointsBuilder, VectorParamsBuilder,
        vectors_config::Config, VectorsConfig,
    },
};
use async_trait::async_trait;

use crate::{
    traits::{VectorStore, VectorPoint, VectorSearchResult},
    error::StorageError,
};

pub struct QdrantStore {
    client: Qdrant,
}

impl QdrantStore {
    pub async fn connect(url: &str) -> Result<Self, StorageError> {
        let client = Qdrant::from_url(url).build()
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        Ok(Self { client })
    }

    pub async fn create_collection(
        &self,
        name: &str,
        vector_size: u64,
    ) -> Result<(), StorageError> {
        self.client.create_collection(
            CreateCollectionBuilder::new(name)
                .vectors_config(VectorsConfig {
                    config: Some(Config::Params(
                        VectorParamsBuilder::new(vector_size, Distance::Cosine).build()
                    )),
                })
        ).await
            .map_err(|e| StorageError::CreateFailed(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl VectorStore for QdrantStore {
    async fn upsert(
        &self,
        collection: &str,
        points: Vec<VectorPoint>,
    ) -> Result<(), StorageError> {
        let qdrant_points: Vec<PointStruct> = points.into_iter().map(|p| {
            PointStruct::new(
                p.id,
                p.vector,
                serde_json::from_value(serde_json::to_value(p.payload).unwrap()).unwrap(),
            )
        }).collect();

        self.client.upsert_points(collection, None, qdrant_points, None).await
            .map_err(|e| StorageError::UpsertFailed(e.to_string()))?;

        Ok(())
    }

    async fn search(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<VectorSearchResult>, StorageError> {
        let mut search = SearchPointsBuilder::new(collection, vector, limit);

        if let Some(threshold) = score_threshold {
            search = search.score_threshold(threshold);
        }

        let results = self.client.search_points(search).await
            .map_err(|e| StorageError::SearchFailed(e.to_string()))?;

        Ok(results.result.into_iter().map(|r| {
            VectorSearchResult {
                id: match r.id.unwrap().point_id_options.unwrap() {
                    qdrant_client::qdrant::point_id::PointIdOptions::Uuid(u) => u,
                    qdrant_client::qdrant::point_id::PointIdOptions::Num(n) => n.to_string(),
                },
                score: r.score,
                payload: r.payload,
            }
        }).collect())
    }

    async fn delete(
        &self,
        collection: &str,
        ids: Vec<String>,
    ) -> Result<(), StorageError> {
        self.client.delete_points(
            collection,
            None,
            &ids.into_iter().map(|id| id.into()).collect::<Vec<_>>(),
            None,
        ).await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;

        Ok(())
    }
}
```

### 3.4 Tâches Phase 3

| Tâche | Semaine | Description | Livrable |
|-------|---------|-------------|----------|
| 3.1 | S1 | MCP Client base | `agentic-mcp/src/client.rs` |
| 3.2 | S1 | MCP Transport stdio | `agentic-mcp/src/transport.rs` |
| 3.3 | S1 | CodeExecutor avec MCP | `agentic-mcp/src/executor.rs` |
| 3.4 | S2 | StateStore trait | `agentic-storage/src/traits.rs` |
| 3.5 | S2 | Redis implementation | `agentic-storage/src/redis.rs` |
| 3.6 | S2 | VectorStore trait | Dans traits.rs |
| 3.7 | S2 | Qdrant implementation | `agentic-storage/src/qdrant.rs` |
| 3.8 | S3 | Agent + MCP integration | Dans agentic-core |
| 3.9 | S3 | Tests MCP | `tests/mcp_integration.rs` |
| 3.10 | S3 | Example mcp_agent | `examples/mcp_agent.rs` |

### Critères de Validation Phase 3
- [ ] MCP client se connecte aux servers
- [ ] Tool discovery fonctionne
- [ ] call_tool exécute correctement
- [ ] Redis get/set fonctionne
- [ ] Qdrant upsert/search fonctionne
- [ ] Agent peut utiliser MCP tools

---

## Phase 4: Production (3 semaines)

### Objectif
Observabilité, performance, sécurité, et documentation.

### 4.1 Tracing/Telemetry

```rust
// crates/agentic-core/src/telemetry.rs

use tracing_subscriber::{
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
    fmt,
};
use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use tracing_opentelemetry::OpenTelemetryLayer;

pub struct TelemetryConfig {
    pub service_name: String,
    pub otlp_endpoint: Option<String>,
    pub json_logs: bool,
}

pub fn init_telemetry(config: TelemetryConfig) -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = if config.json_logs {
        fmt::layer().json().boxed()
    } else {
        fmt::layer().boxed()
    };

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    if let Some(endpoint) = config.otlp_endpoint {
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .tonic()
                    .with_endpoint(endpoint)
            )
            .install_batch(opentelemetry_sdk::runtime::Tokio)?;

        let otel_layer = OpenTelemetryLayer::new(tracer);
        subscriber.with(otel_layer).init();
    } else {
        subscriber.init();
    }

    Ok(())
}

pub fn shutdown_telemetry() {
    global::shutdown_tracer_provider();
}
```

### 4.2 Docker Sandbox

```rust
// crates/agentic-sandbox/src/docker.rs

use bollard::{
    Docker,
    container::{Config, CreateContainerOptions, StartContainerOptions, LogsOptions, WaitContainerOptions},
    exec::{CreateExecOptions, StartExecResults},
};
use async_trait::async_trait;
use futures::StreamExt;

use crate::{
    traits::{Sandbox, ExecutionResult, SandboxArtifact},
    error::SandboxError,
};

pub struct DockerSandbox {
    docker: Docker,
    image: String,
    timeout_ms: u64,
    memory_limit: u64,
    cpu_limit: f64,
}

impl DockerSandbox {
    pub async fn new(image: &str) -> Result<Self, SandboxError> {
        let docker = Docker::connect_with_socket_defaults()
            .map_err(|e| SandboxError::ConnectionFailed(e.to_string()))?;

        Ok(Self {
            docker,
            image: image.to_string(),
            timeout_ms: 30000,
            memory_limit: 512 * 1024 * 1024, // 512MB
            cpu_limit: 1.0,
        })
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_memory_limit(mut self, bytes: u64) -> Self {
        self.memory_limit = bytes;
        self
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError> {
        let start = std::time::Instant::now();

        // Créer le conteneur
        let container_name = format!("agentic-{}", uuid::Uuid::new_v4());

        let config = Config {
            image: Some(self.image.clone()),
            cmd: Some(vec!["sh".to_string(), "-c".to_string(), code.to_string()]),
            host_config: Some(bollard::service::HostConfig {
                memory: Some(self.memory_limit as i64),
                nano_cpus: Some((self.cpu_limit * 1e9) as i64),
                network_mode: Some("none".to_string()), // Pas de réseau
                ..Default::default()
            }),
            ..Default::default()
        };

        let create_options = CreateContainerOptions {
            name: &container_name,
            platform: None,
        };

        self.docker.create_container(Some(create_options), config).await
            .map_err(|e| SandboxError::CreateFailed(e.to_string()))?;

        // Démarrer le conteneur
        self.docker.start_container(&container_name, None::<StartContainerOptions<String>>).await
            .map_err(|e| SandboxError::StartFailed(e.to_string()))?;

        // Attendre avec timeout
        let wait_result = tokio::time::timeout(
            std::time::Duration::from_millis(self.timeout_ms),
            self.docker.wait_container(&container_name, None::<WaitContainerOptions<String>>).next()
        ).await;

        // Récupérer les logs
        let mut stdout = String::new();
        let mut stderr = String::new();

        let log_options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            ..Default::default()
        };

        let mut logs = self.docker.logs(&container_name, Some(log_options));
        while let Some(log) = logs.next().await {
            match log {
                Ok(bollard::container::LogOutput::StdOut { message }) => {
                    stdout.push_str(&String::from_utf8_lossy(&message));
                }
                Ok(bollard::container::LogOutput::StdErr { message }) => {
                    stderr.push_str(&String::from_utf8_lossy(&message));
                }
                _ => {}
            }
        }

        // Nettoyer
        let _ = self.docker.remove_container(&container_name, None).await;

        let exit_code = match wait_result {
            Ok(Some(Ok(wait))) => wait.status_code,
            Ok(_) => -1,
            Err(_) => {
                // Timeout - tuer le conteneur
                let _ = self.docker.kill_container(&container_name, None::<bollard::container::KillContainerOptions<String>>).await;
                let _ = self.docker.remove_container(&container_name, None).await;
                return Err(SandboxError::Timeout);
            }
        };

        Ok(ExecutionResult {
            exit_code: exit_code as i32,
            stdout,
            stderr,
            execution_time_ms: start.elapsed().as_millis() as u64,
            artifacts: vec![],
        })
    }

    async fn is_ready(&self) -> Result<bool, SandboxError> {
        self.docker.ping().await
            .map(|_| true)
            .map_err(|e| SandboxError::NotReady(e.to_string()))
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        Ok(()) // Pas de cleanup global nécessaire
    }
}
```

### 4.3 Benchmarks

```rust
// benches/agent_benchmark.rs

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use tokio::runtime::Runtime;

use dasein_agentic_core::{Agent, AgentConfig, ExecutionMode};
use dasein_agentic_llm::MockLLMAdapter; // Mock pour benchmarks
use dasein_agentic_sandbox::ProcessSandbox;

fn agent_run_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("agent_run");

    for execution_mode in [ExecutionMode::Never, ExecutionMode::Task] {
        group.bench_with_input(
            BenchmarkId::new("mode", format!("{:?}", execution_mode)),
            &execution_mode,
            |b, &mode| {
                b.to_async(&rt).iter(|| async {
                    let config = AgentConfig {
                        name: "bench-agent".to_string(),
                        description: None,
                        system_prompt: "Tu es un assistant.".to_string(),
                        execution_mode: mode,
                        timeout_ms: 30000,
                        max_retries: 0,
                    };

                    let llm = MockLLMAdapter::new("Mock response with ```typescript\nconsole.log('test');\n```");
                    let agent = Agent::new(config, llm, None::<ProcessSandbox>);

                    agent.start().await.unwrap();

                    let task = dasein_agentic_core::TaskPayload {
                        action: "generate".to_string(),
                        spec: serde_json::json!({"task": "test"}),
                        inputs: None,
                        constraints: vec![],
                    };

                    let _ = agent.run(task).await;
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, agent_run_benchmark);
criterion_main!(benches);
```

### 4.4 Tâches Phase 4

| Tâche | Semaine | Description | Livrable |
|-------|---------|-------------|----------|
| 4.1 | S1 | Telemetry setup | `agentic-core/src/telemetry.rs` |
| 4.2 | S1 | OpenTelemetry integration | Traces distribuées |
| 4.3 | S1 | Metrics Prometheus | Via tracing |
| 4.4 | S2 | Docker sandbox | `agentic-sandbox/src/docker.rs` |
| 4.5 | S2 | Security hardening | Network isolation, limits |
| 4.6 | S2 | Error handling audit | Tous les crates |
| 4.7 | S3 | Benchmarks | `benches/` |
| 4.8 | S3 | Documentation rustdoc | Tous les crates |
| 4.9 | S3 | Examples complets | `examples/` |
| 4.10 | S3 | README + CHANGELOG | Root |

### Critères de Validation Phase 4
- [ ] Tracing fonctionne (logs structurés)
- [ ] OpenTelemetry exportable
- [ ] Docker sandbox sécurisé (no network, limits)
- [ ] Benchmarks exécutables
- [ ] Documentation générée (`cargo doc`)
- [ ] Tous les examples fonctionnent
- [ ] Coverage >80%
- [ ] Clippy sans warnings
- [ ] `cargo audit` sans vulnérabilités

---

## Métriques de Succès

| Phase | Métrique | Cible |
|-------|----------|-------|
| 0 | Setup complet | CI/CD fonctionnel |
| 1 | Agent single task | <50ms overhead (hors LLM) |
| 2 | Workflow 5 steps | <100ms coordination overhead |
| 3 | MCP call latency | <20ms par call |
| 4 | Memory footprint | <30MB par agent idle |

---

## Risques et Mitigations

| Risque | Probabilité | Impact | Mitigation |
|--------|-------------|--------|------------|
| async-trait performance | Faible | Faible | Boxing négligeable vs LLM latency |
| LLM SDK immature | Moyen | Moyen | Fallback sur HTTP direct |
| MCP SDK jeune | Moyen | Faible | rmcp officiel maintenu |
| Courbe apprentissage | Élevé | Moyen | Formation équipe, pair programming |

---

## Ressources Nécessaires

| Ressource | Quantité | Notes |
|-----------|----------|-------|
| Développeur Rust senior | 1 | Lead technique |
| Développeur Rust/TS | 1-2 | Implémentation |
| Infrastructure test | 1 | Docker, NATS, Redis, Qdrant |
| CI/CD | GitHub Actions | Inclus |

---

## Prochaines Étapes Immédiates

1. **Semaine 1:** Initialiser le workspace Cargo + CI
2. **Semaine 2:** Implémenter types + LLMAdapter trait
3. **Semaine 3:** Premier agent fonctionnel avec OpenAI

---

> **Maintainer:** @rbometon
> **Créé:** Janvier 2026
> **Dernière mise à jour:** Janvier 2026
