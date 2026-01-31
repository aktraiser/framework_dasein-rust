# API Reference

## Crates Overview

| Crate | Description | Docs |
|-------|-------------|------|
| [agentic-core](/api/core) | Core agent runtime and types | [docs.rs](https://docs.rs/agentic-core) |
| [agentic-llm](/api/llm) | LLM adapters | [docs.rs](https://docs.rs/agentic-llm) |
| [agentic-sandbox](/api/sandbox) | Sandboxed execution | [docs.rs](https://docs.rs/agentic-sandbox) |
| [agentic-bus](/api/bus) | NATS message bus | [docs.rs](https://docs.rs/agentic-bus) |
| [agentic-storage](/api/storage) | Redis + Qdrant storage | [docs.rs](https://docs.rs/agentic-storage) |
| [agentic-orchestrator](/api/orchestrator) | Multi-agent workflows | [docs.rs](https://docs.rs/agentic-orchestrator) |
| [agentic-mcp](/api/mcp) | MCP protocol client | [docs.rs](https://docs.rs/agentic-mcp) |

## Quick Links

### Core Types

```rust
use agentic_core::{
    Agent,           // Main agent struct
    AgentConfig,     // Agent configuration
    TaskPayload,     // Task input
    ResultPayload,   // Task output
    ExecutionMode,   // Always/Never/Auto/Task
    AgentState,      // Running/Idle/Stopped
};
```

### LLM Types

```rust
use agentic_llm::{
    // Adapters
    OpenAIAdapter,
    AnthropicAdapter,
    GeminiAdapter,
    OllamaAdapter,

    // Traits
    LLMAdapter,

    // Types
    LLMMessage,
    LLMResponse,
    StreamChunk,
    TokenUsage,
    FinishReason,
    LLMError,
};
```

### Sandbox Types

```rust
use agentic_sandbox::{
    ProcessSandbox,   // Local process execution
    DockerSandbox,    // Container execution
    Sandbox,          // Trait
    ExecutionResult,
    SandboxError,
};
```

### Storage Types

```rust
use agentic_storage::{
    RedisStore,       // Key-value state
    QdrantStore,      // Vector search
    StateStore,       // Trait
    VectorStore,      // Trait
    VectorPoint,
    VectorSearchResult,
};
```
