//! Interactive chat console for testing LLM adapters

use dasein_agentic_llm::{LLMAdapter, LLMMessage};
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
            Box::new(
                dasein_agentic_llm::GeminiAdapter::new(key.trim(), "gemini-2.0-flash")
                    .with_temperature(0.7),
            )
        }
        "2" => {
            print!("OpenAI API key: ");
            io::stdout().flush()?;
            let mut key = String::new();
            io::stdin().lock().read_line(&mut key)?;
            Box::new(
                dasein_agentic_llm::OpenAIAdapter::new(key.trim(), "gpt-4o-mini").with_temperature(0.7),
            )
        }
        "3" => {
            print!("Anthropic API key: ");
            io::stdout().flush()?;
            let mut key = String::new();
            io::stdin().lock().read_line(&mut key)?;
            Box::new(
                dasein_agentic_llm::AnthropicAdapter::new(key.trim(), "claude-3-5-haiku-20241022")
                    .with_temperature(0.7),
            )
        }
        "4" => {
            println!("Using Ollama at localhost:11434");
            Box::new(dasein_agentic_llm::OllamaAdapter::new("llama3.2").with_temperature(0.7))
        }
        _ => {
            println!("Invalid choice, using Gemini by default");
            print!("Gemini API key: ");
            io::stdout().flush()?;
            let mut key = String::new();
            io::stdin().lock().read_line(&mut key)?;
            Box::new(
                dasein_agentic_llm::GeminiAdapter::new(key.trim(), "gemini-2.0-flash")
                    .with_temperature(0.7),
            )
        }
    };

    println!("\n---------------------------------");
    println!("Provider: {}", adapter.provider());
    println!("Model: {}", adapter.model());
    println!("---------------------------------");
    println!("Commands:");
    println!("  /quit    - Exit chat");
    println!("  /clear   - Clear history");
    println!("  /stream  - Toggle streaming mode");
    println!("---------------------------------\n");

    let system_prompt = "You are a helpful AI assistant. Be concise and helpful.";
    let mut history: Vec<LLMMessage> = vec![LLMMessage::system(system_prompt)];
    let mut streaming = false;

    loop {
        print!("You: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().lock().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        match input {
            "/quit" | "/exit" | "/q" => {
                println!("Goodbye!");
                break;
            }
            "/clear" => {
                history = vec![LLMMessage::system(system_prompt)];
                println!("History cleared.\n");
                continue;
            }
            "/stream" => {
                streaming = !streaming;
                println!("Streaming: {}\n", if streaming { "ON" } else { "OFF" });
                continue;
            }
            _ => {}
        }

        history.push(LLMMessage::user(input));

        print!("Assistant: ");
        io::stdout().flush()?;

        if streaming {
            // Streaming mode
            use futures::StreamExt;
            let mut stream = adapter.generate_stream(&history);
            let mut full_response = String::new();

            while let Some(result) = stream.next().await {
                match result {
                    Ok(chunk) => {
                        print!("{}", chunk.content);
                        io::stdout().flush()?;
                        full_response.push_str(&chunk.content);
                    }
                    Err(e) => {
                        println!("\nError: {}", e);
                        break;
                    }
                }
            }
            println!("\n");

            if !full_response.is_empty() {
                history.push(LLMMessage::assistant(&full_response));
            }
        } else {
            // Non-streaming mode
            match adapter.generate(&history).await {
                Ok(response) => {
                    println!("{}\n", response.content.trim());
                    history.push(LLMMessage::assistant(&response.content));
                }
                Err(e) => {
                    println!("Error: {}\n", e);
                }
            }
        }
    }

    Ok(())
}
