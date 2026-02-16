# agentic-core

Core runtime and orchestration for the Agentic Framework.

## Features

- **Supervisor/Executor Pattern**: Distributed agent orchestration
- **Graph-based Workflows**: DAG execution with state management
- **Validation Pipeline**: Multi-stage code validation
- **Async-first**: Built on Tokio

## Installation

```toml
[dependencies]
agentic-core = "0.1"
```

## Quick Start

```rust
use agentic_core::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a supervisor with executors
    let supervisor = Supervisor::new("code-pipeline")
        .executors(4)
        .llm_openai("gpt-4")
        .build();

    // Run the pipeline
    let result = supervisor.run("Write a Rust function").await?;

    println!("{}", result);
    Ok(())
}
```

## Architecture

```
Supervisor
    |
    +-- Executor 1 (Generator)
    |       |
    |       +-- LLM Adapter
    |
    +-- Executor 2 (Validator)
    |       |
    |       +-- Sandbox
    |
    +-- Executor 3 (Corrector)
            |
            +-- LLM Adapter
```

## Patterns

| Pattern | Description |
|---------|-------------|
| `Supervisor` | Orchestrates multiple executors |
| `Executor` | Single unit of work (LLM call, validation, etc.) |
| `Validator` | Validates executor output |
| `Workflow` | Graph-based multi-step execution |

## License

MIT
