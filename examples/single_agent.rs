//! Single agent example.
//!
//! Run with: cargo run --example single_agent
//!
//! Requires: OPENAI_API_KEY environment variable

use agentic_core::{Agent, AgentConfig, ExecutionMode, TaskPayload};
use agentic_llm::OllamaAdapter;
use agentic_sandbox::ProcessSandbox;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    println!("=== Agentic-RS Single Agent Example ===\n");

    // Configure the agent
    let config = AgentConfig {
        name: "assistant".to_string(),
        description: Some("A helpful coding assistant".to_string()),
        system_prompt: r#"
You are a helpful coding assistant.
When asked to write code, provide complete, working examples.
Use clear variable names and add comments for complex logic.
"#
        .to_string(),
        execution_mode: ExecutionMode::Never, // Don't execute, just generate
        timeout_ms: 60000,
        max_retries: 3,
    };

    // Create LLM adapter (using Ollama for local testing)
    // Change to OpenAIAdapter for production
    let llm = OllamaAdapter::new("llama3.2");

    // Create sandbox (not used in Never mode, but required for type)
    let sandbox = ProcessSandbox::new();

    // Create agent
    let agent = Agent::new(config, llm, Some(sandbox));

    // Start agent
    println!("Starting agent...");
    agent.start().await?;
    println!("Agent started!\n");

    // Create a task
    let task = TaskPayload {
        action: "generate".to_string(),
        spec: serde_json::json!({
            "task": "Write a Rust function that checks if a number is prime",
            "requirements": [
                "Handle edge cases (0, 1, negative numbers)",
                "Use efficient algorithm",
                "Include documentation"
            ]
        }),
        inputs: None,
        constraints: vec!["Must be no_std compatible".to_string()],
    };

    // Run task
    println!("Running task: {}", task.action);
    println!("Spec: {}\n", serde_json::to_string_pretty(&task.spec)?);

    let result = agent.run(task).await?;

    // Display result
    println!("=== Result ===");
    println!("Status: {:?}", result.status);
    println!("Execution time: {}ms", result.metrics.execution_time_ms);

    if let Some(tokens) = result.metrics.tokens_used {
        println!("Tokens used: {}", tokens);
    }

    if let Some(data) = &result.data {
        if let Some(output) = data.get("output") {
            println!("\n=== Generated Code ===\n{}", output);
        }
    }

    // Get metrics
    let metrics = agent.get_metrics().await;
    println!("\n=== Agent Metrics ===");
    println!("Total tasks: {}", metrics.total_tasks);
    println!("Successful: {}", metrics.successful_tasks);
    println!("Failed: {}", metrics.failed_tasks);
    println!("Avg execution time: {:.2}ms", metrics.average_execution_time_ms);

    // Stop agent
    agent.stop().await?;
    println!("\nAgent stopped.");

    Ok(())
}
