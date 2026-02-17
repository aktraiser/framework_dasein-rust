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

- **Multi-LLM** - OpenAI, Anthropic, Gemini, Ollama (local models)
- **Safe Execution** - Sandboxed code execution with process isolation
- **Distributed** - NATS-based message bus for scaling
- **Type-Safe** - Full Rust type safety, zero runtime overhead
- **Async-First** - Built on Tokio for maximum concurrency

## Quick Start

```bash
cargo add dasein-agentic-core dasein-agentic-llm tokio --features "dasein-agentic-llm/anthropic, tokio/full"
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
┌─────────────────────────────────────────────────────────────┐
│                    DASEIN AGENTIC SDK                       │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                   ORCHESTRATOR                       │   │
│  │   Agent Registry • Workflow Engine • Task Router     │   │
│  └───────────────────────────┬─────────────────────────┘   │
│                              │                              │
│              ┌───────────────┴───────────────┐             │
│              │          NATS BUS             │             │
│              │    Pub/Sub • Request/Reply    │             │
│              └───────────────┬───────────────┘             │
│                              │                              │
│     ┌────────────────────────┼────────────────────────┐    │
│     ▼                        ▼                        ▼    │
│ ┌────────┐              ┌────────┐              ┌────────┐ │
│ │ AGENT  │              │ AGENT  │              │ AGENT  │ │
│ │┌──────┐│              │┌──────┐│              │┌──────┐│ │
│ ││ LLM  ││              ││ LLM  ││              ││ LLM  ││ │
│ │├──────┤│              │├──────┤│              │├──────┤│ │
│ ││SANDBOX│              ││SANDBOX│              ││SANDBOX│ │
│ │└──────┘│              │└──────┘│              │└──────┘│ │
│ └────────┘              └────────┘              └────────┘ │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## Crates

| Crate | Description |
|-------|-------------|
| `dasein-agentic-core` | Core runtime, executor trait, protocols |
| `dasein-agentic-llm` | LLM adapters (OpenAI, Anthropic, Gemini, Ollama) |
| `dasein-agentic-sandbox` | Sandboxed code execution |
| `dasein-agentic-bus` | NATS message bus |
| `dasein-agentic-orchestrator` | Multi-agent workflows |
| `dasein-agentic-mcp` | Model Context Protocol client |

## Examples

```bash
# Interactive chat
cargo run --example chat --features "anthropic"

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
