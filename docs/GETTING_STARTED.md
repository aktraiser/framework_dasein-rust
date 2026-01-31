# Getting Started with Agentic-RS

This guide will help you get up and running with Agentic-RS in 5 minutes.

## Pre-flight Checklist

Before you begin, make sure you have:

- [ ] **Rust 1.75+** installed (`rustc --version`)
- [ ] **LLM API Key** - one of:
  - `GEMINI_API_KEY` (recommended for testing)
  - `OPENAI_API_KEY`
  - `ANTHROPIC_API_KEY`
- [ ] **Docker** (optional, for Docker sandbox)
- [ ] **NATS Server** (optional, for distributed features)

## Quick Verification

```bash
# Check Rust version
rustc --version  # Should be >= 1.75.0

# Set your API key
export GEMINI_API_KEY="your-api-key-here"

# Clone and test
git clone https://github.com/your-org/agentic-rs
cd agentic-rs
cargo run --example test_gemini
```

Expected output:
```
Testing Gemini adapter...
Provider: gemini
Model: gemini-2.0-flash

1. Health check...
   ✓ API accessible

2. Generate completion...
   ✓ Response: Four
```

---

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
agentic-core = { path = "crates/agentic-core" }
agentic-llm = { path = "crates/agentic-llm", features = ["gemini"] }
agentic-sandbox = { path = "crates/agentic-sandbox" }
tokio = { version = "1", features = ["full"] }
```

### Feature Flags

Choose your LLM provider:

```toml
# For Gemini (recommended)
agentic-llm = { path = "crates/agentic-llm", features = ["gemini"] }

# For OpenAI
agentic-llm = { path = "crates/agentic-llm", features = ["openai"] }

# For Anthropic (Claude)
agentic-llm = { path = "crates/agentic-llm", features = ["anthropic"] }

# For local models (Ollama)
agentic-llm = { path = "crates/agentic-llm", features = ["ollama"] }

# Multiple providers
agentic-llm = { path = "crates/agentic-llm", features = ["gemini", "openai"] }
```

---

## Your First Agent

### Option 1: Simple Executor (No Supervisor)

For simple tasks, use an Executor directly:

```rust
use agentic_core::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create an executor with Gemini
    let executor = Executor::new("my-executor", "standalone")
        .llm_gemini("gemini-2.0-flash")
        .build();

    // Execute a task
    let result = executor.execute(
        "You are a helpful assistant.",
        "What is the capital of France?"
    ).await?;

    println!("Response: {}", result.content);
    Ok(())
}
```

### Option 2: Supervisor with Executor Pool

For production workloads, use a Supervisor to manage multiple executors:

```rust
use agentic_core::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a supervisor with 4 executors
    let supervisor = Supervisor::new("code-supervisor")
        .domain("code")
        .executors(4)
        .llm_gemini("gemini-2.0-flash")
        .build_async()
        .await;

    println!("Pool size: {}", supervisor.pool_size().await);
    println!("Idle executors: {}", supervisor.idle_count().await);

    Ok(())
}
```

---

## Code Generation with Validation

For generating code that actually compiles:

```rust
use agentic_core::prelude::*;
use agentic_sandbox::ProcessSandbox;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create executor
    let executor = Executor::new("code-gen", "supervisor")
        .llm_gemini("gemini-2.0-flash")
        .build();

    // Create sandbox validator
    let sandbox = ProcessSandbox::new().with_timeout(60_000);
    let validator = SandboxValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/code-gen"))
        .run_tests(true);

    // Generate code
    let task = "Write a Rust function that calculates fibonacci numbers";
    let result = executor.execute(
        "You are an expert Rust developer. Return ONLY valid Rust code.",
        task
    ).await?;

    // Validate the generated code
    let validation = validator.validate_rust_code(&result.content).await?;

    if validation.passed {
        println!("✓ Code compiles and tests pass!");
        println!("{}", result.content);
    } else {
        println!("✗ Validation failed:");
        for error in &validation.errors {
            println!("  - {}", error);
        }
    }

    Ok(())
}
```

---

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `GEMINI_API_KEY` | Yes* | Google AI API key |
| `OPENAI_API_KEY` | Yes* | OpenAI API key |
| `ANTHROPIC_API_KEY` | Yes* | Anthropic API key |
| `FAST_MODEL` | No | Model for simple errors (default: `gemini-2.0-flash`) |
| `SMART_MODEL` | No | Model for complex errors (default: `gemini-2.0-flash`) |
| `CONTEXT7_API_KEY` | No | Enable MCP documentation enrichment |
| `NATS_URL` | No | NATS server URL (default: `nats://localhost:4222`) |

*At least one LLM API key is required.

---

## Examples by Difficulty

### Beginner
| Example | Description | Command |
|---------|-------------|---------|
| `test_gemini` | Test LLM connection | `cargo run --example test_gemini` |
| `single_agent` | Basic agent usage | `cargo run --example single_agent` |

### Intermediate
| Example | Description | Command |
|---------|-------------|---------|
| `grounded_loop` | Code generation with validation | `cargo run --example grounded_loop` |
| `validator_pipeline` | Chained validators | `cargo run --example validator_pipeline` |

### Advanced
| Example | Description | Command |
|---------|-------------|---------|
| `multi_executor` | Progressive Locking pipeline | `cargo run --example multi_executor` |
| `distributed_pipeline` | NATS-based distribution | `cargo run --example distributed_pipeline` |

---

## Common Issues

### "GEMINI_API_KEY environment variable not set"

```bash
export GEMINI_API_KEY="your-api-key-here"
```

### "feature `gemini` is required"

Add the feature flag to your Cargo.toml:
```toml
agentic-llm = { ..., features = ["gemini"] }
```

### "connection refused" (NATS)

NATS is optional. Either:
1. Start NATS: `docker run -p 4222:4222 nats:latest`
2. Or use examples that don't require NATS (most of them)

### Compilation errors with sandbox

Make sure you have write permissions to the workspace directory:
```rust
.workspace(PathBuf::from("/tmp/my-workspace"))
```

---

## Next Steps

1. **Read the Architecture**: [ARCHITECTURE.md](./ARCHITECTURE.md)
2. **Understand Progressive Locking**: [MULTI_EXECUTOR.md](./MULTI_EXECUTOR.md)
3. **Create Custom Agents**: [AGENT_CREATION_GUIDE.md](./AGENT_CREATION_GUIDE.md)
4. **Learn NATS Protocol**: [NATS_PROTOCOL.md](./NATS_PROTOCOL.md)

---

## Getting Help

- **GitHub Issues**: Report bugs and request features
- **Examples**: Check `examples/` directory for working code
- **Documentation**: Run `cargo doc --open` for API docs
