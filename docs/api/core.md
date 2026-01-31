# agentic-core

Core agent runtime, types, and protocol definitions.

## Installation

```toml
[dependencies]
agentic-core = "0.1"
```

## Main Types

### Agent

```rust
pub struct Agent<L: LLMAdapter, S: Sandbox> {
    // ...
}

impl<L: LLMAdapter, S: Sandbox> Agent<L, S> {
    pub fn new(config: AgentConfig, llm: L, sandbox: Option<S>) -> Self;
    pub async fn start(&self) -> Result<(), AgentError>;
    pub async fn run(&self, task: TaskPayload) -> Result<ResultPayload, AgentError>;
    pub async fn stop(&self) -> Result<(), AgentError>;
    pub fn state(&self) -> AgentState;
    pub fn metrics(&self) -> AgentMetrics;
}
```

### AgentConfig

```rust
pub struct AgentConfig {
    pub name: String,
    pub execution_mode: ExecutionMode,
    pub timeout_ms: u64,
    pub max_retries: u32,
    pub system_prompt: Option<String>,
}

impl AgentConfig {
    pub fn builder() -> AgentConfigBuilder;
}
```

### ExecutionMode

```rust
pub enum ExecutionMode {
    Always,  // Always execute generated code
    Never,   // Never execute, return code only
    Auto,    // LLM decides via markers
    Task,    // Based on task type
}
```

### TaskPayload

```rust
pub struct TaskPayload {
    pub instruction: String,
    pub context: Option<String>,
    pub action_type: ActionType,
    pub parameters: HashMap<String, Value>,
}

impl TaskPayload {
    pub fn new(instruction: impl Into<String>) -> Self;
    pub fn with_context(self, context: impl Into<String>) -> Self;
}
```

### ResultPayload

```rust
pub struct ResultPayload {
    pub status: ResultStatus,
    pub data: HashMap<String, Value>,
    pub artifacts: Vec<Artifact>,
    pub execution_time_ms: u64,
    pub tokens_used: Option<TokenUsage>,
}

pub enum ResultStatus {
    Success,
    Failed,
    Partial,
}
```

### AgentState

```rust
pub enum AgentState {
    Created,
    Idle,
    Running,
    Stopped,
    Failed,
}
```

## Protocol Types

### Message

```rust
pub struct Message {
    pub metadata: Metadata,
    pub routing: RoutingInfo,
    pub content: MessageContent,
    pub control: ControlSettings,
}
```

### MessageContent

```rust
pub enum MessageContent {
    Task { payload: TaskPayload },
    Result { payload: ResultPayload },
    Event { event_type: String, data: Value },
    Error { code: String, message: String, recoverable: bool },
}
```

## Errors

```rust
pub enum AgentError {
    NotStarted,
    AlreadyRunning,
    LLMError(LLMError),
    SandboxError(SandboxError),
    Timeout,
    ConfigError(String),
}
```
