# Cargo Features Guide

Agentic-RS uses Cargo features to enable optional functionality. This keeps the default build fast and lightweight.

## LLM Providers (`agentic-llm`)

Choose your LLM provider(s):

```toml
[dependencies]
# Single provider
agentic-llm = { path = "crates/agentic-llm", features = ["gemini"] }

# Multiple providers
agentic-llm = { path = "crates/agentic-llm", features = ["gemini", "openai", "anthropic"] }
```

### Available Features

| Feature | Provider | Environment Variable | Models |
|---------|----------|---------------------|--------|
| `gemini` | Google AI | `GEMINI_API_KEY` | gemini-2.0-flash, gemini-pro |
| `openai` | OpenAI | `OPENAI_API_KEY` | gpt-4, gpt-4-turbo, gpt-3.5-turbo |
| `anthropic` | Anthropic | `ANTHROPIC_API_KEY` | claude-3-opus, claude-3-sonnet |
| `ollama` | Ollama (local) | - | llama2, codellama, mistral |

### Default Feature

If no feature is specified, **no LLM provider is available**. You must explicitly enable at least one.

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
agentic-sandbox = { path = "crates/agentic-sandbox" }

# With Docker support
agentic-sandbox = { path = "crates/agentic-sandbox", features = ["docker"] }

# With Firecracker VM support
agentic-sandbox = { path = "crates/agentic-sandbox", features = ["firecracker"] }
```

### Available Features

| Feature | Sandbox Type | Requirements | Security Level |
|---------|--------------|--------------|----------------|
| (default) | `ProcessSandbox` | None | Low |
| `docker` | `DockerSandbox` | Docker installed | Medium |
| `firecracker` | `FirecrackerSandbox` | Firecracker + KVM | High |

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
    let sandbox = DockerSandbox::new("rust:latest")
        .with_memory_limit(512 * 1024 * 1024);
}
```

---

## Storage Backends (`agentic-storage`)

For persistent storage and vector search:

```toml
[dependencies]
agentic-storage = { path = "crates/agentic-storage", features = ["qdrant", "redis"] }
```

### Available Features

| Feature | Backend | Use Case |
|---------|---------|----------|
| `qdrant` | Qdrant | Vector similarity search |
| `redis` | Redis | Key-value caching |
| `memory` | In-memory | Testing, development |

---

## MCP Integration (`agentic-mcp`)

For Model Context Protocol support:

```toml
[dependencies]
agentic-mcp = { path = "crates/agentic-mcp" }
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
agentic-core = { path = "crates/agentic-core" }
agentic-llm = { path = "crates/agentic-llm", features = ["gemini"] }
agentic-sandbox = { path = "crates/agentic-sandbox" }
```

### Full (Production)

```toml
agentic-core = { path = "crates/agentic-core" }
agentic-llm = { path = "crates/agentic-llm", features = ["gemini", "openai", "anthropic"] }
agentic-sandbox = { path = "crates/agentic-sandbox", features = ["docker"] }
agentic-storage = { path = "crates/agentic-storage", features = ["qdrant", "redis"] }
agentic-mcp = { path = "crates/agentic-mcp" }
```

### High Security

```toml
agentic-sandbox = { path = "crates/agentic-sandbox", features = ["firecracker"] }
```

---

## Troubleshooting

### "unresolved import `dasein_agentic_llm::GeminiAdapter`"

You need to enable the `gemini` feature:

```toml
agentic-llm = { path = "crates/agentic-llm", features = ["gemini"] }
```

### "cannot find type `DockerSandbox`"

You need to enable the `docker` feature:

```toml
agentic-sandbox = { path = "crates/agentic-sandbox", features = ["docker"] }
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
