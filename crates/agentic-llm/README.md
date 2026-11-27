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
use dasein_agentic_llm::{OpenAIAdapter, LLMAdapter, LLMMessage, Role};

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

## OpenAI reasoning models

`OpenAIAdapter` routes GPT-5 and later (including GPT-5.5 and GPT-5.6 Luna)
and o1/o3/o4 models through `/v1/responses`; older GPT models keep Chat
Completions. `OpenAIResponsesAdapter` is also available explicitly.

Responses requests omit temperature, retain the provider's default reasoning
configuration, send `max_output_tokens`, and use `store: false`. Optional tool
parameters remain optional (`strict: false`). Output tokens include reasoning.
Function outputs use the provider's `call_id`, not its item `id`.

The first `ToolCall` in each response carries opaque `provider_metadata` with
all original output items, including encrypted reasoning and assistant phase.
These survive thread serialization and are replayed before function results.
Never send this metadata to tool handlers or substitute it for user-facing
reasoning. The laboratory omits it from observable model event payloads.
Old serialized calls without metadata remain readable; their missing reasoning
state cannot be reconstructed. Streaming through this Responses implementation
returns one buffered final chunk; the laboratory uses non-streaming tool calls.

Protocol regressions (local HTTP fixtures, no model fees):

```sh
cargo test -p dasein-agentic-llm --lib openai
```

Specification: [Function calling](https://developers.openai.com/api/docs/guides/function-calling)
and [stateless reasoning](https://developers.openai.com/api/docs/guides/reasoning).
