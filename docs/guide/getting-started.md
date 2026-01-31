# Getting Started

## Prerequisites

- **Rust 1.75+** - Install via [rustup](https://rustup.rs/)
- **API Keys** (optional) - For cloud LLM providers

## Installation

Add the crates you need to your `Cargo.toml`:

```toml
[dependencies]
agentic-core = "0.1"
agentic-llm = { version = "0.1", features = ["openai", "anthropic", "gemini", "ollama"] }
agentic-sandbox = "0.1"
tokio = { version = "1", features = ["full"] }
```

Or add them via cargo:

```bash
cargo add agentic-core agentic-llm agentic-sandbox tokio --features tokio/full
```

## Quick Example

### 1. Simple LLM Call

```rust
use agentic_llm::{GeminiAdapter, LLMAdapter, LLMMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create an adapter
    let llm = GeminiAdapter::new("YOUR_API_KEY", "gemini-2.0-flash")
        .with_temperature(0.7);

    // Build messages
    let messages = vec![
        LLMMessage::system("You are a helpful assistant."),
        LLMMessage::user("What is Rust?"),
    ];

    // Generate response
    let response = llm.generate(&messages).await?;
    println!("{}", response.content);
    println!("Tokens used: {}", response.tokens_used.total);

    Ok(())
}
```

### 2. Full Agent with Sandbox

```rust
use agentic_core::{Agent, AgentConfig, ExecutionMode, TaskPayload};
use agentic_llm::OpenAIAdapter;
use agentic_sandbox::ProcessSandbox;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure the agent
    let config = AgentConfig::builder()
        .name("code-assistant")
        .execution_mode(ExecutionMode::Auto)
        .timeout_ms(30000)
        .build();

    // Create LLM adapter
    let llm = OpenAIAdapter::new("sk-...", "gpt-4o")
        .with_temperature(0.7);

    // Create sandbox
    let sandbox = ProcessSandbox::new()
        .with_timeout(10000);

    // Create and start agent
    let agent = Agent::new(config, llm, Some(sandbox));
    agent.start().await?;

    // Run a task
    let task = TaskPayload::new("Write a Python script that prints the Fibonacci sequence up to 100");
    let result = agent.run(task).await?;

    println!("Status: {:?}", result.status);
    println!("Output: {}", result.data.get("output").unwrap_or(&"".into()));

    Ok(())
}
```

### 3. Streaming Response

```rust
use agentic_llm::{AnthropicAdapter, LLMAdapter, LLMMessage};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = AnthropicAdapter::new("sk-ant-...", "claude-3-5-haiku-20241022");

    let messages = vec![
        LLMMessage::user("Write a haiku about Rust programming"),
    ];

    let mut stream = llm.generate_stream(&messages);

    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => print!("{}", chunk.content),
            Err(e) => eprintln!("Error: {}", e),
        }
    }
    println!();

    Ok(())
}
```

## Next Steps

- Learn about [Architecture](/guide/architecture)
- Explore [LLM Adapters](/guide/llm-adapters)
- Try the [Examples](/examples/)
