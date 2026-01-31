# What is Agentic-RS?

**Agentic-RS** is a high-performance Rust framework for building autonomous multi-agent AI systems. It provides everything you need to create, coordinate, and deploy AI agents at scale.

## Key Features

### Performance
- **~5ms agent overhead** (excluding LLM latency)
- **~20MB memory** per idle agent
- **1000+ concurrent agents** on modest hardware
- Zero-copy async design with Tokio

### Type Safety
- No `unsafe` code - compiler-verified safety
- Generic traits for extensibility
- Comprehensive error handling

### Multi-Provider LLM Support
- **OpenAI** - GPT-4o, GPT-4o-mini, o1
- **Anthropic** - Claude Sonnet, Haiku, Opus
- **Google** - Gemini 2.0 Flash, 1.5 Pro
- **Ollama** - Local models (Llama, Qwen, Mistral)

### Sandboxed Execution
- **ProcessSandbox** - Fast local execution for development
- **DockerSandbox** - Production-ready with full isolation

### Distributed Architecture
- NATS message bus for agent communication
- Redis for state persistence
- Qdrant for vector search / RAG

## Crate Structure

| Crate | Description |
|-------|-------------|
| `agentic-core` | Core agent runtime, types, and protocol |
| `agentic-llm` | LLM adapters (OpenAI, Anthropic, Gemini, Ollama) |
| `agentic-sandbox` | Sandboxed code execution |
| `agentic-bus` | NATS message bus integration |
| `agentic-storage` | Redis + Qdrant storage backends |
| `agentic-orchestrator` | Multi-agent coordination and workflows |
| `agentic-mcp` | Model Context Protocol client |

## When to Use Agentic-RS

✅ **Good fit:**
- Production AI agent systems requiring reliability
- Multi-agent workflows with coordination
- Systems needing sandboxed code execution
- High-throughput agent deployments

❌ **Consider alternatives:**
- Quick prototypes (Python may be faster to iterate)
- Simple single-shot LLM calls (use API directly)
- When your team isn't familiar with Rust
