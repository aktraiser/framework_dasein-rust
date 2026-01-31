# Agentic Framework (Rust)

> High-performance framework for building autonomous multi-agent AI systems in Rust.

[![CI](https://github.com/your-org/agentic-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/your-org/agentic-rs/actions)
[![Crates.io](https://img.shields.io/crates/v/agentic-core.svg)](https://crates.io/crates/agentic-core)
[![Documentation](https://docs.rs/agentic-core/badge.svg)](https://docs.rs/agentic-core)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## Features

- **Multi-LLM Support** - OpenAI, Ollama (Anthropic, Gemini coming soon)
- **Sandboxed Execution** - Safe code execution with Docker isolation
- **Message Bus** - NATS-based distributed communication
- **Persistent Storage** - Redis (state) + Qdrant (vectors)
- **Type-Safe** - Full Rust type safety with zero runtime overhead
- **Async-First** - Built on Tokio for maximum concurrency

---

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
agentic-core = "0.1"
agentic-llm = { version = "0.1", features = ["openai"] }
agentic-sandbox = { version = "0.1", features = ["process"] }
tokio = { version = "1", features = ["full"] }
```

### Create an Agent

```rust
use dasein_agentic_core::{Agent, AgentConfig, TaskPayload, ExecutionMode};
use dasein_agentic_llm::OpenAIAdapter;
use dasein_agentic_sandbox::ProcessSandbox;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure the agent
    let config = AgentConfig {
        name: "coder".to_string(),
        description: Some("A coding assistant".to_string()),
        system_prompt: "You are an expert Rust programmer.".to_string(),
        execution_mode: ExecutionMode::Task,
        timeout_ms: 30000,
        max_retries: 3,
    };

    // Create LLM adapter
    let llm = OpenAIAdapter::new(
        std::env::var("OPENAI_API_KEY")?,
        "gpt-4o"
    );

    // Create sandbox
    let sandbox = ProcessSandbox::new();

    // Create and start agent
    let agent = Agent::new(config, llm, Some(sandbox));
    agent.start().await?;

    // Run a task
    let task = TaskPayload {
        action: "generate".to_string(),
        spec: serde_json::json!({
            "task": "Write a function that calculates fibonacci numbers"
        }),
        inputs: None,
        constraints: vec![],
    };

    let result = agent.run(task).await?;
    println!("Result: {:?}", result);

    agent.stop().await?;
    Ok(())
}
```

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         AGENTIC-RS                                   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                      ORCHESTRATOR                             │   │
│  │  • Agent registry  • Workflow execution  • Task routing       │   │
│  └──────────────────────────────────┬───────────────────────────┘   │
│                                     │                                │
│                    ┌────────────────┴────────────────┐              │
│                    │         NATS BUS                │              │
│                    │  • Pub/Sub  • Request/Reply     │              │
│                    └────────────────┬────────────────┘              │
│                                     │                                │
│       ┌─────────────────────────────┼─────────────────────────┐     │
│       │                             │                         │     │
│       ▼                             ▼                         ▼     │
│  ┌─────────┐                   ┌─────────┐               ┌─────────┐│
│  │  AGENT  │                   │  AGENT  │               │  AGENT  ││
│  │┌───────┐│                   │┌───────┐│               │┌───────┐││
│  ││  LLM  ││                   ││  LLM  ││               ││  LLM  │││
│  │├───────┤│                   │├───────┤│               │├───────┤││
│  ││SANDBOX││                   ││SANDBOX││               ││SANDBOX│││
│  │└───────┘│                   │└───────┘│               │└───────┘││
│  └─────────┘                   └─────────┘               └─────────┘│
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                      STORAGE LAYER                            │   │
│  │  ┌─────────────────────┐      ┌─────────────────────┐        │   │
│  │  │   Redis (State)     │      │   Qdrant (Vectors)  │        │   │
│  │  └─────────────────────┘      └─────────────────────┘        │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Crates

| Crate | Description |
|-------|-------------|
| `agentic-core` | Core agent runtime, types, protocol |
| `agentic-llm` | LLM adapters (OpenAI, Ollama, etc.) |
| `agentic-sandbox` | Code execution (Process, Docker) |
| `agentic-bus` | NATS message bus |
| `agentic-storage` | Redis + Qdrant storage |
| `agentic-mcp` | MCP protocol client |
| `agentic-orchestrator` | Multi-agent workflows |

---

## LLM Providers

| Provider | Feature | Status |
|----------|---------|--------|
| OpenAI | `openai` | ✅ Ready |
| Ollama | `ollama` | ✅ Ready |
| Anthropic | `anthropic` | 🚧 Coming |
| Gemini | `gemini` | 🚧 Coming |

---

## Execution Modes

| Mode | Behavior |
|------|----------|
| `Always` | Always execute generated code |
| `Never` | Never execute (return code only) |
| `Auto` | LLM decides via `[EXECUTE]`/`[NO_EXECUTE]` markers |
| `Task` | Based on action type (default) |

---

## Development

```bash
# Build
cargo build --workspace

# Test
cargo test --workspace

# Lint
cargo clippy --workspace --all-features

# Format
cargo fmt --all

# Docs
cargo doc --workspace --open
```

---

## Performance

| Metric | Value |
|--------|-------|
| Agent overhead (excl. LLM) | ~5ms |
| Memory per agent (idle) | ~20MB |
| Concurrent agents | 1000+ |

---

## License

MIT License - see [LICENSE](LICENSE)

---

## Contributing

Contributions welcome! Please read our contributing guidelines first.
