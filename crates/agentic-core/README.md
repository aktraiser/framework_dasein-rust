# agentic-core

Core runtime and orchestration for the Agentic Framework.

## Features

- **Supervisor/Executor Pattern**: Distributed agent orchestration
- **Graph-based Workflows**: DAG execution with state management
- **Validation Pipeline**: Multi-stage code validation
- **Async-first**: Built on Tokio

## Which API to use

This crate grew in stages and still carries more than one way of doing the same thing. For new code:

| Need | Use | Earlier alternatives |
|------|-----|----------------------|
| An agent that talks to a model and calls tools | `distributed::graph::agent::ChatAgent` | `Agent` (deprecated): one model call, then runs the first code block of the answer |
| Several steps, branches, retries, resume | `distributed::graph` (`WorkflowBuilder`, `Workflow`) | `Supervisor` / `Executor` / `ExecutorPool`: an in-process pool with rule-based validation, still used by the code-generation pipelines |
| Tasks that survive a restart and are shared between processes | `distributed::bus::DurableQueue` | `Sequencer` (deprecated): in memory only |

The code-generation helpers (`IncrementalPipeline`, `RepairEngine`, `SandboxValidator`, `LiaisonArchitect`, `TaskDecomposer`, `CodeAssembler`) are a separate toolbox built on the `Supervisor` generation. They are not deprecated, but they are not where new orchestration features go.

Deprecated items keep working and only produce a compiler warning. They will be removed in a later release.

Some names exist twice in this crate with different meanings, depending on the module: `Agent`, `AgentError`, `Executor`, `ValidationResult`, `ErrorCategory`, `LogLevel`, `TokenUsage`, `CodeAssembler`. Import them by their full path rather than through `prelude::*` when both are in scope.

The separate crates `dasein-agentic-bus` and `dasein-agentic-orchestrator` are not used by the rest of the SDK; their main types are deprecated in favour of the modules above.

## Installation

```toml
[dependencies]
dasein-agentic-core = { git = "https://github.com/aktraiser/framework_dasein-rust" }
```

## Quick Start

```rust
use dasein_agentic_core::prelude::*;

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
