# Architecture

## Overview

Agentic-RS follows a layered, modular architecture where each crate handles a specific concern.

```
┌─────────────────────────────────────────────────────────────────┐
│                      APPLICATION LAYER                          │
│                 (Your agents and workflows)                     │
└─────────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────▼─────────────────────────────────┐
│                    ORCHESTRATION LAYER                         │
│       agentic-orchestrator: Agent registry, workflows          │
└───────────┬─────────────────┬─────────────────┬───────────────┘
            │                 │                 │
┌───────────▼───────────┐ ┌───▼───────────┐ ┌───▼───────────────┐
│     AGENT LAYER       │ │  MESSAGE BUS  │ │   STORAGE LAYER   │
│  ┌─────────────────┐  │ │               │ │ ┌───────────────┐ │
│  │ Agent Runtime   │  │ │  agentic-bus  │ │ │ agentic-      │ │
│  │ (agentic-core)  │  │ │  (NATS)       │ │ │ storage       │ │
│  └────────┬────────┘  │ │               │ │ │ Redis+Qdrant  │ │
│           │           │ │  Pub/Sub      │ │ └───────────────┘ │
│  ┌────────┴────────┐  │ │  Req/Reply    │ │                   │
│  │ LLM    Sandbox  │  │ │  Queue Groups │ │                   │
│  └────────┴────────┘  │ │               │ │                   │
└───────────────────────┘ └───────────────┘ └───────────────────┘
```

## Core Components

### Agent (`agentic-core`)

The `Agent<L, S>` struct is the fundamental building block:

```rust
pub struct Agent<L: LLMAdapter, S: Sandbox> {
    config: AgentConfig,
    llm: L,
    sandbox: Option<S>,
    state: Arc<RwLock<AgentState>>,
    metrics: Arc<RwLock<AgentMetrics>>,
}
```

**Generic parameters:**
- `L: LLMAdapter` - Any LLM provider
- `S: Sandbox` - Any execution environment

### Agent Lifecycle

```
┌─────────────┐
│   Created   │
└──────┬──────┘
       │ start()
       ▼
┌─────────────┐
│    Idle     │◄──────────┐
└──────┬──────┘           │
       │ run(task)        │
       ▼                  │
┌─────────────┐           │
│   Running   │───────────┘
└──────┬──────┘  complete
       │
       ▼
┌─────────────┐
│   Stopped   │
└─────────────┘
```

### Execution Flow

When `agent.run(task)` is called:

```
1. THINK Phase
   └─ Build messages with system prompt + task
   └─ Call LLM.generate()

2. EXTRACT Phase
   └─ Parse markdown code blocks from response
   └─ Identify language and code

3. DECIDE Phase
   └─ Check ExecutionMode: Always/Never/Auto/Task
   └─ Determine if code should execute

4. EXECUTE Phase (if applicable)
   └─ Call Sandbox.execute(code)
   └─ Capture stdout, stderr, artifacts

5. RESPOND Phase
   └─ Package results in ResultPayload
   └─ Update metrics and state
```

### Execution Modes

| Mode | Behavior |
|------|----------|
| `Always` | Execute all generated code |
| `Never` | Return code without execution |
| `Auto` | LLM decides via `[EXECUTE]`/`[NO_EXECUTE]` markers |
| `Task` | Based on action type (default: safe) |

## Message Protocol

All inter-agent communication uses a standardized protocol:

```rust
pub struct Message {
    pub metadata: Metadata,      // id, timestamp, version
    pub routing: RoutingInfo,    // from, to, correlation_id
    pub content: MessageContent, // Task/Result/Event/Error
    pub control: ControlSettings, // priority, ttl, task_id
}
```

### Message Types

```rust
pub enum MessageContent {
    Task { payload: TaskPayload },
    Result { payload: ResultPayload },
    Event { event_type: String, data: Value },
    Error { code: String, message: String, recoverable: bool },
}
```

## NATS Subject Patterns

```
agents.{agent_id}.inbox     # Receive tasks
agents.{agent_id}.outbox    # Send results
agents.{agent_id}.events    # Lifecycle events
orchestrator.inbox          # Central coordination
system.events               # System-wide notifications
```

## Storage Architecture

### State Store (Redis)

Key-value storage for agent state:

```rust
// Store agent state
state_store.set("agent:123:state", &state, Some(3600000)).await?;

// Retrieve
let state: AgentState = state_store.get("agent:123:state").await?.unwrap();
```

### Vector Store (Qdrant)

For embeddings and similarity search:

```rust
// Create collection
vector_store.create_collection("memories", 1536).await?;

// Upsert vectors
vector_store.upsert("memories", points).await?;

// Search similar
let results = vector_store.search("memories", query_vector, 10, Some(0.7)).await?;
```

## Extensibility

Agentic-RS is designed for extensibility through traits:

- **`LLMAdapter`** - Add new LLM providers
- **`Sandbox`** - Add new execution environments
- **`StateStore`** - Add new state backends
- **`VectorStore`** - Add new vector databases
- **`MessageBus`** - Add new message transports
