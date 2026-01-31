# Single Agent Example

Basic example of creating and running a single agent.

## Run

```bash
cargo run --example single_agent
```

## Description

This example demonstrates:
- Creating an agent with LLM and sandbox
- Configuring execution mode
- Running a code generation task
- Handling the result

## Source Code

```rust
//! Single agent example

use agentic_core::{Agent, AgentConfig, ExecutionMode, TaskPayload};
use agentic_llm::OpenAIAdapter;
use agentic_sandbox::ProcessSandbox;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create LLM adapter
    let llm = OpenAIAdapter::new(
        std::env::var("OPENAI_API_KEY")?,
        "gpt-4o-mini"
    ).with_temperature(0.7);

    // Create sandbox for code execution
    let sandbox = ProcessSandbox::new()
        .with_timeout(30000);

    // Configure the agent
    let config = AgentConfig::builder()
        .name("code-assistant")
        .execution_mode(ExecutionMode::Auto)
        .timeout_ms(60000)
        .system_prompt("You are a helpful coding assistant. When asked to write code, provide working examples.")
        .build();

    // Create and start the agent
    let agent = Agent::new(config, llm, Some(sandbox));
    agent.start().await?;

    println!("Agent started: {}", agent.name());
    println!("State: {:?}", agent.state());

    // Create a task
    let task = TaskPayload::new(
        "Write a Python script that calculates the first 10 Fibonacci numbers and prints them."
    );

    // Run the task
    println!("\nRunning task...\n");
    let result = agent.run(task).await?;

    // Display results
    println!("Status: {:?}", result.status);
    println!("Execution time: {}ms", result.execution_time_ms);

    if let Some(output) = result.data.get("output") {
        println!("\nOutput:\n{}", output);
    }

    if let Some(code) = result.data.get("code") {
        println!("\nGenerated code:\n{}", code);
    }

    // Show metrics
    let metrics = agent.metrics();
    println!("\nAgent Metrics:");
    println!("  Tasks completed: {}", metrics.tasks_completed);
    println!("  Success rate: {:.1}%", metrics.success_rate * 100.0);
    println!("  Total tokens: {}", metrics.total_tokens);

    // Stop the agent
    agent.stop().await?;
    println!("\nAgent stopped.");

    Ok(())
}
```

## Expected Output

```
Agent started: code-assistant
State: Idle

Running task...

Status: Success
Execution time: 1523ms

Output:
0
1
1
2
3
5
8
13
21
34

Generated code:
def fibonacci(n):
    fib = [0, 1]
    for i in range(2, n):
        fib.append(fib[i-1] + fib[i-2])
    return fib[:n]

for num in fibonacci(10):
    print(num)

Agent Metrics:
  Tasks completed: 1
  Success rate: 100.0%
  Total tokens: 245

Agent stopped.
```

## Configuration Options

### Execution Modes

```rust
// Always execute generated code
ExecutionMode::Always

// Never execute, return code only
ExecutionMode::Never

// LLM decides via [EXECUTE]/[NO_EXECUTE] markers
ExecutionMode::Auto

// Based on task type
ExecutionMode::Task
```

### Timeout Settings

```rust
let config = AgentConfig::builder()
    .timeout_ms(60000)      // Overall task timeout
    .max_retries(3)         // Retry on failure
    .build();

let sandbox = ProcessSandbox::new()
    .with_timeout(30000);   // Code execution timeout
```
