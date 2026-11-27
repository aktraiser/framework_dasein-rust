> **Local beta profile:** [launch the Docker/NATS laboratory](docs/BETA_LOCAL.md). Read [validated capabilities and release gates](docs/SDK_STATUS.md); [Firecracker has a qualified local KVM profile](server/firecracker-guest/README.md).

<p align="center">
  <h1 align="center">Dasein Agentic SDK</h1>
  <p align="center">
    <strong>Build autonomous AI agents in Rust</strong>
  </p>
  <p align="center">
    <a href="https://github.com/aktraiser/framework_dasein-rust/actions/workflows/ci.yml"><img src="https://github.com/aktraiser/framework_dasein-rust/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="https://github.com/aktraiser/framework_dasein-rust/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
    <a href="https://github.com/aktraiser/framework_dasein-rust/stargazers"><img src="https://img.shields.io/github/stars/aktraiser/framework_dasein-rust?style=social" alt="Stars"></a>
  </p>
</p>

---

A high-performance framework for building **multi-agent AI systems** in Rust. Connect to any LLM, execute code safely, and orchestrate complex workflows.

## Why Dasein?

- **Multi-LLM** - OpenAI, Anthropic, Gemini, Ollama (local models), with timeouts, safe retries and streaming
- **Tools** - Agents call your functions or any MCP server (HTTP or stdio)
- **Code Execution** - Ephemeral Docker containers or Firecracker microVMs with no network and resource limits; `ProcessSandbox` for trusted local development only
- **Workflows** - Graph engine with parallel branches, joins, retries and checkpointed resume
- **Durable work** - A NATS JetStream queue shares tasks between processes
- **Gateway** - An OpenAI-compatible endpoint with per-key limits, usage log and an admin dashboard

## Status

This is a **local beta**: built to run on one machine, in a controlled setting. [SDK_STATUS.md](docs/SDK_STATUS.md) lists what is validated and what is not. In short:

- The graph engine, tool-calling agents, Docker execution, the durable queue and the gateway are covered by tests, including against real NATS, Redis and Docker in CI.
- LLM adapters, the MCP client and the chat platforms are tested against mock servers, not against the live services.
- Agents are coordinated inside one process. Spreading a workflow across machines is not implemented beyond the shared task queue.
- There is no OpenTelemetry or Prometheus export yet.

## Quick Start

The crates published on crates.io (0.1.0) predate the security and reliability fixes in this repository. Until a new release, depend on the repository:

```toml
[dependencies]
dasein-agentic-llm = { git = "https://github.com/aktraiser/framework_dasein-rust", features = ["anthropic"] }
tokio = { version = "1", features = ["full"] }
```

```rust
use dasein_agentic_llm::{AnthropicAdapter, LLMAdapter, LLMMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = AnthropicAdapter::new(
        std::env::var("ANTHROPIC_API_KEY")?,
        "claude-sonnet-4-20250514"
    );

    let response = llm.generate(&[
        LLMMessage::system("You are a helpful assistant."),
        LLMMessage::user("What is Rust?"),
    ]).await?;

    println!("{}", response.content);
    Ok(())
}
```

## LLM Providers

| Provider | Feature | Models |
|----------|---------|--------|
| **Anthropic** | `anthropic` | Claude Sonnet, Haiku, Opus |
| **OpenAI** | `openai` | GPT-4o, o1, o3 |
| **Google** | `gemini` | Gemini 2.0 Flash, 1.5 Pro |
| **Ollama** | `ollama` | Llama, Mistral, Qwen (local) |

## Architecture

```
          your application
                 │
   ┌─────────────┴──────────────┐
   │      dasein-agentic-core    │   graph workflows, agents, tool loop,
   │                             │   checkpoints, durable queue (NATS)
   └──┬──────────┬───────────┬──┘
      │          │           │
  ┌───┴───┐  ┌───┴────┐  ┌───┴────┐
  │  llm  │  │ sandbox │  │  mcp   │
  └───┬───┘  └───┬────┘  └───┬────┘
      │          │           │
  providers   Docker /    MCP servers
              Firecracker (HTTP, stdio)
```

Agents and workflows run inside your process. NATS carries the durable task queue, the audit trail and shared memory; it is optional for a single in-process workflow.

## Crates

| Crate | Description |
|-------|-------------|
| `dasein-agentic-core` | Graph workflows, agents, durable queue, persistence |
| `dasein-agentic-llm` | LLM adapters (OpenAI, Anthropic, Gemini, Ollama) |
| `dasein-agentic-sandbox` | Sandboxed code execution (Docker, Firecracker, process) |
| `dasein-agentic-mcp` | Model Context Protocol client (HTTP and stdio) |
| `dasein-agentic-tools` | Ready-made tools (web search, Google Workspace) |
| `dasein-agentic-skills` | Skill registry and routing |
| `dasein-agentic-chat-platforms` | Discord, Telegram, Slack (send only) |
| `dasein-agentic-storage` | Redis and Qdrant stores |
| `dasein-agentic-bus`, `dasein-agentic-orchestrator` | Earlier bus and orchestrator; not used by the other crates |

The [gateway](gateway/README.md) is a separate workspace under `gateway/`.

## Examples

```bash
# Interactive chat
cargo run --example chat

# Single agent with sandbox
cargo run --example single_agent

# Graph workflow
cargo run --example graph_workflow
```

See [examples/](examples/) for more.

## Documentation

- [Getting Started](docs/guide/index.md)
- [API Reference](https://docs.rs/dasein-agentic-core)
- [Examples](docs/examples/index.md)

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

Looking for something to work on? Check out issues labeled [`good first issue`](https://github.com/aktraiser/framework_dasein-rust/labels/good%20first%20issue).

## License

MIT License - see [LICENSE](LICENSE)

---

<p align="center">
  <sub>Built with Rust for reliability and performance</sub>
</p>
