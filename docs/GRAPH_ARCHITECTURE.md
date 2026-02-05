# Architecture Graph Multi-Agent

> **Vision**: Passer d'une architecture linéaire (Executor → Validator → Retry) à une architecture **graph dynamique** avec **Executors** (noeuds) et **Edges** (connexions).

---

## 📊 STATUS TRACKER (2025-02-04)

### Implémenté ✅

| PR | Feature | Fichier(s) | Status |
|----|---------|------------|--------|
| **PR #1** | Executor Trait | `graph/executor.rs` | ✅ Complet |
| **PR #2** | WorkflowContext | `graph/context.rs` | ✅ Complet |
| **PR #3** | 5 Edge Types | `graph/edge.rs` | ✅ Complet |
| **PR #4** | WorkflowBuilder | `graph/builder.rs` | ✅ Complet |
| **PR #5** | Superstep Execution | `graph/superstep.rs`, `graph/workflow.rs` | ✅ Complet |
| **PR #6** | Graph Persistence | `graph/persistence.rs` | ✅ Complet |

### En cours 🚧

| Feature | Status | Notes |
|---------|--------|-------|
| `Workflow.run_with_resume()` | 🚧 PR #6 | Resume depuis checkpoint |
| `SharedValidatorPipeline` | 🚧 PR #6 | Arc wrapper pour stateless sharing |

### Implémenté récemment ✅

| Feature | PR/Phase | Notes |
|---------|----------|-------|
| **Agent Trait** (`run`, `run_stream`, `tools`) | Phase 6 | `distributed/graph/agent/trait_def.rs` |
| **AgentThread** (in-memory) | Phase 6 | NATS KV déféré Phase 8 |
| **ChatAgent** | Phase 6 | `distributed/graph/agent/chat_agent.rs` |
| **WorkflowAgent** | Phase 6 | `distributed/graph/agent/workflow_agent.rs` |
| **`workflow.as_agent()`** | Phase 6 | `WorkflowAsAgent` trait |
| **SequentialBuilder** | Phase 7 | `patterns/sequential.rs` |
| **ConcurrentBuilder** | Phase 7 | `patterns/concurrent.rs` |
| **GroupChatBuilder** | Phase 7 | `patterns/group_chat.rs` + selectors |
| **HandoffBuilder** | Phase 7 | `patterns/handoff.rs` + `HandoffCapable` |
| **MemoryProvider** | Phase 8 | `agent/memory.rs` - trait + InMemoryProvider |
| **ChatReducer** | Phase 8 | `agent/reducer.rs` - 4 implémentations |
| **Memory types** | Phase 8 | `Memory`, `MemoryCategory`, `MemoryContext` |

### Non implémenté ❌

| Feature | Priorité | Effort | Section doc |
|---------|----------|--------|-------------|
| **AgentThread NATS KV** (persisté) | P1 | 0.5 sem | [Thread](#thread-conversation-persistée-sur-nats) |
| **NatsMemoryProvider** | P1 | 0.5 sem | [Agent Memory](#agent-memory-court-terme-et-long-terme) |
| **ContinuationToken** | P2 | 1 sem | [Background Responses](#background-responses-tâches-longues-avec-continuation) |
| **MagenticPlanner** | P3 | 2 sem | [Magentic](#pattern-magentic-planner-based) |
| **llm_selector** | P2 | 0.5 sem | LLM-based speaker selection |

### Architecture actuelle vs cible

```
ACTUEL (v0.3.0):                          CIBLE (v1.0):
┌─────────────────────────────┐           ┌─────────────────────────────┐
│         AGENT LAYER         │           │         AGENT LAYER         │
│  ✅ trait Agent             │           │  ✅ trait Agent + ChatAgent │
│  ✅ ChatAgent, WorkflowAgent│  ──────▶  │  + WorkflowAgent + Thread   │
│  ✅ Memory (in-memory)      │           │  + Memory + Background      │
│  ✅ ChatReducer (4 types)   │           │  + NatsMemoryProvider       │
└─────────────────────────────┘           └─────────────────────────────┘
              │                                         │
              ▼                                         ▼
┌─────────────────────────────┐           ┌─────────────────────────────┐
│       WORKFLOW LAYER        │           │       WORKFLOW LAYER        │
│  ✅ Executor trait          │           │  ✅ Executor trait          │
│  ✅ 5 Edge types            │           │  ✅ 5 Edge types            │
│  ✅ Superstep execution     │           │  ✅ Superstep execution     │
│  ✅ WorkflowBuilder         │           │  ✅ WorkflowBuilder         │
│  ✅ Persistence (Redis)     │           │  ✅ Persistence (Redis)     │
│  ✅ Orchestration Patterns  │           │  ✅ Sequential, Concurrent  │
│    Sequential, Concurrent   │           │  ✅ GroupChat, Handoff      │
│    GroupChat, Handoff       │           │  ⏳ MagenticPlanner         │
└─────────────────────────────┘           └─────────────────────────────┘
```

---

## TL;DR

| Concept | Description | Inspiré de |
|---------|-------------|------------|
| **Agent** | Interface haut-niveau: gère conversation, invoque workflows | [MAF Agents](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/agents/agent-types/) |
| **Executor** | Noeud générique: Worker, Validator, ou Orchestrator | [MAF Executors](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/core-concepts/executors) |
| **Edge** | Connexion typée: Direct, Conditional, Switch, FanOut, FanIn | [MAF Edges](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/core-concepts/edges) |
| **Workflow** | Orchestration via Supersteps (Bulk Synchronous Parallel) | [Google Pregel](https://research.google/pubs/pub37252/) |
| **NATS** | Mémoire, historique, events (notre ajout) | - |

```
┌─────────────────────────────────────────────────────────────────┐
│                        NOTRE ARCHITECTURE                        │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                      AGENT LAYER                          │  │
│  │  User ←→ Agent (conversation, tools, streaming)          │  │
│  └───────────────────────┬───────────────────────────────────┘  │
│                          │ invoke workflow                       │
│  ┌───────────────────────▼───────────────────────────────────┐  │
│  │                    WORKFLOW LAYER                         │  │
│  │  Executor ─── Edge ─── Executor ─── Edge ─── Executor     │  │
│  │  (Worker)    (Direct)  (Validator) (Cond)   (Worker)      │  │
│  └───────────────────────┬───────────────────────────────────┘  │
│                          │                                       │
│  ┌───────────────────────▼───────────────────────────────────┐  │
│  │                      NATS LAYER                           │  │
│  │              Events + State + Audit + History             │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Concepts Fondamentaux

### Graph = Executors + Edges

```
     ┌──────────────────────────────────────────────────────────────┐
     │                         TASK GRAPH                           │
     │                                                              │
     │    ┌─────────┐         ┌─────────┐         ┌─────────┐      │
     │    │Executor │───edge──│Executor │───edge──│Executor │      │
     │    │ (Worker)│         │(Orchestr)│        │(Validator)│    │
     │    └─────────┘         └────┬─────┘         └─────────┘      │
     │                             │                                │
     │                    ┌────────┴────────┐                       │
     │                    │   SUB-GRAPH     │                       │
     │                    │ ┌────┐  ┌────┐  │                       │
     │                    │ │Work│──│Work│  │                       │
     │                    │ └────┘  └────┘  │                       │
     │                    └─────────────────┘                       │
     └──────────────────────────────────────────────────────────────┘
```

| Concept | Description |
|---------|-------------|
| **Agent** | Interface utilisateur. Gère conversation, outils, streaming |
| **Executor** | Noeud du graph. Abstraction générique (Worker, Validator, ou Orchestrator) |
| **Edge** | Connexion entre executors. Transporte data + metadata + conditions |
| **Graph** | L'ensemble. Peut être récursif (sub-graphs) |

---

## Agent Layer: L'Interface Utilisateur

> ✅ **STATUT: IMPLÉMENTÉ (Phase 6)** - `distributed/graph/agent/` module.
> Le `trait Agent` avec `run()` et `run_stream()` est implémenté.
> `ChatAgent`, `WorkflowAgent`, `AgentThread` (in-memory), et `Tool` sont disponibles.

Inspiré de [Microsoft Agent Framework - Agent Types](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/agents/agent-types/).

### Agents vs Executors

| Aspect | Agent | Executor |
|--------|-------|----------|
| **Niveau** | Haut (interface utilisateur) | Bas (noeud du workflow) |
| **Rôle** | Gère une conversation multi-tours | Fait une unité de travail atomique |
| **Lifecycle** | `run()`, `run_stream()` | `handle(input, ctx)` |
| **État** | Thread de conversation | State dans le graph |
| **Outils** | Functions, Code Interpreter, MCP | N/A (l'executor EST l'outil) |

### Les 3 Types d'Agents

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            AGENT TYPES                                      │
│                                                                             │
│  1. ChatAgent (Simple)                                                      │
│     ├─ Wrapper autour d'un service LLM (Anthropic, OpenAI, Gemini...)      │
│     ├─ Supporte: function tools, streaming, structured output              │
│     └─ Usage: conversations simples, Q&A                                   │
│                                                                             │
│  2. WorkflowAgent (Custom)                                                  │
│     ├─ Agent qui invoque un Workflow pour les tâches complexes             │
│     ├─ Délègue la génération de code aux Executors                         │
│     └─ Usage: génération de code, tâches multi-étapes                      │
│                                                                             │
│  3. ProxyAgent (Remote)                                                     │
│     ├─ Proxy pour agents distants (protocole A2A)                          │
│     ├─ Communication inter-agents via NATS                                  │
│     └─ Usage: agents distribués, collaboration                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Trait Agent

```rust
/// Agent de haut niveau (interface utilisateur)
/// Inspiré de MAF BaseAgent
#[async_trait]
pub trait Agent: Send + Sync {
    /// Identifiant unique
    fn id(&self) -> &AgentId;

    /// Exécute une conversation (bloquant)
    async fn run(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError>;

    /// Exécute une conversation (streaming)
    fn run_stream(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Pin<Box<dyn Stream<Item = AgentChunk> + Send>>;

    /// Outils disponibles pour cet agent
    fn tools(&self) -> &[Tool];
}

/// Thread de conversation (persisté sur NATS)
pub struct AgentThread {
    pub id: ThreadId,
    pub messages: Vec<ChatMessage>,
    pub metadata: ThreadMetadata,
}

/// Réponse d'un agent
pub struct AgentResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
}

/// Chunk pour streaming
pub struct AgentChunk {
    pub text: Option<String>,
    pub tool_call: Option<ToolCallChunk>,
    pub done: bool,
}
```

### ChatAgent: Agent Simple

```rust
/// Agent simple basé sur un LLM
pub struct ChatAgent {
    id: AgentId,
    llm: Arc<dyn LLMAdapter>,
    instructions: String,
    tools: Vec<Tool>,
    nats: Arc<NatsClient>,
}

#[async_trait]
impl Agent for ChatAgent {
    fn id(&self) -> &AgentId { &self.id }

    async fn run(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError> {
        // 1. Ajouter les messages au thread
        thread.messages.extend(messages);

        // 2. Persister le thread sur NATS
        self.nats.kv_put(&format!("thread.{}", thread.id), thread).await?;

        // 3. Appeler le LLM avec les outils
        let response = self.llm.chat(
            &self.instructions,
            &thread.messages,
            &self.tools,
        ).await?;

        // 4. Gérer les tool calls si nécessaire
        if !response.tool_calls.is_empty() {
            let tool_results = self.execute_tools(&response.tool_calls).await?;
            // Recursive call avec les résultats
            return self.run(tool_results.into_messages(), thread).await;
        }

        // 5. Ajouter la réponse au thread
        thread.messages.push(ChatMessage::assistant(&response.text));
        self.nats.kv_put(&format!("thread.{}", thread.id), thread).await?;

        Ok(response)
    }

    fn run_stream(&self, messages: Vec<ChatMessage>, thread: &mut AgentThread)
        -> Pin<Box<dyn Stream<Item = AgentChunk> + Send>>
    {
        // Streaming implementation...
    }

    fn tools(&self) -> &[Tool] { &self.tools }
}
```

### WorkflowAgent: Agent + Workflow

```rust
/// Agent qui délègue à un Workflow pour les tâches complexes
pub struct WorkflowAgent {
    id: AgentId,
    llm: Arc<dyn LLMAdapter>,
    workflow: Workflow,
    nats: Arc<NatsClient>,
}

impl WorkflowAgent {
    /// Détecte si la tâche nécessite un workflow
    fn needs_workflow(&self, messages: &[ChatMessage]) -> bool {
        // Heuristiques: "génère", "crée", "implémente", etc.
        let last = messages.last().map(|m| m.content.to_lowercase());
        last.map(|c| {
            c.contains("génère") || c.contains("crée") ||
            c.contains("implémente") || c.contains("code")
        }).unwrap_or(false)
    }
}

#[async_trait]
impl Agent for WorkflowAgent {
    async fn run(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError> {
        thread.messages.extend(messages.clone());

        if self.needs_workflow(&thread.messages) {
            // Déléguer au Workflow
            let task = self.extract_task(&thread.messages)?;

            // Publier sur NATS: workflow started
            self.nats.publish("agentic.agent.workflow.started", &task).await?;

            // Exécuter le workflow (Executors + Edges)
            let result = self.workflow.run(task).await?;

            // Formater la réponse
            let response = AgentResponse {
                text: self.format_workflow_result(&result),
                tool_calls: vec![],
                usage: result.total_usage(),
            };

            thread.messages.push(ChatMessage::assistant(&response.text));
            return Ok(response);
        }

        // Sinon, simple chat
        let response = self.llm.chat(
            "You are a helpful coding assistant.",
            &thread.messages,
            &[],
        ).await?;

        thread.messages.push(ChatMessage::assistant(&response.text));
        Ok(response)
    }

    // ...
}
```

### Workflow comme Agent: `workflow.as_agent()`

Inspiré de [MAF Workflows as Agents](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/workflows/as-agents), on peut **convertir un workflow en agent** pour l'exposer via l'API Agent standard.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    WORKFLOW AS AGENT                                        │
│                                                                             │
│  workflow.as_agent("Content Pipeline")                                      │
│                    │                                                        │
│                    ▼                                                        │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    WorkflowAgent (wrapper)                           │   │
│  │                                                                      │   │
│  │  impl Agent for WorkflowAgent {                                     │   │
│  │      fn run() → délègue à workflow.run()                            │   │
│  │      fn run_stream() → délègue à workflow.run_stream()              │   │
│  │  }                                                                   │   │
│  │                                                                      │   │
│  │  ┌───────────────────────────────────────────────────────────────┐  │   │
│  │  │                    Workflow interne                            │  │   │
│  │  │  Researcher → Writer → Reviewer                               │  │   │
│  │  └───────────────────────────────────────────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                    │                                                        │
│                    ▼                                                        │
│  Utilisable comme n'importe quel Agent:                                     │
│  - Dans un autre workflow                                                   │
│  - Comme outil d'un autre agent                                            │
│  - Via une API REST                                                         │
└─────────────────────────────────────────────────────────────────────────────┘
```

#### API `as_agent()`

```rust
impl Workflow {
    /// Convertit ce workflow en Agent
    pub fn as_agent(self, name: &str) -> WorkflowAsAgent {
        WorkflowAsAgent {
            id: AgentId::new(name),
            workflow: self,
        }
    }
}

/// Wrapper qui expose un Workflow comme un Agent
pub struct WorkflowAsAgent {
    id: AgentId,
    workflow: Workflow,
}

#[async_trait]
impl Agent for WorkflowAsAgent {
    fn id(&self) -> &AgentId { &self.id }

    async fn run(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError> {
        // Extraire le dernier message comme input
        let input = messages.last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        // Exécuter le workflow
        let result = self.workflow.run(input).await?;

        // Convertir en AgentResponse
        Ok(AgentResponse {
            text: result.get_output::<String>().unwrap_or_default(),
            messages: result.to_chat_messages(),
            ..Default::default()
        })
    }

    fn run_stream(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Pin<Box<dyn Stream<Item = AgentChunk> + Send>> {
        let input = messages.last().map(|m| m.content.clone()).unwrap_or_default();

        Box::pin(async_stream::stream! {
            let mut stream = self.workflow.run_stream(input);

            while let Some(event) = stream.next().await {
                match event {
                    WorkflowEvent::AgentResponseUpdate { chunk, executor_id } => {
                        yield AgentChunk {
                            text: Some(chunk),
                            author_name: Some(executor_id.to_string()),
                            ..Default::default()
                        };
                    }
                    WorkflowEvent::Output { data } => {
                        yield AgentChunk {
                            text: Some(serde_json::to_string(&data).unwrap()),
                            done: true,
                            ..Default::default()
                        };
                    }
                    _ => {}
                }
            }
        })
    }
}
```

#### Usage

```rust
// Créer un workflow complexe
let workflow = SequentialBuilder::new()
    .participants(vec![researcher, writer, reviewer])
    .build();

// Convertir en agent
let content_agent = workflow.as_agent("Content Pipeline");

// Utiliser comme n'importe quel agent
let thread = content_agent.get_new_thread();
let response = content_agent.run(
    vec![ChatMessage::user("Write about quantum computing")],
    &mut thread
).await?;

// Ou en streaming
async for chunk in content_agent.run_stream(messages, &mut thread) {
    print!("{}", chunk.text.unwrap_or_default());
}
```

#### Composition: Workflow d'agents de workflow

```rust
// Plusieurs workflows convertis en agents
let research_workflow = research_pipeline.as_agent("Research");
let writing_workflow = writing_pipeline.as_agent("Writing");
let review_workflow = review_pipeline.as_agent("Review");

// Orchestrer ces agents de workflow ensemble
let meta_workflow = SequentialBuilder::new()
    .participants(vec![
        research_workflow,   // C'est un workflow!
        writing_workflow,    // C'est aussi un workflow!
        review_workflow,     // Encore un workflow!
    ])
    .build();

// Le meta-workflow peut aussi être converti en agent
let super_agent = meta_workflow.as_agent("Super Content Pipeline");
```

### Articulation Agent ↔ Workflow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           FLOW COMPLET                                      │
│                                                                             │
│  User: "Crée une state machine TypeScript"                                 │
│                          │                                                  │
│                          ▼                                                  │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ WorkflowAgent                                                        │   │
│  │   1. Reçoit le message                                              │   │
│  │   2. Détecte: needs_workflow() = true                               │   │
│  │   3. Extrait la tâche                                               │   │
│  │   4. Invoque self.workflow.run(task)                                │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│                                 ▼                                           │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Workflow (Supersteps)                                                │   │
│  │                                                                      │   │
│  │   Superstep 0: TypeGen, TestGen (parallel)                          │   │
│  │        │                                                             │   │
│  │   Superstep 1: ImplGen                                              │   │
│  │        │                                                             │   │
│  │   Superstep 2: Assembler                                            │   │
│  │        │                                                             │   │
│  │   Superstep 3: CompileValidator → (success) → TestValidator         │   │
│  │                      │                              │                │   │
│  │               (failure)                      (failure)               │   │
│  │                      └──────────────────────────────┘                │   │
│  │                                 │                                    │   │
│  │                                 ▼                                    │   │
│  │                          ImplGen (retry avec feedback)              │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│                                 ▼                                           │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ NATS                                                                 │   │
│  │   - Thread persisté                                                  │   │
│  │   - Workflow events                                                  │   │
│  │   - Executor outputs                                                 │   │
│  │   - Historique pour retry                                           │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                 │                                           │
│                                 ▼                                           │
│  User: reçoit le code généré + tests                                       │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Agents wrappés dans Executors

Inspiré de [MAF Using Agents](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/workflows/using-agents), les **Agents sont automatiquement wrappés dans des Executors** pour être utilisés dans les workflows.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    AGENT WRAPPING                                           │
│                                                                             │
│  llm.as_agent("instructions", "name")                                       │
│                    │                                                        │
│                    ▼                                                        │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    AgentExecutor (auto-généré)                       │   │
│  │  ┌───────────────────────────────────────────────────────────────┐  │   │
│  │  │                         Agent                                  │  │   │
│  │  │  - instructions                                               │  │   │
│  │  │  - name                                                       │  │   │
│  │  │  - llm adapter                                                │  │   │
│  │  └───────────────────────────────────────────────────────────────┘  │   │
│  │                                                                      │   │
│  │  impl Executor for AgentExecutor {                                  │   │
│  │      type Input = ChatMessage | Vec<ChatMessage> | String;          │   │
│  │      type Message = AgentExecutorResponse;                          │   │
│  │  }                                                                   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                    │                                                        │
│                    ▼                                                        │
│            Utilisable dans WorkflowBuilder                                  │
└─────────────────────────────────────────────────────────────────────────────┘
```

#### API simple: `as_agent()`

```rust
// Créer un agent directement utilisable dans un workflow
let writer = llm.as_agent(
    "You are an excellent content writer.",
    "writer"
);

let reviewer = llm.as_agent(
    "You are an excellent content reviewer.",
    "reviewer"
);

// Utiliser directement dans le workflow (wrapping automatique)
let workflow = WorkflowBuilder::new()
    .set_start_executor(writer)   // Agent auto-wrappé en Executor
    .add_edge(&writer.id(), &reviewer.id())
    .build();
```

#### AgentExecutorResponse

```rust
/// Réponse d'un agent wrappé dans un executor
#[derive(Serialize, Deserialize)]
pub struct AgentExecutorResponse {
    /// ID de l'executor qui a produit la réponse
    pub executor_id: ExecutorId,
    /// Réponse de l'agent
    pub agent_response: AgentResponse,
    /// Historique complet de la conversation jusqu'ici
    pub full_conversation: Vec<ChatMessage>,
}
```

#### Executor custom avec Agent

Pour plus de contrôle, créer un executor custom qui contient un agent:

```rust
/// Executor personnalisé wrappant un agent
pub struct WriterExecutor {
    id: ExecutorId,
    agent: ChatAgent,
}

impl WriterExecutor {
    pub fn new(llm: Arc<dyn LLMAdapter>) -> Self {
        Self {
            id: ExecutorId::new("writer"),
            agent: ChatAgent::new(
                llm,
                "You are an excellent content writer.",
            ),
        }
    }
}

#[async_trait]
impl Executor for WriterExecutor {
    type Input = ChatMessage;
    type Message = Vec<ChatMessage>;
    type Output = Never;

    fn id(&self) -> &ExecutorId { &self.id }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Worker }

    async fn handle(
        &self,
        message: ChatMessage,
        ctx: &mut WorkflowContext<Vec<ChatMessage>>,
    ) -> Result<(), ExecutorError> {
        // Appeler l'agent
        let response = self.agent.run(vec![message.clone()]).await?;

        // Construire la conversation complète
        let mut conversation = vec![message];
        conversation.extend(response.messages);

        // Passer au prochain executor
        ctx.send_message(conversation).await?;

        Ok(())
    }
}
```

#### Streaming avec Agents

```rust
// Les agents émettent des événements de streaming
async for event in workflow.run_stream("Write a blog post about AI").await {
    match event {
        WorkflowEvent::AgentResponseUpdate { executor_id, chunk } => {
            // Chunk de réponse en streaming
            print!("{}", chunk);
        }
        WorkflowEvent::AgentRunCompleted { executor_id, response } => {
            // Réponse complète
            println!("\n[{}] Done", executor_id);
        }
        _ => {}
    }
}
```

### Thread: Conversation Persistée sur NATS

> ✅ **STATUT: PARTIELLEMENT IMPLÉMENTÉ** - `AgentThread` in-memory implémenté en Phase 6. Persistance NATS KV planifiée pour Phase 8.

> Inspiré de [MAF Multi-turn Conversation](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/agents/multi-turn-conversation)

#### Principe Clé: Agents Stateless, Threads Stateful

Les agents sont **sans état** - ils peuvent gérer plusieurs conversations simultanées.
C'est le **Thread** qui porte l'état de la conversation.

```
┌─────────────────────────────────────────────────────────────────┐
│                        UN AGENT (stateless)                     │
│                              │                                  │
│         ┌────────────────────┼────────────────────┐             │
│         │                    │                    │             │
│         ▼                    ▼                    ▼             │
│   ┌──────────┐        ┌──────────┐        ┌──────────┐          │
│   │ Thread A │        │ Thread B │        │ Thread C │          │
│   │ User 1   │        │ User 2   │        │ User 1   │          │
│   │ [msg1,   │        │ [msg1,   │        │ [msg1]   │          │
│   │  msg2,   │        │  msg2]   │        │          │          │
│   │  msg3]   │        │          │        │          │          │
│   └──────────┘        └──────────┘        └──────────┘          │
│                                                                 │
│   Chaque thread = conversation isolée, persistée sur NATS KV    │
└─────────────────────────────────────────────────────────────────┘
```

#### API Thread

```rust
impl AgentThread {
    /// Crée un nouveau thread
    pub async fn new(nats: &NatsClient) -> Result<Self, ThreadError> {
        let thread = Self {
            id: ThreadId::new(),
            messages: vec![],
            metadata: ThreadMetadata::default(),
        };
        nats.kv_put(&format!("thread.{}", thread.id), &thread).await?;
        Ok(thread)
    }

    /// Charge un thread existant depuis NATS
    pub async fn load(id: &ThreadId, nats: &NatsClient) -> Result<Self, ThreadError> {
        nats.kv_get(&format!("thread.{}", id)).await?
            .ok_or(ThreadError::NotFound(id.clone()))
    }

    /// Sauvegarde le thread sur NATS
    pub async fn save(&self, nats: &NatsClient) -> Result<(), ThreadError> {
        nats.kv_put(&format!("thread.{}", self.id), self).await?;
        Ok(())
    }
}
```

#### Sérialisation / Désérialisation (Persistence)

Le thread peut être sérialisé pour pause/reprise de conversation:

```rust
impl Agent {
    /// Crée un nouveau thread pour cet agent
    pub fn new_thread(&self) -> AgentThread {
        AgentThread::new_with_agent_id(self.id.clone())
    }
}

impl AgentThread {
    /// Sérialise le thread (pour stockage externe, pause, transfert)
    pub fn serialize(&self) -> Result<Vec<u8>, SerializeError> {
        // Utilise serde pour serialiser tout l'état
        bincode::serialize(self)
    }

    /// Désérialise un thread (reprise de conversation)
    pub fn deserialize(data: &[u8]) -> Result<Self, DeserializeError> {
        bincode::deserialize(data)
    }
}

// Exemple: Pause/Reprise d'une conversation
async fn pause_resume_example(agent: &ChatAgent, nats: &NatsClient) {
    // 1. Créer et utiliser un thread
    let mut thread = agent.new_thread();
    let response = agent.invoke("Bonjour!", &mut thread).await?;

    // 2. Sérialiser pour pause (stockage longue durée)
    let serialized = thread.serialize()?;
    nats.kv_put("paused_conversations.user_123", &serialized).await?;

    // ... Plus tard (même process ou autre) ...

    // 3. Reprendre la conversation
    let data = nats.kv_get("paused_conversations.user_123").await?;
    let mut restored_thread = AgentThread::deserialize(&data)?;

    // 4. Continuer avec le même agent (stateless!)
    let response = agent.invoke("On en était où ?", &mut restored_thread).await?;
    // L'agent a accès à tout l'historique via le thread restauré
}
```

#### ChatMessageStore: Factory pour Storage Personnalisé

Pour des besoins spécifiques (base de données, cache, etc.):

```rust
/// Factory pour créer des message stores personnalisés
pub trait ChatMessageStoreFactory: Send + Sync {
    type Store: ChatMessageStore;

    fn create(&self, thread_id: &ThreadId) -> Self::Store;
}

/// Store par défaut: NATS KV
pub struct NatsMessageStoreFactory {
    nats: NatsClient,
    bucket: String,
}

impl ChatMessageStoreFactory for NatsMessageStoreFactory {
    type Store = NatsMessageStore;

    fn create(&self, thread_id: &ThreadId) -> Self::Store {
        NatsMessageStore {
            nats: self.nats.clone(),
            key: format!("{}.{}", self.bucket, thread_id),
        }
    }
}

// Utilisation avec factory personnalisée
let agent = ChatAgentBuilder::new()
    .model("gpt-4o")
    .message_store_factory(NatsMessageStoreFactory::new(nats, "threads"))
    .build();
```

#### Compatibilité Thread ↔ Agent

> ⚠️ **Attention**: Un thread créé avec un agent doit être utilisé avec un agent **compatible**.

Critères de compatibilité:
- Même format de messages (ChatMessage vs autre)
- Même schéma d'outils (tools disponibles)
- Même modèle LLM (ou compatible)

---

### Background Responses: Tâches Longues avec Continuation

> ⚠️ **STATUT: NON IMPLÉMENTÉ** - Planifié pour Phase 9.

> Inspiré de [MAF Background Responses](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/agents/agent-background-responses)

Pour les tâches de longue durée (raisonnement complexe, génération massive), on utilise un système de **continuation token** permettant de reprendre le traitement.

```
┌─────────────────────────────────────────────────────────────────┐
│                    BACKGROUND RESPONSE FLOW                     │
│                                                                 │
│  Client                        Agent                            │
│    │                             │                              │
│    │──── invoke("long task") ───▶│                              │
│    │                             │                              │
│    │◀── ContinuationToken + ─────│  (traitement en cours)       │
│    │    partial_result           │                              │
│    │                             │                              │
│    │ ... poll/wait ...           │  ... processing ...          │
│    │                             │                              │
│    │──── resume(token) ─────────▶│                              │
│    │                             │                              │
│    │◀── final_result ────────────│  (terminé)                   │
│    │    token = None             │                              │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

#### API Rust avec NATS

```rust
/// Jeton de continuation pour reprendre une tâche longue
#[derive(Serialize, Deserialize, Clone)]
pub struct ContinuationToken {
    pub task_id: TaskId,
    pub checkpoint: Vec<u8>,  // État sérialisé
    pub created_at: DateTime<Utc>,
}

/// Réponse d'un agent (peut être partielle ou finale)
pub enum AgentResponse<T> {
    /// Tâche terminée
    Complete(T),
    /// Tâche en cours, utiliser le token pour reprendre
    InProgress {
        partial_result: Option<T>,
        continuation_token: ContinuationToken,
    },
}

impl Agent {
    /// Invoque avec support background responses
    pub async fn invoke_with_background(
        &self,
        input: &str,
        thread: &mut AgentThread,
        options: InvokeOptions,
    ) -> Result<AgentResponse<String>, AgentError> {
        // Si un token est fourni, reprendre depuis le checkpoint
        if let Some(token) = options.continuation_token {
            return self.resume_from_token(token, thread).await;
        }

        // Démarrer la tâche
        let task_id = TaskId::new();

        // Publier sur NATS pour traitement async
        self.nats.publish(
            &format!("agent.{}.task.{}", self.id, task_id),
            &BackgroundTask { input: input.to_string(), thread_id: thread.id.clone() }
        ).await?;

        // Retourner immédiatement avec token
        Ok(AgentResponse::InProgress {
            partial_result: None,
            continuation_token: ContinuationToken {
                task_id,
                checkpoint: vec![],
                created_at: Utc::now(),
            },
        })
    }
}

// Exemple: Polling pour tâche longue
async fn long_task_example(agent: &ChatAgent, thread: &mut AgentThread) {
    let mut response = agent.invoke_with_background(
        "Écris un roman très long...",
        thread,
        InvokeOptions { allow_background: true, ..Default::default() }
    ).await?;

    // Polling jusqu'à completion
    while let AgentResponse::InProgress { continuation_token, .. } = response {
        tokio::time::sleep(Duration::from_secs(2)).await;

        response = agent.invoke_with_background(
            "", // Pas de nouveau input, juste reprendre
            thread,
            InvokeOptions { continuation_token: Some(continuation_token), ..Default::default() }
        ).await?;
    }

    if let AgentResponse::Complete(result) = response {
        println!("Résultat final: {}", result);
    }
}
```

#### Streaming avec Reprise (via NATS JetStream)

```rust
// Le token permet aussi de reprendre un stream interrompu
async fn resumable_stream(agent: &ChatAgent, thread: &mut AgentThread) {
    let mut stream = agent.invoke_stream_with_background(
        "Génère du contenu...",
        thread,
        InvokeOptions { allow_background: true, ..Default::default() }
    ).await?;

    let mut last_token = None;

    // Consumer le stream (peut être interrompu)
    while let Some(update) = stream.next().await {
        print!("{}", update.text);
        last_token = update.continuation_token;

        // Simulation d'interruption réseau
        if some_condition { break; }
    }

    // Plus tard: reprendre depuis le dernier token
    if let Some(token) = last_token {
        let resumed_stream = agent.invoke_stream_with_background(
            "",
            thread,
            InvokeOptions { continuation_token: Some(token), ..Default::default() }
        ).await?;

        // Continuer à consumer...
    }
}
```

---

### Agent Memory: Court Terme et Long Terme

> ✅ **STATUT: PARTIELLEMENT IMPLÉMENTÉ** - Phase 8 complète pour in-memory.
>
> Implémenté:
> - `MemoryProvider` trait avec `before_invoke`/`after_invoke`
> - `InMemoryProvider` et `NoOpMemoryProvider`
> - `Memory`, `MemoryCategory`, `UserMemories`, `MemoryContext`
> - `ChatReducer` trait avec 4 implémentations
> - Example: `examples/memory_demo.rs`
>
> À faire: `NatsMemoryProvider` (Phase 9)

> Inspiré de [MAF Agent Memory](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/agents/agent-memory)

```
┌─────────────────────────────────────────────────────────────────┐
│                      ARCHITECTURE MÉMOIRE                       │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   SHORT-TERM MEMORY                     │    │
│  │              (Historique de conversation)               │    │
│  │                                                         │    │
│  │   Thread = [msg1, msg2, msg3, ...]                      │    │
│  │                     │                                   │    │
│  │                     ▼                                   │    │
│  │   NATS KV: "thread.{id}" → messages sérialisés          │    │
│  │                                                         │    │
│  └─────────────────────────────────────────────────────────┘    │
│                             │                                   │
│                             ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   LONG-TERM MEMORY                      │    │
│  │            (Souvenirs persistants, RAG)                 │    │
│  │                                                         │    │
│  │   MemoryProvider → extraire/injecter des souvenirs      │    │
│  │                     │                                   │    │
│  │                     ▼                                   │    │
│  │   NATS KV: "memory.{agent_id}.{user_id}" → embeddings   │    │
│  │                                                         │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

#### Types de Mémoire

| Type | Durée | Stockage | Cas d'usage |
|------|-------|----------|-------------|
| **Short-term** | Session | Thread (NATS KV) | Historique conversation en cours |
| **Long-term** | Persistant | Memory Store (NATS KV) | Préférences utilisateur, faits mémorisés |
| **Semantic** | Persistant | Vector Store | RAG, recherche par similarité |

#### API Memory

```rust
/// Provider de contexte mémoire (injecte/extrait des souvenirs)
#[async_trait]
pub trait MemoryProvider: Send + Sync {
    /// Appelé AVANT chaque invocation - injecte du contexte
    async fn before_invoke(
        &self,
        agent_id: &AgentId,
        thread: &AgentThread,
        ctx: &mut MemoryContext,
    ) -> Result<(), MemoryError>;

    /// Appelé APRÈS chaque invocation - extrait des souvenirs
    async fn after_invoke(
        &self,
        agent_id: &AgentId,
        thread: &AgentThread,
        response: &AgentResponse,
    ) -> Result<(), MemoryError>;
}

/// Contexte mémoire injectable
pub struct MemoryContext {
    /// Instructions additionnelles à injecter
    pub extra_instructions: Option<String>,
    /// Messages système à ajouter
    pub system_messages: Vec<ChatMessage>,
}

/// Implémentation NATS pour mémoire long-terme
pub struct NatsMemoryProvider {
    nats: NatsClient,
    bucket: String,
}

impl MemoryProvider for NatsMemoryProvider {
    async fn before_invoke(
        &self,
        agent_id: &AgentId,
        thread: &AgentThread,
        ctx: &mut MemoryContext,
    ) -> Result<(), MemoryError> {
        // Récupérer les souvenirs de l'utilisateur
        let user_id = thread.metadata.user_id.as_ref();
        if let Some(user_id) = user_id {
            let key = format!("{}.{}.{}", self.bucket, agent_id, user_id);
            if let Some(memories) = self.nats.kv_get::<UserMemories>(&key).await? {
                ctx.extra_instructions = Some(format!(
                    "Souvenirs de l'utilisateur:\n{}",
                    memories.to_context_string()
                ));
            }
        }
        Ok(())
    }

    async fn after_invoke(
        &self,
        agent_id: &AgentId,
        thread: &AgentThread,
        response: &AgentResponse,
    ) -> Result<(), MemoryError> {
        // Extraire et stocker les nouveaux souvenirs
        // (détection automatique de faits importants)
        let new_memories = extract_memories_from_response(response)?;
        if !new_memories.is_empty() {
            let user_id = thread.metadata.user_id.as_ref().unwrap();
            let key = format!("{}.{}.{}", self.bucket, agent_id, user_id);

            let mut memories = self.nats.kv_get::<UserMemories>(&key).await?
                .unwrap_or_default();
            memories.extend(new_memories);
            self.nats.kv_put(&key, &memories).await?;
        }
        Ok(())
    }
}
```

#### Réduction de l'Historique (Context Window Management)

```rust
/// Stratégie de réduction de l'historique pour respecter la fenêtre de contexte
pub trait ChatReducer: Send + Sync {
    fn reduce(&self, messages: &[ChatMessage]) -> Vec<ChatMessage>;
}

/// Garde les N derniers messages
pub struct MessageCountingReducer {
    max_messages: usize,
}

impl ChatReducer for MessageCountingReducer {
    fn reduce(&self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        if messages.len() <= self.max_messages {
            messages.to_vec()
        } else {
            // Garde le system message + les N derniers
            let system = messages.iter()
                .filter(|m| m.role == Role::System)
                .cloned()
                .collect::<Vec<_>>();

            let recent = messages.iter()
                .filter(|m| m.role != Role::System)
                .rev()
                .take(self.max_messages - system.len())
                .cloned()
                .rev()
                .collect::<Vec<_>>();

            [system, recent].concat()
        }
    }
}

/// Résume les anciens messages pour libérer du contexte
pub struct SummarizingReducer {
    summarizer: Arc<dyn Agent>,
    threshold: usize,
}
```

#### Builder avec Memory

```rust
let agent = ChatAgentBuilder::new()
    .model("gpt-4o")
    .instructions("Tu es un assistant personnel.")
    // Mémoire long-terme
    .memory_provider(NatsMemoryProvider::new(nats.clone(), "memories"))
    // Réduction automatique de l'historique
    .chat_reducer(MessageCountingReducer::new(20))
    .build();
```

---

## Executor: L'Abstraction Centrale

Un **Executor** est un noeud générique qui peut prendre 3 formes:

```
                    ┌─────────────────────────────────────┐
                    │         EXECUTOR (Node)             │
                    │                                     │
                    │  ┌───────────────────────────────┐  │
                    │  │           WORKER              │  │
                    │  │  - Fait un travail            │  │
                    │  │  - LLM call, transform, merge │  │
                    │  │  - Input → Output             │  │
                    │  └───────────────────────────────┘  │
                    │                                     │
                    │  ┌───────────────────────────────┐  │
                    │  │          VALIDATOR            │  │
                    │  │  - Vérifie quelque chose      │  │
                    │  │  - Compile, test, lint        │  │
                    │  │  - Input → Pass/Fail + Errors │  │
                    │  └───────────────────────────────┘  │
                    │                                     │
                    │  ┌───────────────────────────────┐  │
                    │  │        ORCHESTRATOR           │  │
                    │  │  - Coordonne un sub-graph     │  │
                    │  │  - Contient d'autres Executors│  │
                    │  │  - Récursif/Nested            │  │
                    │  └───────────────────────────────┘  │
                    │                                     │
                    └─────────────────────────────────────┘
```

### Les 3 Types d'Executor

| Type | Rôle | Inputs | Outputs | Exemples |
|------|------|--------|---------|----------|
| **Worker** | Fait un travail | Data | Data transformée | Génère code, merge, format |
| **Validator** | Vérifie | Data à valider | Pass/Fail + Errors | Compile, test, lint, review |
| **Orchestrator** | Coordonne | Task + Config | Result du sub-graph | Gère une sous-tâche complexe |

### Point Clé: Un Validator EST un Executor

> **Important**: Contrairement à l'architecture actuelle où les Validators sont des composants **séparés** appelés après l'exécution, dans la nouvelle architecture un Validator est simplement un **Executor avec `kind = Validator`**.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    TOUS IMPLÉMENTENT LE MÊME TRAIT                          │
│                                                                             │
│   impl Executor for CodeGenerator {        // Worker                        │
│       type Output = GeneratedCode;                                          │
│       fn kind() -> ExecutorKind::Worker                                     │
│   }                                                                         │
│                                                                             │
│   impl Executor for CompileValidator {     // Validator = Executor aussi!   │
│       type Output = ValidationResult;      // Output = { passed, errors }   │
│       fn kind() -> ExecutorKind::Validator                                  │
│   }                                                                         │
│                                                                             │
│   impl Executor for SubWorkflow {          // Orchestrator                  │
│       type Output = SubTaskResult;                                          │
│       fn kind() -> ExecutorKind::Orchestrator                               │
│   }                                                                         │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Pourquoi c'est important:**

| Aspect | Avant (séparé) | Après (unifié) |
|--------|----------------|----------------|
| **Routing** | `if !validator.validate() { retry() }` manuel | Edge conditionnel automatique |
| **Feedback loop** | Code custom externe | `add_conditional_edge(validator, worker, \|r\| !r.passed)` |
| **Dans le graph** | Non, externe | Oui, c'est un noeud comme les autres |
| **Historique NATS** | Système séparé | Même `ctx.previous_errors()` que les Workers |
| **Parallélisme** | Séquentiel après worker | Peut être en parallèle dans un superstep |

**Conséquence pratique**: Les edges conditionnels peuvent router selon `passed`:
```rust
// Si validation échoue → retour au worker avec feedback
.add_conditional_edge(compile_validator, impl_generator, |r| !r.passed, "retry_on_failure")

// Si validation réussit → continuer au prochain validator
.add_conditional_edge(compile_validator, test_validator, |r| r.passed, "continue_on_success")
```

### Récursivité: Orchestrator contient un Graph

```
Orchestrator (main task: "Create State Machine")
│
├── Worker (types)
│   └── Génère: interfaces, types
│
├── Orchestrator (impl) ◄─── SUB-GRAPH récursif!
│   │
│   ├── Worker (skeleton)
│   │   └── Génère: class avec stubs
│   │
│   ├── Worker (methods)
│   │   └── Génère: implémentations
│   │
│   └── Validator (compile-check)
│       └── Vérifie: tsc --noEmit
│
├── Worker (tests)
│   └── Génère: tests Jest (INDÉPENDANT)
│
├── Worker (assembler)
│   └── Merge: types + impl + tests
│
└── Validator (final)
    ├── Validator (compile)
    ├── Validator (test)
    └── Validator (lint)
```

### Trait Executor (inspiré de MAF)

```rust
/// Trait commun à tous les Executors
/// Inspiré de Microsoft Agent Framework: Executor<TInput, TOutput>
#[async_trait]
pub trait Executor: Send + Sync {
    /// Type d'input accepté
    type Input: Serialize + DeserializeOwned;
    /// Type d'output (messages vers autres executors)
    type Message: Serialize + DeserializeOwned;
    /// Type de sortie finale (visible par le caller du workflow)
    type Output: Serialize + DeserializeOwned;

    /// Identifiant unique
    fn id(&self) -> &ExecutorId;

    /// Type d'executor (Worker, Validator, Orchestrator)
    fn kind(&self) -> ExecutorKind;

    /// Traite un message et utilise le contexte pour communiquer
    async fn handle<Ctx>(
        &self,
        input: Self::Input,
        ctx: &mut Ctx,
    ) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExecutorKind {
    /// Fait un travail (LLM call, transform, merge)
    Worker,
    /// Vérifie quelque chose (compile, test, lint)
    Validator,
    /// Coordonne un sub-graph (récursif)
    Orchestrator,
}
```

### MAF Dynamic Dispatch Pattern

> **Important**: Inspiré de [MAF Python Executors](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/core-concepts/executors),
> nos executors utilisent `serde_json::Value` pour le **dynamic dispatch**.

**Problème**: L'`ExecutorRegistry` impose une contrainte de types:
```rust
pub struct ExecutorRegistry<TMessage, TOutput> {
    executors: HashMap<ExecutorId, Arc<dyn Executor<Message = TMessage, Output = TOutput>>>,
}
```

Tous les executors d'un workflow DOIVENT avoir les mêmes types `Message` et `Output`.

**Solution MAF**: Type erasure avec `serde_json::Value`:

```rust
// TOUS les executors built-in utilisent le même pattern:
impl Executor for LLMGeneratorExecutor {
    type Input = Value;     // Dynamic dispatch
    type Message = Value;   // Dynamic dispatch
    type Output = String;   // Status messages

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Deserialize dynamically
        let typed_input: GeneratorInput = serde_json::from_value(input)?;

        // ... process ...

        // Serialize output to Value
        ctx.send_message(generated.to_value()).await?;
        Ok(())
    }
}
```

**Avantages**:
- ✅ Tous les executors compatibles dans un même workflow
- ✅ Type-safe construction avec helpers (`GeneratorInput`, `CompileInput`, etc.)
- ✅ Runtime flexibility pour routing dynamique
- ✅ Matches MAF Python pattern avec `WorkflowContext[T]`

**Trade-off**: Validation des types au runtime plutôt qu'à la compilation.

### Input/Output Schemas

Chaque executor documente son schema JSON:

| Executor | Input Schema | Output Schema |
|----------|--------------|---------------|
| `LLMGeneratorExecutor` | `GeneratorInput` → `{prompt, language, context?, previous_errors?}` | `GeneratedCode` → `{code, language, prompt, tokens_used}` |
| `CompileValidatorExecutor` | `CompileInput` → `{code, language, task?}` | `CompileOutput` → `{code, language, passed, errors, feedback?}` |
| `TestValidatorExecutor` | `TestInput` → `{code, language, test_filter?, task?}` | `TestOutput` → `{code, language, passed, test_count, tests_passed, tests_failed, errors}` |
| `CodeAssemblerExecutor` | `AssemblyInput` → `{parts: [{name, code, language, section?}], language}` | `AssembledCode` → `{code, language, part_count, size_bytes}` |
| `SubWorkflowExecutor` | `SubWorkflowInput` → `{input: any, task_id?}` | `SubWorkflowOutput` → `{outputs, superstep_count, duration_ms, success, error?}` |
```

### Executors Déclaratifs (Function-based)

Inspiré du pattern `@executor` de MAF Python, on peut créer des executors à partir de fonctions:

```rust
/// Macro pour créer un executor à partir d'une fonction
/// Équivalent du décorateur @executor de MAF Python
#[executor(id = "uppercase")]
async fn uppercase(text: String, ctx: &mut WorkflowContext<String>) {
    ctx.send_message(text.to_uppercase()).await;
}

// Équivalent à:
pub struct UppercaseExecutor;

#[async_trait]
impl Executor for UppercaseExecutor {
    type Input = String;
    type Message = String;
    type Output = Never;

    fn id(&self) -> &ExecutorId { &ExecutorId::new("uppercase") }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Worker }

    async fn handle(&self, text: String, ctx: &mut WorkflowContext<String>)
        -> Result<(), ExecutorError>
    {
        ctx.send_message(text.to_uppercase()).await?;
        Ok(())
    }
}
```

### Multi-Handler Executors

Un executor peut gérer plusieurs types de messages (routing interne):

```rust
/// Executor avec plusieurs handlers selon le type de message
pub struct MultiTypeExecutor {
    id: ExecutorId,
}

impl MultiTypeExecutor {
    /// Handler pour les String
    async fn handle_string(&self, text: String, ctx: &mut WorkflowContext<String>) {
        ctx.send_message(text.to_uppercase()).await;
    }

    /// Handler pour les i32
    async fn handle_int(&self, num: i32, ctx: &mut WorkflowContext<i32>) {
        ctx.send_message(num * 2).await;
    }
}

// En Rust, on utilise un enum pour le multi-type
#[derive(Serialize, Deserialize)]
pub enum MultiInput {
    Text(String),
    Number(i32),
}

#[derive(Serialize, Deserialize)]
pub enum MultiOutput {
    Text(String),
    Number(i32),
}

#[async_trait]
impl Executor for MultiTypeExecutor {
    type Input = MultiInput;
    type Message = MultiOutput;
    type Output = Never;

    async fn handle(&self, input: MultiInput, ctx: &mut WorkflowContext<MultiOutput>)
        -> Result<(), ExecutorError>
    {
        match input {
            MultiInput::Text(s) => {
                ctx.send_message(MultiOutput::Text(s.to_uppercase())).await?;
            }
            MultiInput::Number(n) => {
                ctx.send_message(MultiOutput::Number(n * 2)).await?;
            }
        }
        Ok(())
    }
}
```

### WorkflowContext: Communication

Le `WorkflowContext` est générique sur deux types (comme en MAF):
- `TMessage`: type des messages envoyés aux autres executors (via edges)
- `TOutput`: type des sorties finales visibles par le caller du workflow

```rust
/// Contexte fourni à chaque Executor pour communiquer
/// Générique: WorkflowContext<TMessage, TOutput>
pub struct WorkflowContext<TMessage, TOutput = Never> {
    /// ID de l'executor courant
    executor_id: ExecutorId,
    /// Client NATS pour mémoire/historique
    nats: Arc<NatsClient>,
    /// Messages à envoyer aux edges sortants
    outgoing_messages: Vec<TMessage>,
    /// Outputs à retourner au workflow caller
    outputs: Vec<TOutput>,
    /// Historique (lu depuis NATS)
    history: ExecutorHistory,
}
```

### Les 3 méthodes de communication

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    TROIS MÉTHODES, TROIS DESTINATIONS                        │
│                                                                             │
│  send_message(msg)        yield_output(out)        add_event(evt)          │
│       │                        │                        │                   │
│       ▼                        ▼                        ▼                   │
│  ┌─────────────┐        ┌─────────────┐        ┌─────────────┐             │
│  │   EDGES     │        │   CALLER    │        │    NATS     │             │
│  │  (internes) │        │  (externe)  │        │  (observ.)  │             │
│  └──────┬──────┘        └─────────────┘        └──────┬──────┘             │
│         │                                             │                     │
│         ▼                                             ▼                     │
│  Autres Executors        Agent/code appelant    Monitoring/Audit            │
└─────────────────────────────────────────────────────────────────────────────┘
```

| Méthode | Destination | NATS Subject | Usage |
|---------|-------------|--------------|-------|
| `send_message(msg)` | Edges → autres Executors | (interne) | Flux de données dans le workflow |
| `yield_output(out)` | Caller (Agent, code) | `agentic.workflow.{id}.output` | Résultat visible de l'extérieur |
| `add_event(evt)` | Stream NATS | `agentic.executor.{id}.event.*` | Observabilité, audit, monitoring |

```rust
impl<TMessage: Serialize, TOutput: Serialize> WorkflowContext<TMessage, TOutput> {
    /// Envoie un message aux executors connectés (via edges)
    /// → Flux INTERNE au workflow
    pub async fn send_message(&mut self, message: TMessage) -> Result<(), ContextError> {
        self.outgoing_messages.push(message);
        Ok(())
    }

    /// Produit un output visible par le caller du workflow
    /// → Flux EXTERNE (résultat final)
    pub async fn yield_output(&mut self, output: TOutput) -> Result<(), ContextError> {
        self.outputs.push(output);
        // Aussi publié sur NATS pour le streaming
        self.nats.publish(
            &format!("agentic.workflow.{}.output", self.workflow_id),
            &output
        ).await?;
        Ok(())
    }

    /// Émet un événement custom (observabilité)
    /// Inspiré de MAF ctx.add_event()
    /// → Publié sur NATS pour monitoring/audit
    pub async fn add_event<E: WorkflowEvent>(&self, event: E) -> Result<(), ContextError> {
        self.nats.publish(
            &format!("agentic.executor.{}.event.{}", self.executor_id, E::event_type()),
            &event
        ).await?;
        // Aussi ajouté au stream d'événements du workflow
        self.events.push(Box::new(event));
        Ok(())
    }

    /// Accède à l'historique des attempts précédents (depuis NATS)
    pub fn previous_attempts(&self) -> &[Attempt] {
        &self.history.attempts
    }

    /// Accède aux erreurs des attempts précédents (depuis NATS)
    pub fn previous_errors(&self) -> &[ExecutorError] {
        &self.history.errors
    }

    /// Accède aux patterns qui ont marché (depuis NATS)
    pub fn successful_patterns(&self) -> &[Pattern] {
        &self.history.successful_patterns
    }

    /// Log un événement sur NATS (raccourci pour add_event avec LogEvent)
    pub async fn log(&self, level: LogLevel, message: &str) -> Result<(), ContextError> {
        self.add_event(LogEvent {
            level,
            message: message.to_string(),
            timestamp: Utc::now()
        }).await
    }

    // ══════════════════════════════════════════════════════════════════════
    // SHARED STATE (inspiré MAF) - Stocké dans NATS KV
    // ══════════════════════════════════════════════════════════════════════

    /// Stocke un état partagé accessible par d'autres executors
    /// Inspiré de MAF ctx.set_shared_state()
    /// → Persisté dans NATS KV
    pub async fn set_shared_state<T: Serialize>(
        &self,
        key: &str,
        value: T,
    ) -> Result<(), ContextError> {
        self.nats.kv_put(
            &format!("workflow.{}.state.{}", self.workflow_id, key),
            &value
        ).await?;
        Ok(())
    }

    /// Lit un état partagé stocké par un autre executor
    /// Inspiré de MAF ctx.get_shared_state()
    /// → Lu depuis NATS KV
    pub async fn get_shared_state<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, ContextError> {
        self.nats.kv_get(
            &format!("workflow.{}.state.{}", self.workflow_id, key)
        ).await
    }

    // ══════════════════════════════════════════════════════════════════════
    // REQUEST/RESPONSE (inspiré MAF) - Human-in-the-loop via NATS
    // ══════════════════════════════════════════════════════════════════════

    /// Envoie une requête externe et attend une réponse
    /// Utile pour human-in-the-loop, approbations, etc.
    /// Inspiré de MAF ctx.request_info()
    /// → Utilise NATS Request-Reply pattern
    pub async fn request_info<Req: Serialize, Res: DeserializeOwned>(
        &self,
        request: Req,
    ) -> Result<Res, ContextError> {
        // Publier un événement de requête
        let request_id = uuid::Uuid::new_v4().to_string();

        self.add_event(RequestInfoEvent {
            request_id: request_id.clone(),
            request: serde_json::to_value(&request)?,
        }).await?;

        // Attendre la réponse via NATS
        let response: Res = self.nats.request(
            &format!("agentic.workflow.{}.request.{}", self.workflow_id, request_id),
            &request
        ).await?;

        Ok(response)
    }
}
```

### Shared State: État partagé entre Executors

Inspiré de [MAF Shared States](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/workflows/shared-states), permet à plusieurs executors de partager des données sans passer par les edges.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         SHARED STATE (via NATS KV)                          │
│                                                                             │
│   FileReader                        WordCounter                             │
│  ┌──────────┐                      ┌──────────┐                            │
│  │  read()  │                      │  count() │                            │
│  └────┬─────┘                      └────┬─────┘                            │
│       │                                 │                                   │
│       │ set_shared_state               │ get_shared_state                   │
│       │ ("file_abc", content)          │ ("file_abc")                       │
│       │                                 │                                   │
│       ▼                                 ▼                                   │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                         NATS KV                                      │   │
│  │  workflow.{id}.state.file_abc = "contenu du fichier..."             │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
│  Usage: gros fichiers, données intermédiaires, cache partagé               │
└─────────────────────────────────────────────────────────────────────────────┘
```

```rust
// Executor qui lit un fichier et stocke dans shared state
impl Executor for FileReader {
    async fn handle(&self, path: String, ctx: &mut WorkflowContext<String>) {
        let content = std::fs::read_to_string(&path)?;
        let file_id = uuid::Uuid::new_v4().to_string();

        // Stocker dans NATS KV (shared state)
        ctx.set_shared_state(&file_id, &content).await?;

        // Passer seulement l'ID via edge (pas le contenu!)
        ctx.send_message(file_id).await?;
    }
}

// Executor qui récupère le fichier depuis shared state
impl Executor for WordCounter {
    async fn handle(&self, file_id: String, ctx: &mut WorkflowContext<usize>) {
        // Récupérer depuis NATS KV
        let content: String = ctx.get_shared_state(&file_id).await?
            .ok_or(ContextError::StateNotFound)?;

        let word_count = content.split_whitespace().count();
        ctx.send_message(word_count).await?;
    }
}
```

### Request/Response: Human-in-the-loop

Inspiré de [MAF Requests and Responses](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/workflows/requests-and-responses), permet aux executors de demander une intervention externe (humain, API, etc.).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    REQUEST/RESPONSE (Human-in-the-loop)                     │
│                                                                             │
│   Executor                          External System                         │
│  ┌──────────┐                      ┌──────────────────┐                    │
│  │ needs    │ ── request_info ──▶  │  Human / API     │                    │
│  │ approval │                      │                  │                    │
│  │          │ ◀── response ──────  │  [Approve/Deny]  │                    │
│  └──────────┘                      └──────────────────┘                    │
│       │                                                                     │
│       ▼                                                                     │
│  continue workflow                                                          │
│                                                                             │
│  Transport: NATS Request-Reply pattern                                      │
│  Subject: agentic.workflow.{id}.request.{request_id}                       │
└─────────────────────────────────────────────────────────────────────────────┘
```

```rust
/// Requête d'approbation
#[derive(Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub action: String,
    pub details: String,
}

#[derive(Serialize, Deserialize)]
pub struct ApprovalResponse {
    pub approved: bool,
    pub reason: Option<String>,
}

// Executor qui demande une approbation
impl Executor for DeployExecutor {
    async fn handle(&self, deploy_config: DeployConfig, ctx: &mut WorkflowContext<DeployResult>) {
        // Demander approbation humaine
        let response: ApprovalResponse = ctx.request_info(ApprovalRequest {
            action: "deploy".into(),
            details: format!("Deploy to {} with config {:?}", deploy_config.env, deploy_config),
        }).await?;

        if !response.approved {
            return Err(ExecutorError::Rejected(response.reason.unwrap_or_default()));
        }

        // Continuer le déploiement
        let result = self.deploy(&deploy_config).await?;
        ctx.send_message(result).await?;
    }
}
```

#### Handler de réponses (côté externe)

```rust
// Côté système externe qui écoute les requêtes
async fn handle_approval_requests(nats: &NatsClient) {
    let mut sub = nats.subscribe("agentic.workflow.*.request.*").await?;

    while let Some(msg) = sub.next().await {
        let request: ApprovalRequest = serde_json::from_slice(&msg.payload)?;

        // Afficher à l'utilisateur et attendre sa décision
        println!("Approval requested: {}", request.details);
        let user_input = prompt_user("Approve? (y/n): ");

        let response = ApprovalResponse {
            approved: user_input == "y",
            reason: if user_input != "y" { Some("User rejected".into()) } else { None },
        };

        // Répondre via NATS
        msg.respond(serde_json::to_vec(&response)?).await?;
    }
}
```

### Événements Custom

Inspiré de MAF, les executors peuvent émettre des événements personnalisés:

```rust
/// Trait pour les événements custom
pub trait WorkflowEvent: Serialize + Send + Sync {
    fn event_type() -> &'static str;
}

/// Événement custom: progression de génération
#[derive(Serialize)]
pub struct GenerationProgressEvent {
    pub phase: String,
    pub percent: u8,
    pub tokens_used: u32,
}

impl WorkflowEvent for GenerationProgressEvent {
    fn event_type() -> &'static str { "generation_progress" }
}

/// Utilisation dans un executor
impl Executor for CodeGenerator {
    async fn handle(&self, input: GenerationRequest, ctx: &mut WorkflowContext<GeneratedCode>)
        -> Result<(), ExecutorError>
    {
        // Émettre un événement de progression
        ctx.add_event(GenerationProgressEvent {
            phase: "parsing".into(),
            percent: 10,
            tokens_used: 0,
        }).await?;

        let types = self.generate_types(&input).await?;

        ctx.add_event(GenerationProgressEvent {
            phase: "types_done".into(),
            percent: 50,
            tokens_used: 500,
        }).await?;

        let impl_code = self.generate_impl(&input, &types).await?;

        ctx.add_event(GenerationProgressEvent {
            phase: "impl_done".into(),
            percent: 100,
            tokens_used: 1200,
        }).await?;

        ctx.send_message(GeneratedCode { types, impl_code }).await?;
        Ok(())
    }
}
```

### Mapping Events → NATS

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         EVENTS → NATS SUBJECTS                              │
│                                                                             │
│  MAF Event                    │  NATS Subject                               │
│  ─────────────────────────────┼─────────────────────────────────────────    │
│  ExecutorInvokedEvent         │  agentic.executor.{id}.started              │
│  ExecutorCompletedEvent       │  agentic.executor.{id}.completed            │
│  ExecutorFailedEvent          │  agentic.executor.{id}.failed               │
│  SuperStepStartedEvent        │  agentic.workflow.{id}.superstep.started    │
│  SuperStepCompletedEvent      │  agentic.workflow.{id}.superstep.completed  │
│  WorkflowOutputEvent          │  agentic.workflow.{id}.output               │
│  WorkflowErrorEvent           │  agentic.workflow.{id}.error                │
│  CustomEvent (via add_event)  │  agentic.executor.{id}.event.{type}         │
│                                                                             │
│  Tous les events sont aussi persistés dans:                                 │
│  - Stream: AGENTIC_EVENTS (pour replay)                                     │
│  - KV: workflow.{id}.events (pour query)                                    │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Exemples d'usage

```rust
// Executor qui ne fait que passer des messages (pas de sortie finale)
impl Executor for CodeGenerator {
    type Input = GenerationRequest;
    type Message = GeneratedCode;
    type Output = Never;  // Pas de yield_output

    async fn handle(&self, input: GenerationRequest, ctx: &mut WorkflowContext<GeneratedCode>) {
        let code = self.llm.generate(&input).await?;
        ctx.send_message(code).await?;  // → vers Assembler via edge
    }
}

// Executor final qui produit la sortie du workflow
impl Executor for FinalAssembler {
    type Input = AllParts;
    type Message = Never;  // Pas de send_message
    type Output = FinalResult;

    async fn handle(&self, input: AllParts, ctx: &mut WorkflowContext<Never, FinalResult>) {
        let result = self.assemble(input)?;
        ctx.yield_output(result).await?;  // → visible par l'Agent
    }
}

// Executor qui fait les deux
impl Executor for IntermediateProcessor {
    type Input = RawData;
    type Message = ProcessedData;
    type Output = ProgressUpdate;

    async fn handle(&self, input: RawData, ctx: &mut WorkflowContext<ProcessedData, ProgressUpdate>) {
        // Signaler la progression à l'extérieur
        ctx.yield_output(ProgressUpdate::Started).await?;

        let processed = self.process(input)?;

        // Envoyer aux executors suivants
        ctx.send_message(processed).await?;

        ctx.yield_output(ProgressUpdate::Completed).await?;
    }
}
```

### Exemples d'Executors Concrets (MAF Pattern)

> **Note**: Tous les executors utilisent `serde_json::Value` pour Input/Message
> (MAF dynamic dispatch). Les types helpers (`GeneratorInput`, etc.) sont pour
> la construction type-safe côté appelant.

```rust
use serde_json::Value;

// === WORKER: Génère du code avec LLM ===
pub struct LLMGeneratorExecutor {
    id: ExecutorId,
    llm: Arc<Mutex<LLMExecutor>>,
    system_prompt: String,
}

#[async_trait]
impl Executor for LLMGeneratorExecutor {
    type Input = Value;      // MAF: dynamic dispatch
    type Message = Value;    // MAF: dynamic dispatch
    type Output = String;    // Status messages

    fn id(&self) -> &ExecutorId { &self.id }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Worker }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Deserialize dynamically
        let typed_input: GeneratorInput = serde_json::from_value(input)?;

        // Appeler le LLM
        let response = self.llm.lock().await.execute(&self.system_prompt, &typed_input.prompt).await?;

        // Build output
        let generated = GeneratedCode::new(&response.content, &typed_input.language);

        // Serialize output to Value for next executors
        ctx.send_message(generated.to_value()).await?;

        // Yield status output
        ctx.yield_output(format!("Generated {} bytes", response.content.len())).await?;

        Ok(())
    }
}

// === VALIDATOR: Vérifie la compilation ===
pub struct CompileValidatorExecutor<S: Sandbox> {
    id: ExecutorId,
    validator: SandboxValidator<S>,
}

#[async_trait]
impl<S: Sandbox + Send + Sync + 'static> Executor for CompileValidatorExecutor<S> {
    type Input = Value;      // MAF: dynamic dispatch
    type Message = Value;    // MAF: dynamic dispatch
    type Output = String;    // Status messages

    fn id(&self) -> &ExecutorId { &self.id }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Validator }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Deserialize dynamically
        let typed_input: CompileInput = serde_json::from_value(input)?;

        // Compiler dans le sandbox
        let result = self.validator.validate_code(&typed_input.code, language).await?;

        // Build output
        let output = CompileOutput {
            code: typed_input.code.clone(),
            language: typed_input.language.clone(),
            passed: result.compiles,
            errors: result.compiler_errors,
            feedback: result.feedback,
            execution_time_ms: result.execution_time_ms,
        };

        // Serialize to Value (edges conditionnels routeront selon "passed")
        ctx.send_message(output.to_value()).await?;

        // Yield status
        let status = if output.passed { "Compilation: PASSED" } else { "Compilation: FAILED" };
        ctx.yield_output(status.into()).await?;

        Ok(())
    }
}

// === WORKER: Assemble code fragments ===
pub struct CodeAssemblerExecutor {
    id: ExecutorId,
    assembler: CodeAssembler,
}

#[async_trait]
impl Executor for CodeAssemblerExecutor {
    type Input = Value;      // MAF: dynamic dispatch
    type Message = Value;    // MAF: dynamic dispatch
    type Output = String;    // Final assembled code

    fn id(&self) -> &ExecutorId { &self.id }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Worker }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Deserialize dynamically
        let typed_input: AssemblyInput = serde_json::from_value(input)?;

        // Assemble code
        let code = self.assemble_parts(&typed_input);
        let assembled = AssembledCode::new(&code, &typed_input.language, typed_input.parts.len());

        // Send as Value to next executors
        ctx.send_message(assembled.to_value()).await?;

        // Yield the final code as workflow output
        ctx.yield_output(code).await?;

        Ok(())
    }
}

// === ORCHESTRATOR: Coordonne un sub-workflow ===
pub struct SubWorkflowExecutor {
    id: ExecutorId,
    workflow: Arc<Workflow<Value, String>>,  // Child workflow
    fail_on_child_failure: bool,
}

#[async_trait]
impl Executor for SubWorkflowExecutor {
    type Input = Value;      // MAF: dynamic dispatch
    type Message = Value;    // MAF: dynamic dispatch
    type Output = String;    // Status messages

    fn id(&self) -> &ExecutorId { &self.id }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Orchestrator }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Run the child workflow
        let result = self.workflow.run(input).await?;

        // Convert to output
        let output = SubWorkflowOutput::from_result(result);

        // Check for failure
        if self.fail_on_child_failure && !output.success {
            return Err(ExecutorError::new(self.id.clone(), output.error.unwrap_or_default()));
        }

        // Yield status
        ctx.yield_output(format!(
            "Sub-workflow: {} ({} outputs, {} supersteps)",
            if output.success { "COMPLETED" } else { "FAILED" },
            output.outputs.len(),
            output.superstep_count
        )).await?;

        // Send output as Value to next executors
        ctx.send_message(output.to_value()).await?;

        Ok(())
    }
}
```

### Type-Safe Construction Helpers

```rust
// Input helpers with builder pattern
let input = GeneratorInput::new("Write fibonacci function", "rust")
    .with_context("Must be iterative, not recursive")
    .with_errors(vec!["error[E0308]: type mismatch".into()])
    .to_value();  // → serde_json::Value

let compile_input = CompileInput::new(code, "rust")
    .with_task("Implement state machine")
    .to_value();

let assembly_input = AssemblyInput::new("rust")
    .add_part(CodePart::new("types", types_code, "rust").with_section("Type Definitions"))
    .add_part(CodePart::new("impl", impl_code, "rust").with_section("Implementation"))
    .to_value();

// Workflow usage
let workflow = WorkflowBuilder::<Value>::new("code-gen")
    .set_start("generator")
    .add_executor(LLMGeneratorExecutor::new("generator", llm))
    .add_executor(CompileValidatorExecutor::new("validator", sandbox))
    .add_direct_edge("generator", "validator")
    .build()?;

workflow.run(input).await?;
```

---

## Edges: Les Connexions Typées

Inspiré de [Microsoft Agent Framework](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/core-concepts/edges), nous supportons **5 types d'edges**:

### Les 5 Types d'Edges

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              EDGE TYPES                                     │
│                                                                             │
│  1. DIRECT           A ────────────────────▶ B                              │
│     (1-to-1)         Simple séquentiel                                      │
│                                                                             │
│  2. CONDITIONAL      A ───── if(cond) ─────▶ B                              │
│     (1-to-1)              sinon rien                                        │
│                                                                             │
│  3. SWITCH-CASE      A ───── match ────┬───▶ B  (si cond1)                  │
│     (1-to-N)                           ├───▶ C  (si cond2)                  │
│                                        └───▶ D  (default)                   │
│                                                                             │
│  4. FAN-OUT          A ────────────────┬───▶ B                              │
│     (1-to-N parallel)                  ├───▶ C  (tous en parallèle)         │
│                                        └───▶ D                              │
│                                                                             │
│  5. FAN-IN           A ───┐                                                 │
│     (N-to-1)         B ───┼────────────────▶ D  (agrégation)                │
│                      C ───┘                                                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Structure d'un Edge

```rust
/// Connexion typée entre deux Executors
pub struct Edge<T> {
    /// Identifiant unique
    pub id: EdgeId,
    /// Type d'edge
    pub edge_type: EdgeType,
    /// Executor source
    pub from: ExecutorId,
    /// Executor(s) destination
    pub to: EdgeTarget,
    /// Condition d'activation (pour Conditional/Switch)
    pub condition: Option<EdgeCondition<T>>,
    /// Métadonnées
    pub metadata: EdgeMetadata,
}

#[derive(Clone)]
pub enum EdgeType {
    /// Simple 1-to-1
    Direct,
    /// 1-to-1 avec condition
    Conditional,
    /// 1-to-N avec switch/match
    Switch,
    /// 1-to-N parallèle
    FanOut,
    /// N-to-1 agrégation
    FanIn,
}

#[derive(Clone)]
pub enum EdgeTarget {
    /// Un seul executor
    Single(ExecutorId),
    /// Plusieurs executors
    Multiple(Vec<ExecutorId>),
}

/// Condition pour edges conditionnels
pub struct EdgeCondition<T> {
    /// Fonction de condition
    pub predicate: Box<dyn Fn(&T) -> bool + Send + Sync>,
    /// Description (pour debug/logs)
    pub description: String,
}
```

### Exemples d'utilisation

```rust
// 1. Direct Edge
builder.add_edge(type_generator, impl_generator);

// 2. Conditional Edge - seulement si validation OK
builder.add_edge(
    compile_validator,
    test_validator,
    |result: &ValidationResult| result.passed,  // condition lambda
);

// 3. Switch-Case Edge - routing selon le type d'erreur
builder.add_switch_case_edge(
    error_analyzer,
    vec![
        Case::new(|e| e.is_type_error(), type_fixer),
        Case::new(|e| e.is_impl_error(), impl_fixer),
        Case::default(general_fixer),  // Default = quand aucune condition ne match
    ]
);

// 4. Fan-Out Edge - paralléliser la génération (tous les targets)
builder.add_fan_out_edge(
    task_splitter,
    vec![type_generator, impl_generator, test_generator],
);

// 5. Fan-Out Edge avec sélection dynamique (inspiré MAF selection_func)
builder.add_fan_out_edge_with_selection(
    priority_router,
    vec![fast_worker, medium_worker, slow_worker],
    |message: &Task, target_count: usize| -> Vec<usize> {
        // Retourne les indices des targets à activer
        match message.priority {
            Priority::High => vec![0],              // Juste fast_worker
            Priority::Normal => vec![0, 1],         // fast + medium
            Priority::Low => (0..target_count).collect(),  // Tous
        }
    }
);

// 6. Fan-In Edge - agréger les résultats
builder.add_fan_in_edge(
    vec![type_generator, impl_generator, test_generator],
    assembler,
);
```

### Fan-Out avec Sélection Dynamique

Le `selection_func` permet de choisir **dynamiquement** quels targets activer selon le message:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    FAN-OUT AVEC SÉLECTION                                   │
│                                                                             │
│                         ┌─────────────────────────────────────┐             │
│                         │         selection_func              │             │
│                         │  |msg, targets| -> Vec<usize>       │             │
│                         └──────────────┬──────────────────────┘             │
│                                        │                                    │
│                    Message             │                                    │
│                       │                ▼                                    │
│                       │     ┌─────────────────────┐                         │
│   ┌───────────┐       │     │   Priority::High    │──▶ [0]                  │
│   │  Router   │───────┼────▶│   Priority::Normal  │──▶ [0, 1]               │
│   └───────────┘       │     │   Priority::Low     │──▶ [0, 1, 2]            │
│                       │     └─────────────────────┘                         │
│                       │                │                                    │
│                       ▼                ▼                                    │
│              ┌────────────────────────────────────────┐                     │
│              │  [0]           [1]           [2]       │                     │
│              │  FastWorker    MediumWorker  SlowWorker│                     │
│              └────────────────────────────────────────┘                     │
│                                                                             │
│   High   → seulement FastWorker                                             │
│   Normal → FastWorker + MediumWorker                                        │
│   Low    → Tous les workers                                                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

```rust
/// Signature du selection_func
pub type SelectionFunc<T> = Box<dyn Fn(&T, usize) -> Vec<usize> + Send + Sync>;

impl WorkflowBuilder {
    /// Fan-Out avec sélection dynamique des targets
    pub fn add_fan_out_edge_with_selection<T, F>(
        &mut self,
        from: &ExecutorId,
        targets: Vec<ExecutorId>,
        selection_func: F,
    ) -> &mut Self
    where
        F: Fn(&T, usize) -> Vec<usize> + Send + Sync + 'static,
    {
        self.edges.push(Edge::fan_out_with_selection(
            from.clone(),
            targets,
            Box::new(selection_func),
        ));
        self
    }
}
```

### Edge avec NATS (persistance)

Chaque activation d'edge est persistée sur NATS:

```rust
impl<T: Serialize> Edge<T> {
    /// Active l'edge et transporte les données
    pub async fn activate(&self, data: T, nats: &NatsClient) -> Result<(), EdgeError> {
        // 1. Sérialiser et hasher les données
        let payload = serde_json::to_value(&data)?;
        let hash = sha256(&payload);

        // 2. Persister sur NATS KV
        nats.kv_put(&format!("edge.{}.data", self.id), &EdgeData {
            payload,
            hash: hash.clone(),
            timestamp: Utc::now(),
            source: self.from.clone(),
        }).await?;

        // 3. Publier l'événement
        nats.publish(
            &format!("agentic.edge.{}.activated", self.id),
            &EdgeEvent::Activated {
                edge_id: self.id.clone(),
                from: self.from.clone(),
                to: self.to.clone(),
                data_hash: hash,
            }
        ).await?;

        Ok(())
    }
}
```

---

## Workflow: Execution Model (Pregel/Supersteps)

Inspiré de [Google Pregel](https://research.google/pubs/pub37252/) et [Microsoft Agent Framework](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/core-concepts/workflows).

### Supersteps: Bulk Synchronous Parallel

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SUPERSTEP MODEL                                   │
│                                                                             │
│   Superstep 0          Superstep 1          Superstep 2                     │
│  ┌──────────┐         ┌──────────┐         ┌──────────┐                     │
│  │ TypeGen  │         │ ImplGen  │         │ Assembler│                     │
│  │ TestGen  │ ──────▶ │          │ ──────▶ │          │ ──────▶ ...         │
│  │(parallel)│ BARRIER │          │ BARRIER │          │ BARRIER             │
│  └──────────┘         └──────────┘         └──────────┘                     │
│                                                                             │
│  Chaque superstep:                                                          │
│  1. Collect: récupère messages du step précédent                            │
│  2. Route: envoie aux executors selon les edges                             │
│  3. Execute: TOUS les executors triggered en PARALLÈLE                      │
│  4. Barrier: ATTEND que tous finissent                                      │
│  5. Emit: nouveaux messages → superstep suivant                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Pourquoi les Supersteps?

| Avantage | Explication |
|----------|-------------|
| **Déterministe** | Même input → même ordre d'exécution |
| **Pas de race conditions** | Chaque superstep a une vue cohérente |
| **Checkpointing facile** | État sauvegardé aux frontières de superstep |
| **Parallélisme clair** | Dans un superstep = parallèle, entre supersteps = séquentiel |

### Implémentation Workflow

```rust
pub struct Workflow {
    /// Executors du workflow
    executors: HashMap<ExecutorId, Box<dyn Executor>>,
    /// Edges entre executors
    edges: Vec<Edge>,
    /// Executor de départ
    start_executor: ExecutorId,
    /// Client NATS
    nats: Arc<NatsClient>,
    /// Config
    config: WorkflowConfig,
}

impl Workflow {
    /// Exécute le workflow avec le modèle Superstep
    pub async fn run<T: Serialize>(&self, input: T) -> WorkflowResult {
        let mut superstep = 0;
        let mut pending_messages: Vec<Message> = vec![
            Message::new(self.start_executor.clone(), input)
        ];

        loop {
            // 1. COLLECT: messages pour ce superstep
            let messages = std::mem::take(&mut pending_messages);

            if messages.is_empty() {
                break; // Workflow terminé
            }

            // Log superstep sur NATS
            self.nats.publish("agentic.workflow.superstep", &SuperstepEvent {
                superstep,
                message_count: messages.len(),
            }).await?;

            // 2. ROUTE: grouper par executor destination
            let routed = self.route_messages(messages)?;

            // 3. EXECUTE: tous en parallèle
            let futures: Vec<_> = routed.into_iter().map(|(executor_id, msgs)| {
                self.execute_one(executor_id, msgs)
            }).collect();

            let results = futures::future::join_all(futures).await;

            // 4. BARRIER: on attend tous les résultats (implicite avec join_all)

            // 5. EMIT: collecter les nouveaux messages
            for result in results {
                match result? {
                    ExecutorResult::Output(data) => {
                        // Trouver les edges sortants et les activer
                        for edge in self.outgoing_edges(&result.executor_id) {
                            if edge.should_activate(&data) {
                                pending_messages.push(
                                    Message::new(edge.target(), data.clone())
                                );
                            }
                        }
                    }
                    ExecutorResult::Validation { passed, errors, .. } => {
                        // Router selon succès/échec
                        // ...
                    }
                    // ...
                }
            }

            // Checkpoint sur NATS
            self.checkpoint(superstep, &pending_messages).await?;

            superstep += 1;
        }

        self.collect_final_result().await
    }
}
```

### Workflow Events (Streaming)

Inspiré de MAF, le workflow émet des événements pendant l'exécution:

```rust
/// Événements émis pendant l'exécution du workflow
#[derive(Debug, Clone, Serialize)]
pub enum WorkflowEvent {
    /// Workflow démarré
    Started { workflow_id: WorkflowId, input_hash: String },

    /// Nouveau superstep
    SuperstepStarted { superstep: u32, executor_count: usize },

    /// Un executor a terminé
    ExecutorCompleted {
        executor_id: ExecutorId,
        superstep: u32,
        duration_ms: u64,
    },

    /// Un executor a échoué
    ExecutorFailed {
        executor_id: ExecutorId,
        error: String,
        will_retry: bool,
    },

    /// Output produit (via yield_output)
    Output { data: serde_json::Value },

    /// Workflow terminé
    Completed { duration_ms: u64, superstep_count: u32 },

    /// Workflow échoué
    Failed { error: String, last_superstep: u32 },
}
```

### run vs run_stream

```rust
impl Workflow {
    /// Exécution bloquante - attend la fin
    pub async fn run<T: Serialize>(&self, input: T) -> WorkflowResult {
        let events = self.run_stream(input).collect::<Vec<_>>().await;
        WorkflowResult::from_events(events)
    }

    /// Exécution streaming - émet les événements en temps réel
    pub fn run_stream<T: Serialize>(
        &self,
        input: T,
    ) -> impl Stream<Item = WorkflowEvent> + '_ {
        async_stream::stream! {
            yield WorkflowEvent::Started {
                workflow_id: self.id.clone(),
                input_hash: hash(&input),
            };

            let mut superstep = 0;
            let mut pending_messages = vec![Message::new(self.start_executor.clone(), input)];
            let mut outputs = vec![];

            loop {
                let messages = std::mem::take(&mut pending_messages);
                if messages.is_empty() { break; }

                yield WorkflowEvent::SuperstepStarted {
                    superstep,
                    executor_count: messages.len(),
                };

                // Exécuter le superstep...
                let results = self.execute_superstep(messages).await;

                for result in results {
                    match result {
                        Ok(exec_result) => {
                            yield WorkflowEvent::ExecutorCompleted {
                                executor_id: exec_result.executor_id,
                                superstep,
                                duration_ms: exec_result.duration_ms,
                            };

                            // Collecter les outputs (yield_output)
                            for output in exec_result.outputs {
                                yield WorkflowEvent::Output { data: output };
                                outputs.push(output);
                            }

                            // Collecter les messages (send_message)
                            pending_messages.extend(exec_result.messages);
                        }
                        Err(e) => {
                            yield WorkflowEvent::ExecutorFailed {
                                executor_id: e.executor_id,
                                error: e.message,
                                will_retry: e.retriable,
                            };
                        }
                    }
                }

                superstep += 1;
            }

            yield WorkflowEvent::Completed {
                duration_ms: start.elapsed().as_millis() as u64,
                superstep_count: superstep,
            };
        }
    }
}
```

### WorkflowResult

```rust
/// Résultat final d'un workflow
pub struct WorkflowResult {
    /// Tous les outputs produits (via yield_output)
    outputs: Vec<serde_json::Value>,
    /// Nombre de supersteps exécutés
    superstep_count: u32,
    /// Durée totale
    duration: Duration,
    /// Événements (pour debug/audit)
    events: Vec<WorkflowEvent>,
}

impl WorkflowResult {
    /// Récupère tous les outputs
    pub fn get_outputs(&self) -> &[serde_json::Value] {
        &self.outputs
    }

    /// Récupère le premier output (cas commun)
    pub fn get_output<T: DeserializeOwned>(&self) -> Option<T> {
        self.outputs.first().and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Workflow réussi?
    pub fn is_success(&self) -> bool {
        self.events.iter().any(|e| matches!(e, WorkflowEvent::Completed { .. }))
    }
}
```

### Exemple d'utilisation

```rust
// Mode bloquant
let result = workflow.run(task).await?;
let code: FinalCode = result.get_output().unwrap();
println!("Generated {} lines", code.lines().count());

// Mode streaming (afficher la progression)
let mut stream = workflow.run_stream(task);
while let Some(event) = stream.next().await {
    match event {
        WorkflowEvent::SuperstepStarted { superstep, .. } => {
            println!("⏳ Superstep {}...", superstep);
        }
        WorkflowEvent::ExecutorCompleted { executor_id, .. } => {
            println!("✅ {} completed", executor_id);
        }
        WorkflowEvent::Output { data } => {
            println!("📤 Output: {:?}", data);
        }
        WorkflowEvent::Completed { duration_ms, .. } => {
            println!("🎉 Done in {}ms", duration_ms);
        }
        _ => {}
    }
}
```

---

## Graph Persistence: Checkpoints Durables (PR #6)

> ✅ **STATUT: IMPLÉMENTÉ** - `crates/agentic-core/src/distributed/graph/persistence.rs`

Permet aux workflows de survivre aux crashes et de reprendre depuis le dernier checkpoint.

### Cas d'usage: Migration à grande échelle

```text
Vendredi 18:00: Démarrage migration de 500 services
    → Checkpoint sauvegardé au superstep 50
Samedi 02:00: Crash serveur (panne de courant)
Lundi 09:00: Redémarrage du workflow
    → Charge le checkpoint du superstep 50
    → Reprend exactement où il s'était arrêté
    → Pas de tokens LLM gaspillés, pas de progression perdue
```

### API Implémentée

```rust
// Backends disponibles
use agentic_core::distributed::graph::{
    InMemoryPersistentBackend,  // Pour tests
    RedisCheckpointBackend,      // Pour production (feature "redis-persistence")
    PersistentCheckpoint,
    PersistentCheckpointBackend,
};

// Connexion Redis
let backend = RedisCheckpointBackend::connect("redis://localhost:6379").await?
    .with_ttl_days(7);  // Expire après 7 jours

// Workflow avec persistence
let workflow = Workflow::with_config(definition, registry, config)
    .with_checkpoint_backend(Arc::new(backend));

// Exécution normale (checkpoints automatiques)
let result = workflow.run(input).await?;

// OU: Reprise depuis un checkpoint existant
let result = workflow.run_with_resume(Some("checkpoint-id")).await?;

// OU: Reprise automatique du dernier checkpoint
let result = workflow.run_with_resume(None).await?;
```

### PersistentCheckpoint

```rust
/// Checkpoint étendu avec état partagé pour récupération complète.
pub struct PersistentCheckpoint {
    pub checkpoint: Checkpoint,           // Données de base
    pub shared_state: HashMap<String, Value>,  // État partagé capturé
    pub schema_version: u32,              // Pour migrations
    pub workflow_hash: Option<String>,    // Détection changements incompatibles
}
```

### Cleanup automatique

```rust
// Garder seulement les 3 derniers checkpoints
backend.cleanup_keep_last(&workflow_id, &task_id, 3).await?;

// Supprimer tous les checkpoints d'une tâche
backend.delete_all(&workflow_id, &task_id).await?;

// Lister les checkpoints (metadata légère)
let checkpoints = backend.list_metadata(&workflow_id, None).await?;
```

---

## NATS: Le Système Nerveux Central

**Insight clé**: NATS n'est pas juste pour la persistence. C'est le **bus de communication** entre TOUS les composants.

```
                         ┌─────────────────────────────────────────┐
                         │              NATS BUS                   │
                         │                                         │
                         │  ┌─────────┐ ┌─────────┐ ┌───────────┐ │
                         │  │ Events  │ │  State  │ │  Audit    │ │
                         │  │ Stream  │ │  Store  │ │  Trail    │ │
                         │  │ (what   │ │  (KV:   │ │  (why     │ │
                         │  │ happened)│ │ current)│ │ decisions)│ │
                         │  └─────────┘ └─────────┘ └───────────┘ │
                         └──────────────────┬──────────────────────┘
                                            │
              ┌─────────────────────────────┼─────────────────────────────┐
              │                             │                             │
              ▼                             ▼                             ▼
       ┌─────────────┐              ┌─────────────────────────────────────────┐
       │ORCHESTRATOR │              │            TOUS LES EXECUTORS           │
       │             │              │  ┌─────────┐ ┌─────────┐ ┌───────────┐  │
       │ Reads:      │              │  │ Workers │ │Validators│ │Orchestrat.│  │
       │ - All events│              │  │(kind=W) │ │(kind=V)  │ │(kind=O)   │  │
       │ - State     │              │  └─────────┘ └─────────┘ └───────────┘  │
       │             │              │                                         │
       │ Writes:     │              │  Tous lisent/écrivent via WorkflowContext│
       │ - Dispatch  │              │  - ctx.previous_errors()                │
       │ - Decisions │              │  - ctx.send_message()                   │
       │ - Graph state│             │  - ctx.log()                            │
       └─────────────┘              └─────────────────────────────────────────┘
```

---

## Executors: Implémentation Détaillée

> **Rappel**: Worker, Validator et Orchestrator sont tous des **Executors** (même trait).
> La différence est sémantique: `kind()` retourne `Worker`, `Validator`, ou `Orchestrator`.

### État d'un Executor

```rust
#[derive(Clone)]
pub enum ExecutorState {
    /// En attente d'inputs (edges entrants pas encore activés)
    Waiting,
    /// En cours d'exécution
    Running { started_at: Instant },
    /// Terminé avec succès
    Completed { output_hash: String },
    /// Échoué
    Failed { error: String, attempts: u32 },
    /// Bloqué (dépendance échouée)
    Blocked { reason: String },
}
```

### Executor avec contexte NATS

```rust
impl GraphExecutor {
    /// Exécute l'executor avec contexte NATS
    pub async fn execute(&mut self, inputs: Vec<EdgeData>) -> Result<ExecutorOutput, ExecutorError> {
        // 1. LIRE le contexte depuis NATS
        let context = self.load_context_from_nats().await?;

        // 2. Publier: je commence
        self.nats.publish(&format!(
            "agentic.executor.{}.started", self.id
        ), &ExecutorEvent::Started {
            executor_id: self.id.clone(),
            inputs: inputs.iter().map(|i| i.id.clone()).collect(),
        }).await?;

        // 3. Exécuter selon le type
        let output = match self.executor_type {
            ExecutorType::TypeGenerator => self.generate_types(inputs, &context).await?,
            ExecutorType::ImplGenerator => self.generate_impl(inputs, &context).await?,
            ExecutorType::TestGenerator => self.generate_tests(inputs, &context).await?,
            ExecutorType::Assembler => self.assemble(inputs, &context).await?,
            ExecutorType::CompileValidator => self.validate_compile(inputs).await?,
            ExecutorType::TestValidator => self.validate_tests(inputs).await?,
            // ...
        };

        // 4. Sauvegarder output dans NATS
        self.nats.kv_put(&format!(
            "executor.{}.output", self.id
        ), &output).await?;

        // 5. Publier: j'ai fini
        self.nats.publish(&format!(
            "agentic.executor.{}.completed", self.id
        ), &ExecutorEvent::Completed {
            executor_id: self.id.clone(),
            output_summary: output.summary(),
        }).await?;

        Ok(output)
    }

    /// Charge le contexte depuis NATS
    async fn load_context_from_nats(&self) -> Result<ExecutorContext, ExecutorError> {
        // Mes attempts précédents
        let my_attempts: Vec<Attempt> = self.nats
            .kv_get(&format!("executor.{}.attempts", self.id))
            .await
            .unwrap_or_default();

        // Erreurs des attempts précédents
        let previous_errors: Vec<EnrichedError> = self.nats
            .kv_get(&format!("executor.{}.errors", self.id))
            .await
            .unwrap_or_default();

        // Outputs des executors dont je dépends (via edges)
        let mut dependency_outputs = HashMap::new();
        for edge_id in &self.inputs {
            if let Some(edge_data) = self.nats.kv_get(&format!("edge.{}.data", edge_id)).await {
                dependency_outputs.insert(edge_id.clone(), edge_data);
            }
        }

        // Patterns qui ont marché (pour ce type d'executor)
        let successful_patterns: Vec<Pattern> = self.nats
            .kv_get(&format!("patterns.{:?}.success", self.executor_type))
            .await
            .unwrap_or_default();

        Ok(ExecutorContext {
            my_attempts,
            previous_errors,
            dependency_outputs,
            successful_patterns,
        })
    }
}
```

---

## Orchestrator: Le Chef d'Orchestre

L'Orchestrator gère le graph d'executors et d'edges.

```rust
pub struct Orchestrator {
    /// Le graph (executors + edges)
    graph: TaskGraph,
    /// Client NATS
    nats: Arc<NatsClient>,
    /// État global
    state: OrchestratorState,
    /// Décision engine
    decision_engine: DecisionEngine,
}

pub struct TaskGraph {
    /// Tous les executors
    executors: HashMap<ExecutorId, GraphExecutor>,
    /// Tous les edges
    edges: HashMap<EdgeId, Edge>,
    /// Index: executor -> edges sortants
    outgoing: HashMap<ExecutorId, Vec<EdgeId>>,
    /// Index: executor -> edges entrants
    incoming: HashMap<ExecutorId, Vec<EdgeId>>,
}

impl Orchestrator {
    /// Exécute le graph
    pub async fn run(&mut self, task: Task) -> Result<TaskResult, OrchestratorError> {
        // 1. Construire le graph pour cette tâche
        self.build_graph(&task).await?;

        // 2. Publier sur NATS: task started
        self.nats.publish("agentic.orchestrator.task.started", &task).await?;

        // 3. Boucle principale
        loop {
            // Trouver les executors prêts (tous les inputs disponibles)
            let ready = self.find_ready_executors();

            if ready.is_empty() {
                // Vérifier si on a terminé ou si on est bloqué
                if self.is_complete() {
                    break;
                }
                if self.is_deadlocked() {
                    return Err(OrchestratorError::Deadlock);
                }
                // Attendre un événement NATS
                self.wait_for_event().await?;
                continue;
            }

            // Exécuter les executors prêts EN PARALLÈLE
            let futures: Vec<_> = ready.iter().map(|id| {
                self.execute_node(id)
            }).collect();

            let results = futures::future::join_all(futures).await;

            // Traiter les résultats
            for result in results {
                match result {
                    Ok(output) => self.handle_success(output).await?,
                    Err(e) => self.handle_failure(e).await?,
                }
            }
        }

        // 4. Assembler le résultat final
        let final_result = self.collect_final_result().await?;

        // 5. Publier sur NATS: task completed
        self.nats.publish("agentic.orchestrator.task.completed", &final_result).await?;

        Ok(final_result)
    }

    /// Construit le graph pour une tâche
    async fn build_graph(&mut self, task: &Task) -> Result<(), OrchestratorError> {
        // Créer les executors
        let type_gen = self.create_executor(ExecutorType::TypeGenerator, task);
        let impl_gen = self.create_executor(ExecutorType::ImplGenerator, task);
        let test_gen = self.create_executor(ExecutorType::TestGenerator, task);
        let assembler = self.create_executor(ExecutorType::Assembler, task);
        let compile_val = self.create_executor(ExecutorType::CompileValidator, task);
        let test_val = self.create_executor(ExecutorType::TestValidator, task);

        // Créer les edges
        //
        //  TypeGen ──────────────────┐
        //     │                      │
        //     ▼                      ▼
        //  ImplGen ───────────▶ Assembler ──▶ CompileVal ──▶ TestVal
        //                            ▲
        //  TestGen ──────────────────┘
        //
        self.add_edge(&type_gen, &impl_gen, EdgeDataType::Code { language: "typescript".into() });
        self.add_edge(&type_gen, &assembler, EdgeDataType::Code { language: "typescript".into() });
        self.add_edge(&impl_gen, &assembler, EdgeDataType::Code { language: "typescript".into() });
        self.add_edge(&test_gen, &assembler, EdgeDataType::Code { language: "typescript".into() });
        self.add_edge(&assembler, &compile_val, EdgeDataType::Code { language: "typescript".into() });
        self.add_edge(&compile_val, &test_val, EdgeDataType::ValidationResult).on_success();

        // Edge de feedback (loop back)
        self.add_edge(&compile_val, &impl_gen, EdgeDataType::Feedback).on_failure();
        self.add_edge(&test_val, &impl_gen, EdgeDataType::Feedback).on_failure();

        Ok(())
    }
}
```

---

## NATS Subjects et Streams

### Subjects (pub/sub temps réel)

```
# === AGENT LAYER ===
agentic.agent.{id}.message            # Nouveau message utilisateur
agentic.agent.{id}.response           # Réponse de l'agent
agentic.agent.{id}.tool_call          # Appel d'outil
agentic.agent.{id}.workflow.started   # Workflow démarré par l'agent
agentic.agent.{id}.workflow.completed # Workflow terminé

# === WORKFLOW LAYER ===
agentic.workflow.{id}.started         # Workflow démarré
agentic.workflow.{id}.superstep       # Nouveau superstep
agentic.workflow.{id}.completed       # Workflow terminé

agentic.executor.{id}.started         # Executor démarre
agentic.executor.{id}.progress        # Progression
agentic.executor.{id}.completed       # Executor terminé
agentic.executor.{id}.failed          # Executor échoué

agentic.edge.{id}.activated           # Edge activé
agentic.edge.{id}.data                # Data sur l'edge

agentic.validator.compile.result      # Résultat compilation
agentic.validator.test.result         # Résultat tests
agentic.validator.feedback            # Feedback structuré
```

### KV Store (état courant)

```
# === AGENT LAYER ===
thread.{id}                           # Thread de conversation complet
thread.{id}.messages                  # Messages du thread
thread.{id}.metadata                  # Metadata (created_at, agent_id, etc.)

# === WORKFLOW LAYER ===
executor.{id}.state                   # État d'un executor
executor.{id}.output                  # Output d'un executor
executor.{id}.attempts                # Historique attempts
executor.{id}.errors                  # Erreurs accumulées

edge.{id}.data                        # Données sur un edge
edge.{id}.metadata                    # Metadata d'un edge

workflow.{id}.state                   # État du workflow
workflow.{id}.superstep               # Superstep courant
workflow.{id}.pending_messages        # Messages en attente

# === PATTERNS ===
patterns.{executor_type}.success      # Patterns qui marchent
patterns.{executor_type}.failure      # Patterns qui échouent
```

### Streams (historique persisté)

```
AGENTIC_EVENTS          # Tous les événements (pour replay)
AGENTIC_AUDIT           # Audit trail (décisions, raisons)
AGENTIC_METRICS         # Métriques de performance
```

---

## Flow Complet: Exemple TypeScript

```
1. USER: "Create a TypeScript state machine"
   │
   ▼
2. ORCHESTRATOR
   ├─ Publie: task.started
   ├─ Construit graph:
   │   TypeGen ──┬──▶ Assembler ──▶ CompileVal ──▶ TestVal
   │   ImplGen ──┤        ▲
   │   TestGen ──┴────────┘
   └─ Trouve ready: [TypeGen, TestGen] (pas de deps)
   │
   ▼
3. PARALLEL EXECUTION
   ┌─────────────────────────────────────────┐
   │ TypeGen                 │ TestGen       │
   │ - Lit NATS: context     │ - Lit NATS    │
   │ - Génère types          │ - Génère tests│
   │ - Publie: completed     │ - (INDÉPENDANT│
   │ - KV: output            │   du code!)   │
   └─────────────────────────────────────────┘
   │
   ▼
4. EDGES ACTIVATED
   ├─ Edge(TypeGen→ImplGen): Code
   ├─ Edge(TypeGen→Assembler): Code
   └─ Edge(TestGen→Assembler): Code
   │
   ▼
5. ORCHESTRATOR: ImplGen now ready
   │
   ▼
6. ImplGen
   ├─ Lit NATS: types générés (via edge)
   ├─ Lit NATS: mes erreurs précédentes
   ├─ Génère implémentation
   └─ Publie: completed
   │
   ▼
7. EDGE ACTIVATED: ImplGen→Assembler
   │
   ▼
8. Assembler (tous inputs prêts)
   ├─ Merge: types + impl + tests
   ├─ Exécute dans Sandbox
   └─ Publie: completed
   │
   ▼
9. CompileValidator
   ├─ tsc --noEmit
   ├─ Si OK: Edge→TestVal activé
   └─ Si KO: Edge→ImplGen (feedback) activé
   │
   ▼
10. TestValidator (si compile OK)
    ├─ jest
    ├─ Si OK: DONE
    └─ Si KO: Edge→ImplGen (feedback) activé
    │
    ▼
11. Si feedback edge activé:
    └─ ORCHESTRATOR: re-exécute ImplGen avec context enrichi
       (errors, suggestions dans NATS)
    │
    ▼
12. LOOP jusqu'à succès ou max retries
    │
    ▼
13. ORCHESTRATOR: Publie task.completed
    └─ NATS conserve TOUT l'historique
```

---

## Mapping avec composants existants

| Existant | Nouveau rôle |
|----------|--------------|
| `BusCoordinator` | Base pour Orchestrator |
| `Executor` (actuel) | Devient `GraphExecutor` avec NATS |
| `ValidatorPipeline` | Split en `CompileValidator`, `TestValidator`, `LintValidator` |
| `SandboxValidator` | Utilisé par `Assembler` |
| `Sequencer` | Scheduling dans Orchestrator |
| `Arbiter` | Selection best-of-N dans Orchestrator |
| `StateStore` | KV pour outputs/state |
| `ErrorFingerprinter` | Dans Validators pour feedback |
| `RollbackManager` | Rollback graph state |
| `PipelineTracer` | Tracing des edges/executors |
| `AuditCollector` | Audit trail des décisions |

---

## Orchestration Patterns (Prédéfinis)

> ⚠️ **STATUT: NON IMPLÉMENTÉ** - Ces patterns sont planifiés pour Phase 7.
> Actuellement, vous devez construire les patterns manuellement avec `WorkflowBuilder`.

Inspiré de [MAF Orchestrations](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/workflows/orchestrations/overview), nous proposons des **patterns de workflow prêts à l'emploi**:

### Les 5 Patterns

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        ORCHESTRATION PATTERNS                               │
│                                                                             │
│  1. SEQUENTIAL         A ────▶ B ────▶ C ────▶ D                            │
│     Pipeline           Chaîne linéaire, chaque agent passe au suivant       │
│                                                                             │
│  2. CONCURRENT         ┌────▶ B ────┐                                       │
│     Fan-out/in     A ──┼────▶ C ────┼──▶ Aggregator                         │
│                        └────▶ D ────┘                                       │
│                        Tous en parallèle, résultats agrégés                 │
│                                                                             │
│  3. GROUP CHAT              ┌─────┐                                         │
│     Star + Manager    ┌─────│ MGR │─────┐     Manager contrôle qui parle    │
│                       │     └──┬──┘     │                                   │
│                       ▼        │        ▼                                   │
│                      [A]      [B]      [C]                                  │
│                                                                             │
│  4. MAGENTIC               ┌──────────┐                                     │
│     Planner-based          │ PLANNER  │     Planificateur décompose         │
│                            └────┬─────┘     et assigne dynamiquement        │
│                       ┌─────────┼─────────┐                                 │
│                       ▼         ▼         ▼                                 │
│                      [A]       [B]       [C]                                │
│                                                                             │
│  5. HANDOFF            A ◄───▶ B                                            │
│     Mesh dynamic       │       │         Agents se passent le contrôle      │
│                        ▼       ▼         dynamiquement (sans manager)       │
│                        C ◄───▶ D                                            │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Pattern: Sequential

Pipeline linéaire où chaque agent passe son résultat au suivant. Inspiré de [MAF SequentialBuilder](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/workflows/orchestrations/sequential).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SEQUENTIAL FLOW                                   │
│                                                                             │
│   Input                                                                     │
│     │                                                                       │
│     ▼                                                                       │
│  ┌──────────┐    messages    ┌──────────┐    messages    ┌──────────┐      │
│  │  Writer  │ ─────────────▶ │ Reviewer │ ─────────────▶ │ Polisher │      │
│  └──────────┘                └──────────┘                └──────────┘      │
│                                                                             │
│  Chaque agent reçoit l'HISTORIQUE COMPLET des messages précédents          │
│  → Writer voit: [user_input]                                                │
│  → Reviewer voit: [user_input, writer_response]                             │
│  → Polisher voit: [user_input, writer_response, reviewer_response]          │
└─────────────────────────────────────────────────────────────────────────────┘
```

#### SequentialBuilder (API fluent)

```rust
/// Builder pour orchestration séquentielle
pub struct SequentialBuilder {
    participants: Vec<Box<dyn Executor>>,
}

impl SequentialBuilder {
    pub fn new() -> Self {
        Self { participants: vec![] }
    }

    /// Ajoute les participants dans l'ordre d'exécution
    pub fn participants(mut self, agents: Vec<impl Executor + 'static>) -> Self {
        self.participants = agents.into_iter().map(|a| Box::new(a) as _).collect();
        self
    }

    /// Construit le workflow
    pub fn build(self) -> Workflow {
        let mut builder = WorkflowBuilder::new();

        // Premier participant = start
        if let Some(first) = self.participants.first() {
            builder.set_start_executor(first.clone());
        }

        // Chaîner: A → B → C → D
        for window in self.participants.windows(2) {
            builder.add_edge(&window[0].id(), &window[1].id());
        }

        builder.build()
    }
}
```

#### Usage simple

```rust
// Créer les agents
let writer = llm.as_agent(
    "You are a concise copywriter. Provide a punchy marketing sentence.",
    "writer"
);
let reviewer = llm.as_agent(
    "You are a thoughtful reviewer. Give brief feedback on the previous message.",
    "reviewer"
);

// Construire le workflow séquentiel
let workflow = SequentialBuilder::new()
    .participants(vec![writer, reviewer])
    .build();

// Exécuter
let result = workflow.run("Write a tagline for a budget-friendly eBike.").await?;

// Résultat: conversation complète
for msg in result.get_outputs::<Vec<ChatMessage>>()? {
    println!("[{}]: {}", msg.author_name, msg.text);
}
// [user]: Write a tagline...
// [writer]: "Ride Far, Spend Less – Your eBike Adventure Awaits!"
// [reviewer]: Great tagline! Punchy and clear. Maybe add emotion?
```

#### Avec transformateur custom entre étapes

```rust
/// Executor qui transforme/résume entre les étapes
pub struct Summarizer {
    id: ExecutorId,
}

#[async_trait]
impl Executor for Summarizer {
    type Input = Vec<ChatMessage>;
    type Message = Vec<ChatMessage>;
    type Output = Never;

    async fn handle(
        &self,
        conversation: Vec<ChatMessage>,
        ctx: &mut WorkflowContext<Vec<ChatMessage>>,
    ) -> Result<(), ExecutorError> {
        // Compter les messages par rôle
        let user_count = conversation.iter().filter(|m| m.role == Role::User).count();
        let assistant_count = conversation.iter().filter(|m| m.role == Role::Assistant).count();

        // Ajouter un résumé
        let mut updated = conversation.clone();
        updated.push(ChatMessage::assistant(format!(
            "Summary: {} user messages, {} assistant responses",
            user_count, assistant_count
        )));

        ctx.send_message(updated).await?;
        Ok(())
    }
}

// Workflow: content → summarizer
let workflow = SequentialBuilder::new()
    .participants(vec![
        content_agent,
        Summarizer::new("summarizer"),
    ])
    .build();
```

#### Early exit (optionnel)

```rust
/// Executor qui peut arrêter le pipeline si condition remplie
pub struct ConditionalGate {
    id: ExecutorId,
    condition: Box<dyn Fn(&Vec<ChatMessage>) -> bool + Send + Sync>,
}

#[async_trait]
impl Executor for ConditionalGate {
    type Input = Vec<ChatMessage>;
    type Message = Vec<ChatMessage>;
    type Output = Vec<ChatMessage>;  // Peut yield_output pour early exit

    async fn handle(
        &self,
        messages: Vec<ChatMessage>,
        ctx: &mut WorkflowContext<Vec<ChatMessage>, Vec<ChatMessage>>,
    ) -> Result<(), ExecutorError> {
        if (self.condition)(&messages) {
            // Condition remplie → early exit, retourne le résultat
            ctx.yield_output(messages).await?;
        } else {
            // Continuer le pipeline
            ctx.send_message(messages).await?;
        }
        Ok(())
    }
}

// Usage: s'arrêter si le reviewer approuve
let workflow = SequentialBuilder::new()
    .participants(vec![
        writer,
        reviewer,
        ConditionalGate::new("gate", |msgs| {
            msgs.last().map(|m| m.text.contains("approved")).unwrap_or(false)
        }),
        final_polisher,  // Skip si approved
    ])
    .build();
```

### Pattern: Concurrent

Fan-out parallèle avec agrégation des résultats. Inspiré de [MAF ConcurrentBuilder](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/workflows/orchestrations/concurrent).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           CONCURRENT FLOW                                   │
│                                                                             │
│                              Input                                          │
│                                │                                            │
│                    ┌───────────┼───────────┐                                │
│                    ▼           ▼           ▼                                │
│              ┌──────────┐ ┌──────────┐ ┌──────────┐                         │
│              │Researcher│ │ Marketer │ │  Legal   │  ← Parallel execution   │
│              └────┬─────┘ └────┬─────┘ └────┬─────┘                         │
│                   │            │            │                               │
│                   └────────────┼────────────┘                               │
│                                ▼                                            │
│                    ┌───────────────────────┐                                │
│                    │      Aggregator       │  ← Combine results             │
│                    │  (default ou custom)  │                                │
│                    └───────────────────────┘                                │
│                                │                                            │
│                                ▼                                            │
│                        Final Output                                         │
└─────────────────────────────────────────────────────────────────────────────┘
```

#### ConcurrentBuilder (API fluent)

```rust
/// Builder pour orchestration concurrent
pub struct ConcurrentBuilder {
    participants: Vec<Box<dyn Executor>>,
    aggregator: Option<AggregatorFn>,
}

/// Fonction d'agrégation custom
pub type AggregatorFn = Box<dyn Fn(Vec<ExecutorResult>) -> BoxFuture<'static, AggregatedResult> + Send + Sync>;

impl ConcurrentBuilder {
    pub fn new() -> Self {
        Self { participants: vec![], aggregator: None }
    }

    /// Ajoute les participants (agents ou executors)
    pub fn participants(mut self, agents: Vec<impl Executor + 'static>) -> Self {
        self.participants = agents.into_iter().map(|a| Box::new(a) as _).collect();
        self
    }

    /// Configure un agrégateur custom (optionnel)
    /// Par défaut: collecte tous les résultats dans une liste
    pub fn with_aggregator<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(Vec<ExecutorResult>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = AggregatedResult> + Send + 'static,
    {
        self.aggregator = Some(Box::new(move |results| Box::pin(f(results))));
        self
    }

    /// Construit le workflow
    pub fn build(self) -> Workflow {
        let splitter = BroadcastExecutor::new(self.participants.len());
        let aggregator = match self.aggregator {
            Some(f) => CustomAggregator::new(f),
            None => DefaultAggregator::new(),
        };

        WorkflowBuilder::new()
            .set_start_executor(splitter)
            .add_executors(self.participants)
            .add_executor(aggregator)
            .add_fan_out_edge(&splitter.id(), self.participant_ids())
            .add_fan_in_edge(self.participant_ids(), &aggregator.id())
            .build()
    }
}
```

#### Usage simple

```rust
// Créer les agents
let researcher = llm.as_agent("You're an expert market researcher...", "researcher");
let marketer = llm.as_agent("You're a creative marketing strategist...", "marketer");
let legal = llm.as_agent("You're a legal/compliance reviewer...", "legal");

// Construire le workflow concurrent
let workflow = ConcurrentBuilder::new()
    .participants(vec![researcher, marketer, legal])
    .build();

// Exécuter
let result = workflow.run("We are launching a new electric bike...").await?;

// Résultat: liste de toutes les réponses
for response in result.get_outputs::<Vec<AgentResponse>>()? {
    println!("[{}]: {}", response.agent_name, response.text);
}
```

#### Avec agrégateur custom (synthèse LLM)

```rust
// Agrégateur qui synthétise avec un LLM
let workflow = ConcurrentBuilder::new()
    .participants(vec![researcher, marketer, legal])
    .with_aggregator(|results: Vec<ExecutorResult>| async move {
        // Collecter les réponses de chaque expert
        let mut sections = Vec::new();
        for result in results {
            match result {
                Ok(response) => {
                    sections.push(format!("{}:\n{}", response.executor_id, response.text));
                }
                Err(e) => {
                    // Gestion erreur partielle
                    sections.push(format!("{}: (error: {})", e.executor_id, e.message));
                }
            }
        }

        // Synthétiser avec LLM
        let prompt = format!(
            "Consolidate these expert opinions into one cohesive summary:\n\n{}",
            sections.join("\n\n")
        );

        let summary = llm.generate("You are a helpful assistant...", &prompt).await?;

        Ok(AggregatedResult::Text(summary))
    })
    .build();

// Le résultat est maintenant une synthèse unique
let result = workflow.run("We are launching a new electric bike...").await?;
println!("Summary: {}", result.get_output::<String>()?);
```

#### Gestion des erreurs partielles

```rust
// L'agrégateur reçoit Ok et Err, peut décider quoi faire
.with_aggregator(|results| async move {
    let (successes, failures): (Vec<_>, Vec<_>) = results
        .into_iter()
        .partition(|r| r.is_ok());

    if failures.len() > successes.len() {
        // Trop d'échecs, abandon
        return Err(AggregationError::TooManyFailures(failures.len()));
    }

    // Continuer avec les succès seulement
    let valid_responses: Vec<_> = successes.into_iter().filter_map(|r| r.ok()).collect();
    Ok(AggregatedResult::Partial {
        responses: valid_responses,
        failed_count: failures.len(),
    })
})
```

### Pattern: Group Chat

Topologie en étoile avec un orchestrateur qui contrôle le flux de conversation. Inspiré de [MAF GroupChatBuilder](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/workflows/orchestrations/group-chat).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           GROUP CHAT FLOW                                   │
│                                                                             │
│                        ┌───────────────────┐                                │
│                        │   ORCHESTRATOR    │                                │
│                        │  (selection_func) │                                │
│                        └─────────┬─────────┘                                │
│                   ┌──────────────┼──────────────┐                           │
│                   │              │              │                           │
│                   ▼              ▼              ▼                           │
│            ┌──────────┐  ┌──────────┐  ┌──────────┐                         │
│            │Researcher│  │  Writer  │  │ Reviewer │                         │
│            └────┬─────┘  └────┬─────┘  └────┬─────┘                         │
│                 │             │             │                               │
│                 └─────────────┴─────────────┘                               │
│                               │                                             │
│                               ▼                                             │
│                    Broadcast response to ALL                                │
│                    (context synchronization)                                │
│                                                                             │
│  L'orchestrateur:                                                           │
│  1. Sélectionne qui parle (selection_func ou LLM)                          │
│  2. L'agent sélectionné répond                                              │
│  3. La réponse est broadcastée à TOUS les agents                           │
│  4. Répète jusqu'à termination_condition                                    │
└─────────────────────────────────────────────────────────────────────────────┘
```

#### GroupChatBuilder (API fluent)

```rust
/// Builder pour orchestration Group Chat
pub struct GroupChatBuilder {
    participants: Vec<Box<dyn Executor>>,
    selection_func: Option<SelectionFunc>,
    orchestrator_agent: Option<Box<dyn Agent>>,
    termination_condition: Option<TerminationFunc>,
    max_iterations: Option<usize>,
}

/// Fonction de sélection: décide qui parle
pub type SelectionFunc = Box<dyn Fn(&GroupChatState) -> ExecutorId + Send + Sync>;

/// Condition de terminaison
pub type TerminationFunc = Box<dyn Fn(&[ChatMessage]) -> bool + Send + Sync>;

/// État du group chat
pub struct GroupChatState {
    pub participants: HashMap<ExecutorId, Box<dyn Executor>>,
    pub conversation: Vec<ChatMessage>,
    pub current_round: usize,
}

impl GroupChatBuilder {
    pub fn new() -> Self { ... }

    /// Ajoute les participants
    pub fn participants(mut self, agents: Vec<impl Executor + 'static>) -> Self {
        self.participants = agents.into_iter().map(|a| Box::new(a) as _).collect();
        self
    }

    /// Configure l'orchestrateur avec une fonction de sélection
    pub fn with_orchestrator_func<F>(mut self, selection_func: F) -> Self
    where
        F: Fn(&GroupChatState) -> ExecutorId + Send + Sync + 'static,
    {
        self.selection_func = Some(Box::new(selection_func));
        self
    }

    /// Configure l'orchestrateur avec un agent LLM
    pub fn with_orchestrator_agent(mut self, agent: impl Agent + 'static) -> Self {
        self.orchestrator_agent = Some(Box::new(agent));
        self
    }

    /// Configure la condition de terminaison
    pub fn with_termination_condition<F>(mut self, condition: F) -> Self
    where
        F: Fn(&[ChatMessage]) -> bool + Send + Sync + 'static,
    {
        self.termination_condition = Some(Box::new(condition));
        self
    }

    /// Configure le nombre max d'itérations
    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = Some(max);
        self
    }

    pub fn build(self) -> Workflow { ... }
}
```

#### Stratégies de sélection

```rust
/// Round-Robin: alterne entre les agents
pub fn round_robin_selector(state: &GroupChatState) -> ExecutorId {
    let names: Vec<_> = state.participants.keys().collect();
    names[state.current_round % names.len()].clone()
}

/// Smart Selector: basé sur le contenu
pub fn smart_selector(state: &GroupChatState) -> ExecutorId {
    let last_message = state.conversation.last();

    match last_message {
        None => ExecutorId::new("researcher"),  // Commencer par researcher
        Some(msg) => {
            if msg.text.to_lowercase().contains("research complete") {
                ExecutorId::new("writer")  // Passer au writer
            } else if msg.author_name == "writer" {
                ExecutorId::new("reviewer")  // Puis reviewer
            } else {
                ExecutorId::new("researcher")  // Continuer recherche
            }
        }
    }
}

/// LLM-based: le LLM décide qui parle
pub fn llm_selector(llm: Arc<dyn LLMAdapter>) -> impl Fn(&GroupChatState) -> ExecutorId {
    move |state: &GroupChatState| {
        let prompt = format!(
            "Given the conversation so far, who should speak next?\n\
             Participants: {:?}\n\
             Last message: {:?}\n\
             Reply with just the participant name.",
            state.participants.keys().collect::<Vec<_>>(),
            state.conversation.last()
        );

        // Note: en pratique, utiliser async
        let response = llm.generate_sync("You are a conversation coordinator.", &prompt)?;
        ExecutorId::new(&response.trim())
    }
}
```

#### Usage avec Round-Robin

```rust
let researcher = llm.as_agent("You gather facts concisely.", "researcher");
let writer = llm.as_agent("You write clear structured responses.", "writer");

let workflow = GroupChatBuilder::new()
    .participants(vec![researcher, writer])
    .with_orchestrator_func(round_robin_selector)
    .with_termination_condition(|msgs| msgs.len() >= 4)
    .build();

let result = workflow.run("What are the benefits of async/await?").await?;
```

#### Usage avec orchestrateur LLM (intelligent)

```rust
let orchestrator = llm.as_agent(
    "You coordinate a team conversation to solve tasks.\n\
     Guidelines:\n\
     - Start with Researcher to gather info\n\
     - Then have Writer synthesize\n\
     - End when both have contributed",
    "orchestrator"
);

let workflow = GroupChatBuilder::new()
    .participants(vec![researcher, writer, reviewer])
    .with_orchestrator_agent(orchestrator)
    .with_termination_condition(|msgs| {
        // Terminer si le reviewer approuve
        msgs.last()
            .map(|m| m.text.to_lowercase().contains("approved"))
            .unwrap_or(false)
    })
    .with_max_iterations(10)  // Safety limit
    .build();
```

#### Synchronisation du contexte

```rust
/// L'orchestrateur broadcast chaque réponse à tous les participants
impl GroupChatOrchestrator {
    async fn broadcast_to_all(&self, message: &ChatMessage, ctx: &mut WorkflowContext<...>) {
        for participant_id in self.participants.keys() {
            ctx.send_message(BroadcastMessage {
                target: participant_id.clone(),
                message: message.clone(),
            }).await?;
        }
    }
}
```

#### Cas d'usage

| Cas | Configuration |
|-----|---------------|
| **Code Review** | `[coder, reviewer]` + round-robin + "approved" termination |
| **Brainstorming** | `[creative, analyst, critic]` + LLM orchestrator |
| **Q&A Research** | `[researcher, synthesizer]` + smart selector |
| **Content Creation** | `[writer, editor, fact-checker]` + max 6 iterations |

### Pattern: Handoff (Mesh Dynamique)

Les agents se passent le contrôle dynamiquement sans manager central.

```rust
/// Agent capable de faire un handoff
pub trait HandoffCapable: Executor {
    /// Décide à qui passer le contrôle (ou None si terminé)
    fn decide_handoff(&self, result: &Self::Message) -> Option<ExecutorId>;
}

/// Crée un workflow Handoff (mesh)
pub fn handoff(agents: Vec<impl HandoffCapable>) -> Workflow {
    let mut builder = WorkflowBuilder::new()
        .set_start_executor(agents[0]);

    // Chaque agent peut passer à n'importe quel autre
    for from in &agents {
        for to in &agents {
            if from.id() != to.id() {
                builder.add_edge(from.id(), to.id(),
                    |result| from.decide_handoff(result) == Some(to.id())
                );
            }
        }
    }

    builder.build()
}

// Usage: agents spécialisés qui s'entraident
let workflow = handoff(vec![
    general_assistant,      // Répond aux questions simples
    code_specialist,        // Handoff si question de code
    math_specialist,        // Handoff si question de math
    research_specialist,    // Handoff si recherche nécessaire
]);
```

### Pattern: Magentic (Planner-based)

Inspiré de [MagenticOne](https://www.microsoft.com/en-us/research/articles/magentic-one-a-generalist-multi-agent-system-for-solving-complex-tasks/), un planificateur décompose la tâche.

```rust
/// Planificateur qui décompose les tâches
pub struct MagenticPlanner {
    id: ExecutorId,
    llm: Arc<dyn LLMAdapter>,
    available_agents: Vec<AgentCapability>,
}

#[async_trait]
impl Executor for MagenticPlanner {
    type Input = ComplexTask;
    type Message = SubTask;
    type Output = TaskResult;

    async fn handle(&self, task: ComplexTask, ctx: &mut WorkflowContext<SubTask, TaskResult>) {
        // 1. Décomposer la tâche en sous-tâches
        let plan = self.create_plan(&task).await?;

        ctx.add_event(PlanCreatedEvent { steps: plan.len() }).await?;

        // 2. Exécuter le plan
        for step in plan {
            // Assigner à l'agent approprié
            ctx.send_message(SubTask {
                agent_id: step.assigned_agent,
                instruction: step.instruction,
                context: step.context,
            }).await?;
        }

        // 3. Collecter et synthétiser les résultats
        // (géré par les edges de retour)
    }
}
```

### Quand utiliser quel pattern?

| Pattern | Cas d'usage | Exemple |
|---------|-------------|---------|
| **Sequential** | Pipeline linéaire, étapes dépendantes | Parse → Generate → Validate → Format |
| **Concurrent** | Tâches indépendantes, agrégation | Analyser un doc avec 3 analyseurs |
| **Group Chat** | Collaboration itérative, débat | Code review avec plusieurs reviewers |
| **Magentic** | Tâches complexes, planification | "Crée une app complète" |
| **Handoff** | Escalade, spécialistes | Support client avec experts |

---

## Prochaines Étapes

### ✅ Phase 1: Core Abstractions (COMPLÉTÉ - PR #1-3)
- [x] `Executor` trait avec `WorkflowContext`
- [x] `Edge` enum (Direct, Conditional, Switch, FanOut, FanIn)
- [x] `Workflow` struct avec exécution Superstep

### ✅ Phase 2: WorkflowBuilder (COMPLÉTÉ - PR #4)
- [x] API fluent pour construction du graph
- [x] Validation du graph (types, connectivité)
- [x] Intégration NATS pour persistence

### ✅ Phase 3: Superstep Execution (COMPLÉTÉ - PR #5)
- [x] Exécution BSP (Bulk Synchronous Parallel)
- [x] Checkpointing aux frontières de superstep
- [x] `WorkflowConfig` (max_supersteps, max_retries)
- [x] `WorkflowResult` avec métriques

### ✅ Phase 4: Graph Persistence (COMPLÉTÉ - PR #6)
- [x] `PersistentCheckpoint` avec shared state
- [x] `InMemoryPersistentBackend` pour tests
- [x] `RedisCheckpointBackend` pour production
- [x] `Workflow.run_with_resume()` pour reprise
- [x] Cleanup automatique des vieux checkpoints

### ✅ Phase 5: Executors de base (COMPLÉTÉ)
- [x] Executor trait générique
- [x] **MAF Dynamic Dispatch Pattern** - tous les executors utilisent `serde_json::Value` pour Input/Message
- [x] `LLMGeneratorExecutor` (Worker) - génération LLM via `executors/llm_generator.rs`
- [x] `CodeAssemblerExecutor` (Worker) - merge du code via `executors/code_assembler.rs`
- [x] `CompileValidatorExecutor` (Validator) - compilation via `executors/compile_validator.rs`
- [x] `TestValidatorExecutor` (Validator) - tests via `executors/test_validator.rs`
- [x] `SubWorkflowExecutor` (Orchestrator) - nested workflows via `executors/sub_workflow.rs`

### ✅ Phase 6: Agent Layer (IMPLÉMENTÉ)
- [x] `Agent` trait avec `run()` et `run_stream()` - `agent/trait_def.rs`
- [x] `AgentThread` in-memory (NATS KV déféré à Phase 8) - `agent/thread.rs`
- [x] `ChatAgent` - agent simple (wrapper LLM) - `agent/chat_agent.rs`
- [x] `WorkflowAgent` - agent qui invoque des workflows - `agent/workflow_agent.rs`
- [x] `workflow.as_agent()` - conversion workflow → agent - `WorkflowAsAgent` trait
- [x] `Tool`, `ToolParam`, `ToolParameters` - définitions pour function calling - `agent/tools.rs`
- [x] `AgentChunk` streaming response - `agent/response.rs`
- [x] `AgentError`, `AgentResult` - gestion d'erreurs - `agent/error.rs`
- [x] `AgentExt` - extensions: `invoke()`, `run_once()`, `new_thread()` - `agent/trait_def.rs`

### ✅ Phase 7: Orchestration Patterns (IMPLÉMENTÉ)
- [x] `SequentialBuilder` - pipeline linéaire - `patterns/sequential.rs`
- [x] `ConcurrentBuilder` - fan-out/fan-in parallèle - `patterns/concurrent.rs`
- [x] `GroupChatBuilder` - topologie étoile avec orchestrateur - `patterns/group_chat.rs`
- [x] `handoff()` - mesh dynamique entre agents - `patterns/handoff.rs`
- [x] Selectors: `round_robin_selector`, `smart_selector`, `no_repeat_selector`
- [x] Termination: `max_messages`, `max_rounds`, `keyword`, `any_termination`
- [ ] `MagenticPlanner` - planificateur (déféré Phase 8)

### ✅ Phase 8: Agent Memory (FAIT - P1)
- [x] `MemoryProvider` trait - `agent/memory.rs`
- [x] `InMemoryProvider` - implémentation in-memory
- [x] `NoOpMemoryProvider` - implémentation no-op
- [x] `Memory`, `MemoryCategory`, `MemoryContext` types
- [x] `UserMemories` avec retrieval par importance
- [x] Short-term memory (Thread) - in-memory
- [ ] `AgentThread` persisté sur NATS KV (déféré Phase 9)
- [ ] `NatsMemoryProvider` - NATS KV persistence (déféré Phase 9)
- [x] `ChatReducer` trait - `agent/reducer.rs`
- [x] `MessageCountingReducer` - garde N derniers messages
- [x] `TokenCountingReducer` - budget tokens
- [x] `SlidingWindowReducer` - récent + important
- [x] `NoOpReducer` - pas de réduction
- [x] Example: `examples/memory_demo.rs`
- [x] 17 tests (8 memory + 9 reducer)

### ❌ Phase 9: Production Features (À FAIRE - P2)
- [ ] `NatsMemoryProvider` - NATS KV persistence
- [ ] `AgentThread` persisté sur NATS KV
- [ ] `ContinuationToken` pour tâches longues
- [ ] Streaming avec reprise
- [ ] Polling pattern
- [ ] `ProxyAgent` (agents distants via A2A)

### ❌ Phase 10: Tests E2E (À FAIRE)
- [x] `graph_workflow` - exemple basique de workflow
- [x] `graph_persistence_test` - test de crash/resume
- [x] `graph_refactor` - LLM refactoring avec persistence
- [ ] Exemple TypeScript state machine complet
- [ ] Benchmarks vs architecture linéaire actuelle

---

## WorkflowBuilder: Construction du Graph

```rust
pub struct WorkflowBuilder {
    executors: HashMap<ExecutorId, Box<dyn Executor>>,
    edges: Vec<Edge>,
    start_executor: Option<ExecutorId>,
}

impl WorkflowBuilder {
    pub fn new() -> Self { ... }

    /// Définit l'executor de départ
    pub fn set_start_executor(&mut self, executor: impl Executor) -> &mut Self {
        let id = executor.id().clone();
        self.executors.insert(id.clone(), Box::new(executor));
        self.start_executor = Some(id);
        self
    }

    /// Ajoute un executor
    pub fn add_executor(&mut self, executor: impl Executor) -> &mut Self {
        self.executors.insert(executor.id().clone(), Box::new(executor));
        self
    }

    /// Edge direct: A → B
    pub fn add_edge(&mut self, from: &ExecutorId, to: &ExecutorId) -> &mut Self {
        self.edges.push(Edge::direct(from.clone(), to.clone()));
        self
    }

    /// Edge conditionnel: A → B si condition
    pub fn add_conditional_edge<T, F>(
        &mut self,
        from: &ExecutorId,
        to: &ExecutorId,
        condition: F,
        description: &str,
    ) -> &mut Self
    where
        F: Fn(&T) -> bool + Send + Sync + 'static,
    {
        self.edges.push(Edge::conditional(from.clone(), to.clone(), condition, description));
        self
    }

    /// Switch-case: A → B|C|D selon conditions
    pub fn add_switch_edge<T>(
        &mut self,
        from: &ExecutorId,
        cases: Vec<Case<T>>,
    ) -> &mut Self {
        self.edges.push(Edge::switch(from.clone(), cases));
        self
    }

    /// Fan-out: A → [B, C, D] en parallèle
    pub fn add_fan_out_edge(&mut self, from: &ExecutorId, targets: Vec<ExecutorId>) -> &mut Self {
        self.edges.push(Edge::fan_out(from.clone(), targets));
        self
    }

    /// Fan-in: [A, B, C] → D agrégation
    pub fn add_fan_in_edge(&mut self, sources: Vec<ExecutorId>, to: &ExecutorId) -> &mut Self {
        self.edges.push(Edge::fan_in(sources, to.clone()));
        self
    }

    /// Construit le workflow
    pub fn build(self, nats: Arc<NatsClient>) -> Result<Workflow, BuildError> {
        // Validation:
        // - Type compatibility entre edges
        // - Tous les executors atteignables depuis start
        // - Pas de cycles invalides
        self.validate()?;
        Ok(Workflow::new(self.executors, self.edges, self.start_executor.unwrap(), nats))
    }
}
```

### Exemple complet: TypeScript State Machine

```rust
// Créer les executors
let type_gen = CodeGenerator::new("type_gen", llm.clone(), TYPE_SYSTEM_PROMPT);
let impl_gen = CodeGenerator::new("impl_gen", llm.clone(), IMPL_SYSTEM_PROMPT);
let test_gen = CodeGenerator::new("test_gen", llm.clone(), TEST_SYSTEM_PROMPT);
let assembler = CodeAssembler::new("assembler");
let compile_val = CompileValidator::new("compile_val", sandbox.clone());
let test_val = TestValidator::new("test_val", sandbox.clone());

// Construire le workflow
let workflow = WorkflowBuilder::new()
    .set_start_executor(type_gen)
    .add_executor(impl_gen)
    .add_executor(test_gen)
    .add_executor(assembler)
    .add_executor(compile_val)
    .add_executor(test_val)

    // Fan-out: générer types et tests en parallèle
    .add_fan_out_edge(&"type_gen".into(), vec!["impl_gen".into(), "assembler".into()])

    // test_gen démarre aussi (pas de dépendance - génération INDÉPENDANTE)
    .add_edge(&"test_gen".into(), &"assembler".into())

    // impl dépend de types
    .add_edge(&"impl_gen".into(), &"assembler".into())

    // Fan-in: assembler reçoit types + impl + tests
    // (implicite via les edges ci-dessus)

    // Validation pipeline
    .add_edge(&"assembler".into(), &"compile_val".into())

    // Conditional: test seulement si compile OK
    .add_conditional_edge(
        &"compile_val".into(),
        &"test_val".into(),
        |r: &ValidationResult| r.passed,
        "on_compile_success"
    )

    // Feedback loop: si échec, retour à impl_gen
    .add_conditional_edge(
        &"compile_val".into(),
        &"impl_gen".into(),
        |r: &ValidationResult| !r.passed,
        "on_compile_failure"
    )
    .add_conditional_edge(
        &"test_val".into(),
        &"impl_gen".into(),
        |r: &ValidationResult| !r.passed,
        "on_test_failure"
    )

    .build(nats)?;

// Exécuter
let result = workflow.run(GenerationRequest {
    task: "Create a TypeScript state machine".to_string(),
    language: "typescript".to_string(),
}).await?;
```

---

## Références

### Frameworks
- [Microsoft Agent Framework](https://learn.microsoft.com/en-us/agent-framework/) - Architecture Executor/Edge/Workflow
- [MAF Agent Types](https://learn.microsoft.com/fr-fr/agent-framework/user-guide/agents/agent-types/) - ChatAgent, BaseAgent, Proxies
- [MAF Workflows](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/) - Workflow orchestration
- [LangGraph](https://docs.langchain.com/oss/python/langgraph/) - Graph-based agent orchestration
- [Google Pregel](https://research.google/pubs/pub37252/) - Bulk Synchronous Parallel model

### Recherche
- [AgentCoder](https://arxiv.org/abs/2312.13010) - Multi-agent avec tests indépendants
- [LLMLOOP](https://valerio-terragni.github.io/assets/pdf/ravi-icsme-2025.pdf) - Boucles spécialisées
- [TDD for Code Generation](https://arxiv.org/abs/2402.13521) - Tests d'abord

### Patterns
- [Temporal Workflows](https://temporal.io/blog/building-an-agentic-system-thats-actually-production-ready) - Durability patterns
