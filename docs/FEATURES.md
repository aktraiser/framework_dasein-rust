# Cargo Features Guide

Agentic-RS uses Cargo features to enable optional functionality. This keeps the default build fast and lightweight.

## LLM Providers (`agentic-llm`)

Choose your LLM provider(s):

```toml
[dependencies]
# Single provider
dasein-agentic-llm = { path = "crates/agentic-llm", features = ["gemini"] }

# Multiple providers
dasein-agentic-llm = { path = "crates/agentic-llm", features = ["gemini", "openai", "anthropic"] }
```

### Available Features

| Feature | Provider | Environment Variable | Models |
|---------|----------|---------------------|--------|
| `gemini` | Google AI | `GEMINI_API_KEY` | gemini-2.0-flash, gemini-pro |
| `openai` | OpenAI | `OPENAI_API_KEY` | gpt-4, gpt-4-turbo, gpt-3.5-turbo |
| `anthropic` | Anthropic | `ANTHROPIC_API_KEY` | claude-3-opus, claude-3-sonnet |
| `ollama` | Ollama (local) | - | llama2, codellama, mistral |

### Default Feature

With no feature specified, `openai` and `ollama` are enabled. Use `default-features = false` to choose providers yourself.

### Usage Example

```rust
use dasein_agentic_llm::{GeminiAdapter, LLMAdapter};  // Requires "gemini" feature

let adapter = GeminiAdapter::new(api_key, "gemini-2.0-flash");
```

---

## Sandbox Types (`agentic-sandbox`)

Choose your code execution environment:

```toml
[dependencies]
# Default (ProcessSandbox only)
dasein-agentic-sandbox = { path = "crates/agentic-sandbox" }

# With Docker support
dasein-agentic-sandbox = { path = "crates/agentic-sandbox", features = ["docker"] }

# With Firecracker VM support
dasein-agentic-sandbox = { path = "crates/agentic-sandbox", features = ["firecracker"] }
```

### Available Features

| Feature | Sandbox Type | Requirements | Security Level |
|---------|--------------|--------------|----------------|
| (default) | `ProcessSandbox` | None | None: runs on the host, trusted code only |
| `docker` | `DockerSandbox` | Docker installed | Container, no network |
| `firecracker` | `FirecrackerSandbox` | Firecracker + Linux/KVM | MicroVM |
| `gateway`, `remote` | `GatewaySandbox`, `RemoteSandbox` | A running gateway or remote server | As provided by the server |

### Security Comparison

| Feature | Isolation | Speed | Resource Usage |
|---------|-----------|-------|----------------|
| ProcessSandbox | Process only | Fast (~1s) | Low |
| DockerSandbox | Container | Medium (~5s) | Medium |
| FirecrackerSandbox | MicroVM | Slow (~10s) | High |

### Usage Example

```rust
use dasein_agentic_sandbox::ProcessSandbox;  // Always available

let sandbox = ProcessSandbox::new()
    .with_timeout(60_000);

// With Docker feature enabled
#[cfg(feature = "docker")]
{
    use dasein_agentic_sandbox::DockerSandbox;
    let sandbox = DockerSandbox::builder()
        .image("python:3.12-alpine")
        .memory_limit("512m")
        .build();
}
```

---

## Storage Backends (`agentic-storage`)

For persistent storage and vector search:

```toml
[dependencies]
dasein-agentic-storage = { path = "crates/agentic-storage", features = ["qdrant", "redis"] }
```

### Available Features

| Feature | Backend | Use Case |
|---------|---------|----------|
| `redis` (default) | Redis | Key-value state |
| `qdrant` | Qdrant | Vector similarity search |

---

## MCP Integration (`agentic-mcp`)

For Model Context Protocol support:

```toml
[dependencies]
dasein-agentic-mcp = { path = "crates/agentic-mcp" }
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `CONTEXT7_API_KEY` | Enable Context7 documentation fetching |

---

## Core Features (`agentic-core`)

The core crate has no optional features. All distributed components are always available.

---

## Feature Combinations

### Minimal (Development)

```toml
dasein-agentic-core = { path = "crates/agentic-core" }
dasein-agentic-llm = { path = "crates/agentic-llm", features = ["gemini"] }
dasein-agentic-sandbox = { path = "crates/agentic-sandbox" }
```

### Full (Production)

```toml
dasein-agentic-core = { path = "crates/agentic-core" }
dasein-agentic-llm = { path = "crates/agentic-llm", features = ["gemini", "openai", "anthropic"] }
dasein-agentic-sandbox = { path = "crates/agentic-sandbox", features = ["docker"] }
dasein-agentic-storage = { path = "crates/agentic-storage", features = ["qdrant", "redis"] }
dasein-agentic-mcp = { path = "crates/agentic-mcp" }
```

### High Security

```toml
dasein-agentic-sandbox = { path = "crates/agentic-sandbox", features = ["firecracker"] }
```

---

## Troubleshooting

### "unresolved import `dasein_agentic_llm::GeminiAdapter`"

You need to enable the `gemini` feature:

```toml
dasein-agentic-llm = { path = "crates/agentic-llm", features = ["gemini"] }
```

### "cannot find type `DockerSandbox`"

You need to enable the `docker` feature:

```toml
dasein-agentic-sandbox = { path = "crates/agentic-sandbox", features = ["docker"] }
```

### "feature `X` is required but it's not selected"

Check that you've enabled the required feature in your `Cargo.toml`.

---

## Checking Enabled Features

You can use `cfg` attributes to check which features are enabled:

```rust
#[cfg(feature = "gemini")]
fn use_gemini() {
    // Only compiled when "gemini" feature is enabled
}

#[cfg(not(feature = "docker"))]
fn fallback_sandbox() {
    // Use ProcessSandbox when Docker is not available
}
```
