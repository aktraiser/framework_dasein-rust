---
layout: home

hero:
  name: Agentic-RS
  text: Multi-Agent AI Framework
  tagline: High-performance Rust framework for building autonomous AI agents
  image:
    src: /logo.svg
    alt: Agentic-RS
  actions:
    - theme: brand
      text: Get Started
      link: /guide/getting-started
    - theme: alt
      text: View on GitHub
      link: https://github.com/your-org/agentic-rs

features:
  - icon: ⚡
    title: Blazing Fast
    details: Built in Rust with async/await, zero-copy design. ~5ms agent overhead, 1000+ concurrent agents.
  - icon: 🔒
    title: Type Safe
    details: No unsafe code, generic traits, comprehensive error handling with thiserror.
  - icon: 🤖
    title: Multi-LLM Support
    details: OpenAI, Anthropic Claude, Google Gemini, Ollama - unified interface for all providers.
  - icon: 🔧
    title: Sandboxed Execution
    details: Safe code execution with Process, Docker, or Firecracker sandboxes. Memory/CPU limits, network isolation.
  - icon: ✅
    title: Two Validation Strategies
    details: Rule-based Validator for fast heuristics. SandboxValidator for ground-truth code validation with real compilation & tests.
  - icon: 📡
    title: Distributed
    details: NATS message bus for inter-agent communication. Pub/Sub, Request/Reply patterns.
  - icon: 💾
    title: Persistent Storage
    details: Redis for state, Qdrant for vector search. Built-in RAG support.
---

## Quick Start

```bash
# Add to Cargo.toml
cargo add agentic-core agentic-llm agentic-sandbox
```

```rust
use agentic_core::{Agent, AgentConfig};
use agentic_llm::OpenAIAdapter;
use agentic_sandbox::ProcessSandbox;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let llm = OpenAIAdapter::new("sk-...", "gpt-4o");
    let sandbox = ProcessSandbox::new();

    let agent = Agent::new(
        AgentConfig::default(),
        llm,
        Some(sandbox),
    );

    agent.start().await?;

    let result = agent.run(TaskPayload::new("Write hello world")).await?;
    println!("{:?}", result);

    Ok(())
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Your Application                      │
└─────────────────────────────────────────────────────────┘
                            │
┌───────────────────────────▼───────────────────────────┐
│                 agentic-orchestrator                   │
│            Multi-agent coordination & Workflows        │
└───────┬─────────────────┬─────────────────┬───────────┘
        │                 │                 │
┌───────▼───────┐ ┌───────▼───────┐ ┌───────▼───────┐
│  agentic-core │ │  agentic-bus  │ │agentic-storage│
│ Agent Runtime │ │  NATS Pub/Sub │ │ Redis+Qdrant  │
└───────┬───────┘ └───────────────┘ └───────────────┘
        │
┌───────┴───────┐
│               │
▼               ▼
agentic-llm    agentic-sandbox
```

## Supported LLM Providers

| Provider | Models | Status |
|----------|--------|--------|
| OpenAI | GPT-4o, GPT-4o-mini, o1 | ✅ Ready |
| Anthropic | Claude Sonnet, Haiku, Opus | ✅ Ready |
| Google | Gemini 2.0 Flash, 1.5 Pro | ✅ Ready |
| Ollama | Llama, Qwen, Mistral, etc. | ✅ Ready |
