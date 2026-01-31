# Chat Console Example

An interactive CLI chat application supporting multiple LLM providers.

## Run

```bash
cargo run --example chat --features "all"
```

## Features

- **Multi-provider support** - Choose between Gemini, OpenAI, Anthropic, or Ollama
- **Conversation history** - Full context maintained across messages
- **Streaming mode** - Toggle real-time streaming responses
- **Commands** - `/quit`, `/clear`, `/stream`

## Usage

```
=================================
   Agentic-RS Chat Console
=================================

Select LLM provider:
  1. Gemini (gemini-2.0-flash)
  2. OpenAI (gpt-4o-mini)
  3. Anthropic (claude-3-5-haiku-20241022)
  4. Ollama (llama3.2)

Choice [1-4]: 1
Gemini API key: AIza...

---------------------------------
Provider: gemini
Model: gemini-2.0-flash
---------------------------------
Commands:
  /quit    - Exit chat
  /clear   - Clear history
  /stream  - Toggle streaming mode
---------------------------------

You: Hello!
Assistant: Hello! How can I help you today?

You: What is Rust?
Assistant: Rust is a systems programming language focused on safety,
speed, and concurrency. It achieves memory safety without garbage
collection through its ownership system...

You: /stream
Streaming: ON

You: Tell me more
Assistant: [streams response in real-time]

You: /quit
Goodbye!
```

## Source Code

```rust
//! Interactive chat console for testing LLM adapters

use agentic_llm::{LLMAdapter, LLMMessage};
use std::io::{self, BufRead, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=================================");
    println!("   Agentic-RS Chat Console");
    println!("=================================\n");

    // Select provider
    println!("Select LLM provider:");
    println!("  1. Gemini (gemini-2.0-flash)");
    println!("  2. OpenAI (gpt-4o-mini)");
    println!("  3. Anthropic (claude-3-5-haiku-20241022)");
    println!("  4. Ollama (llama3.2)");
    print!("\nChoice [1-4]: ");
    io::stdout().flush()?;

    let mut choice = String::new();
    io::stdin().lock().read_line(&mut choice)?;

    let adapter: Box<dyn LLMAdapter> = match choice.trim() {
        "1" => {
            print!("Gemini API key: ");
            io::stdout().flush()?;
            let mut key = String::new();
            io::stdin().lock().read_line(&mut key)?;
            Box::new(agentic_llm::GeminiAdapter::new(key.trim(), "gemini-2.0-flash"))
        }
        "2" => {
            print!("OpenAI API key: ");
            io::stdout().flush()?;
            let mut key = String::new();
            io::stdin().lock().read_line(&mut key)?;
            Box::new(agentic_llm::OpenAIAdapter::new(key.trim(), "gpt-4o-mini"))
        }
        // ... more providers
        _ => panic!("Invalid choice"),
    };

    let mut history = vec![LLMMessage::system("You are a helpful assistant.")];
    let mut streaming = false;

    loop {
        print!("You: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().lock().read_line(&mut input)?;

        match input.trim() {
            "/quit" => break,
            "/clear" => { history.clear(); continue; }
            "/stream" => { streaming = !streaming; continue; }
            msg => history.push(LLMMessage::user(msg)),
        }

        if streaming {
            use futures::StreamExt;
            let mut stream = adapter.generate_stream(&history);
            let mut response = String::new();
            print!("Assistant: ");
            while let Some(Ok(chunk)) = stream.next().await {
                print!("{}", chunk.content);
                response.push_str(&chunk.content);
            }
            println!();
            history.push(LLMMessage::assistant(&response));
        } else {
            let response = adapter.generate(&history).await?;
            println!("Assistant: {}", response.content);
            history.push(LLMMessage::assistant(&response.content));
        }
    }

    Ok(())
}
```
