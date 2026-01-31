//! Test Gemini adapter
//!
//! Usage:
//!   GEMINI_API_KEY=your-key cargo run --example test_gemini

use agentic_llm::{GeminiAdapter, LLMAdapter, LLMMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt::init();

    let api_key = std::env::var("GEMINI_API_KEY").expect(
        "GEMINI_API_KEY environment variable not set.\n\
         Usage: GEMINI_API_KEY=your-key cargo run --example test_gemini",
    );

    let gemini = GeminiAdapter::new(api_key, "gemini-2.0-flash").with_temperature(0.7);

    println!("Testing Gemini adapter...");
    println!("Provider: {}", gemini.provider());
    println!("Model: {}", gemini.model());

    // Health check
    println!("\n1. Health check...");
    match gemini.health_check().await {
        Ok(true) => println!("   ✓ API accessible"),
        Ok(false) => println!("   ✗ API not accessible"),
        Err(e) => println!("   ✗ Error: {}", e),
    }

    // Generate completion
    println!("\n2. Generate completion...");
    let messages = vec![
        LLMMessage::system("You are a helpful assistant. Be concise."),
        LLMMessage::user("What is 2 + 2? Answer in one word."),
    ];

    match gemini.generate(&messages).await {
        Ok(response) => {
            println!("   ✓ Response: {}", response.content.trim());
            println!("   Model: {}", response.model);
            println!(
                "   Tokens: prompt={}, completion={}, total={}",
                response.tokens_used.prompt,
                response.tokens_used.completion,
                response.tokens_used.total
            );
        }
        Err(e) => println!("   ✗ Error: {}", e),
    }

    // Test streaming
    println!("\n3. Stream completion...");
    let messages = vec![LLMMessage::user("Count from 1 to 5, one number per line.")];

    use futures::StreamExt;
    let mut stream = gemini.generate_stream(&messages);
    print!("   Response: ");
    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => {
                print!("{}", chunk.content);
                if chunk.done {
                    println!("\n   ✓ Stream complete");
                }
            }
            Err(e) => {
                println!("\n   ✗ Stream error: {}", e);
                break;
            }
        }
    }

    println!("\nTest complete!");
    Ok(())
}
