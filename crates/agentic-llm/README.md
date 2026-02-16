# agentic-llm

LLM adapters for the Agentic Framework.

## Features

- **Multiple Providers**: OpenAI, Anthropic, Google Gemini, Ollama
- **Unified Interface**: Single `LLMAdapter` trait for all providers
- **Streaming Support**: Real-time token streaming
- **Async-first**: Built on Tokio

## Installation

```toml
[dependencies]
agentic-llm = "0.1"

# Or with specific providers
agentic-llm = { version = "0.1", features = ["openai", "anthropic"] }
```

## Quick Start

```rust
use agentic_llm::{OpenAIAdapter, LLMAdapter, LLMMessage, Role};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = OpenAIAdapter::new("sk-...", "gpt-4");

    let messages = vec![
        LLMMessage::new(Role::User, "Hello, world!")
    ];

    let response = adapter.generate(&messages).await?;
    println!("{}", response.content);

    Ok(())
}
```

## Providers

| Provider | Feature Flag | Environment Variable |
|----------|--------------|---------------------|
| OpenAI | `openai` | `OPENAI_API_KEY` |
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` |
| Gemini | `gemini` | `GEMINI_API_KEY` |
| Ollama | `ollama` | `OLLAMA_HOST` |

## License

MIT
