# Examples

## Available Examples

| Example | Description | Run |
|---------|-------------|-----|
| [Chat Console](/examples/chat) | Interactive CLI chat with any LLM | `cargo run --example chat` |
| [Single Agent](/examples/single-agent) | Basic agent with sandbox | `cargo run --example single_agent` |
| [Test Gemini](/examples/single-agent) | Test Gemini adapter | `cargo run --example test_gemini` |

## Running Examples

```bash
# Clone the repository
git clone https://github.com/your-org/agentic-rs
cd agentic-rs

# Run with all features
cargo run --example chat --features "all"
```

## Example Code

### Minimal LLM Call

```rust
use dasein_agentic_llm::{GeminiAdapter, LLMAdapter, LLMMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = GeminiAdapter::new("API_KEY", "gemini-2.0-flash");

    let response = llm.generate(&[
        LLMMessage::user("Hello!")
    ]).await?;

    println!("{}", response.content);
    Ok(())
}
```

### Multi-Turn Conversation

```rust
use dasein_agentic_llm::{AnthropicAdapter, LLMAdapter, LLMMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = AnthropicAdapter::new("API_KEY", "claude-3-5-haiku-20241022");

    let mut history = vec![
        LLMMessage::system("You are a helpful math tutor."),
    ];

    // First turn
    history.push(LLMMessage::user("What is calculus?"));
    let response = llm.generate(&history).await?;
    println!("Assistant: {}", response.content);
    history.push(LLMMessage::assistant(&response.content));

    // Second turn
    history.push(LLMMessage::user("Can you give me an example?"));
    let response = llm.generate(&history).await?;
    println!("Assistant: {}", response.content);

    Ok(())
}
```

### Streaming Response

```rust
use dasein_agentic_llm::{OpenAIAdapter, LLMAdapter, LLMMessage};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = OpenAIAdapter::new("API_KEY", "gpt-4o-mini");

    let mut stream = llm.generate_stream(&[
        LLMMessage::user("Write a poem about Rust")
    ]);

    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => print!("{}", chunk.content),
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    Ok(())
}
```

### Compare Providers

```rust
use dasein_agentic_llm::{
    OpenAIAdapter, AnthropicAdapter, GeminiAdapter, OllamaAdapter,
    LLMAdapter, LLMMessage
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = vec![LLMMessage::user("What is 2+2?")];

    let providers: Vec<Box<dyn LLMAdapter>> = vec![
        Box::new(OpenAIAdapter::new("...", "gpt-4o-mini")),
        Box::new(AnthropicAdapter::new("...", "claude-3-5-haiku-20241022")),
        Box::new(GeminiAdapter::new("...", "gemini-2.0-flash")),
        Box::new(OllamaAdapter::new("llama3.2")),
    ];

    for provider in providers {
        match provider.generate(&prompt).await {
            Ok(r) => println!("{}/{}: {}", provider.provider(), provider.model(), r.content.trim()),
            Err(e) => println!("{}: Error - {}", provider.provider(), e),
        }
    }

    Ok(())
}
```
