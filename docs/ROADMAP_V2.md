

# Feuille de Route V2 - Agentic Framework Rust

> **Vision**: Transformer le framework actuel en une architecture **Graph Multi-Agent** complète, inspirée de Microsoft Agent Framework, avec Executors, Edges, et Supersteps.

---

## Analyse de l'État Actuel

### Ce qui est FAIT (Phase 0-3 complètes)

| Composant | État | LOC | Remarques |
|-----------|------|-----|-----------|
| **agentic-core** | ✅ Complet | ~3500 | Agent, Types, Protocol, Error |
| **agentic-llm** | ✅ Complet | ~1300 | OpenAI, Anthropic, Gemini, Ollama |
| **agentic-sandbox** | ✅ Complet | ~1200 | Process, Docker, Firecracker, Remote |
| **agentic-bus** | ✅ Complet | ~400 | NATS pub/sub, request/reply |
| **agentic-storage** | ✅ Complet | ~500 | Redis, Qdrant |
| **agentic-mcp** | ✅ Complet | ~500 | MCP Code Mode (98.7% token reduction) |
| **agentic-orchestrator** | ✅ Basique | ~400 | Workflow séquentiel, DAG |
| **distributed** | ✅ Avancé | ~8800 | Supervisor, Executor, Validators, RepairEngine |

### Fonctionnalités Avancées Existantes

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         DÉJÀ IMPLÉMENTÉ                                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ✅ Grounded Validation Loop     ✅ Error Fingerprinting                     │
│     (SandboxValidator)              (Simple→Fast, Complex→Smart model)       │
│                                                                              │
│  ✅ Progressive Locking          ✅ MCP Code Mode                            │
│     (Types→Stubs→Logic)             (Token-efficient tool calls)             │
│                                                                              │
│  ✅ Repair Engine                ✅ Task Decomposer                          │
│     (Auto-correction)               (Complex→Subtasks)                       │
│                                                                              │
│  ✅ Incremental Pipeline         ✅ Coherence Checking                       │
│     (Streaming validation)          (LiaisonArchitect)                       │
│                                                                              │
│  ✅ 17 Examples                  ✅ 39 Tests                                 │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Ce qui MANQUE (vs GRAPH_ARCHITECTURE.md)

| Composant | État | Priorité | Complexité |
|-----------|------|----------|------------|
| **Executor Trait unifié** | ❌ À faire | P0 | Moyenne |
| **Edge Types** (5 types) | ❌ À faire | P0 | Moyenne |
| **WorkflowBuilder** (graph fluent API) | ❌ À faire | P0 | Haute |
| **Superstep Execution** (Pregel/BSP) | ❌ À faire | P1 | Haute |
| **Agent Trait** (run/run_stream) | ❌ À faire | P1 | Moyenne |
| **AgentThread** (conversation persistée) | ❌ À faire | P1 | Faible |
| **Workflow.as_agent()** | ❌ À faire | P2 | Moyenne |
| **Orchestration Patterns** | ❌ À faire | P2 | Haute |
| **Background Responses** | ❌ À faire | P3 | Moyenne |
| **Agent Memory** (long-term) | ❌ À faire | P3 | Moyenne |

---

## Nouvelle Architecture Cible

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        ARCHITECTURE CIBLE V2                                 │
│                                                                              │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                         AGENT LAYER                                    │  │
│  │   ChatAgent ←→ WorkflowAgent ←→ ProxyAgent                            │  │
│  │        │              │              │                                 │  │
│  │        └──────────────┴──────────────┘                                 │  │
│  │                       │ invoke workflow                                │  │
│  └───────────────────────┼───────────────────────────────────────────────┘  │
│                          ▼                                                   │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                       WORKFLOW LAYER                                   │  │
│  │                                                                        │  │
│  │   ┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐        │  │
│  │   │Executor │─edge│Executor │─edge│Executor │─edge│Executor │        │  │
│  │   │(Worker) │     │(Validat)│     │(Orchest)│     │(Worker) │        │  │
│  │   └─────────┘     └─────────┘     └─────────┘     └─────────┘        │  │
│  │                                                                        │  │
│  │   Superstep 0      Superstep 1     Superstep 2    Superstep 3        │  │
│  │   ═══════════════════════════════════════════════════════════════     │  │
│  └───────────────────────┬───────────────────────────────────────────────┘  │
│                          ▼                                                   │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                         NATS LAYER                                     │  │
│  │   Events + State + Audit + History + Shared State + Request/Reply     │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                          ▼                                                   │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                       EXISTING LAYER                                   │  │
│  │   LLM Adapters │ Sandbox │ Storage │ MCP │ RepairEngine               │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Plan de Migration (8 semaines)

### Phase 5: Graph Core (3 semaines)

> **Objectif**: Implémenter les abstractions fondamentales du graph

#### Semaine 1: Executor Trait Unifié

**Fichiers à créer:**
- `crates/agentic-core/src/graph/mod.rs`
- `crates/agentic-core/src/graph/executor.rs`
- `crates/agentic-core/src/graph/context.rs`

```rust
// Structure cible: WorkflowContext avec 3 méthodes de communication
impl<TMessage, TOutput> WorkflowContext<TMessage, TOutput> {
    /// Envoie aux edges sortants (interne)
    pub async fn send_message(&mut self, msg: TMessage) -> Result<()>;

    /// Produit un output visible par le caller (externe)
    pub async fn yield_output(&mut self, out: TOutput) -> Result<()>;

    /// Émet un événement custom (observabilité)
    pub async fn add_event<E: WorkflowEvent>(&self, evt: E) -> Result<()>;

    /// Accède à l'historique des attempts précédents
    pub fn previous_errors(&self) -> &[ExecutorError];

    /// Shared state via NATS KV
    pub async fn set_shared_state<T>(&self, key: &str, value: T) -> Result<()>;
    pub async fn get_shared_state<T>(&self, key: &str) -> Result<Option<T>>;
}
```

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 5.1.1 | Définir `Executor` trait avec `Input`, `Message`, `Output` | `executor.rs` |
| 5.1.2 | Implémenter `WorkflowContext` | `context.rs` |
| 5.1.3 | Définir `ExecutorKind` enum (Worker, Validator, Orchestrator) | `executor.rs` |
| 5.1.4 | Mapper l'existant `distributed::Executor` vers le nouveau trait | Migration |
| 5.1.5 | Implémenter macro `#[executor]` pour executors déclaratifs | `executor.rs` |
| 5.1.6 | Tests unitaires Executor | `tests/graph/executor_test.rs` |

#### Semaine 2: Edge Types

**Fichiers à créer:**
- `crates/agentic-core/src/graph/edge.rs`
- `crates/agentic-core/src/graph/condition.rs`

```
5 TYPES D'EDGES À IMPLÉMENTER:

1. Direct        A ───────────────────▶ B
2. Conditional   A ─── if(cond) ──────▶ B
3. Switch-Case   A ─── match ────┬────▶ B (cond1)
                                 ├────▶ C (cond2)
                                 └────▶ D (default)
4. Fan-Out       A ──────────────┬────▶ B
                                 ├────▶ C (parallèle)
                                 └────▶ D
5. Fan-In        A ───┐
                 B ───┼──────────────▶ D (agrégation)
                 C ───┘
```

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 5.2.1 | Définir `Edge<T>` struct avec `EdgeType` enum | `edge.rs` |
| 5.2.2 | Implémenter `EdgeCondition<T>` avec closures | `condition.rs` |
| 5.2.3 | Implémenter `EdgeTarget` (Single, Multiple) | `edge.rs` |
| 5.2.4 | Implémenter Fan-Out avec `selection_func` (sélection dynamique) | `edge.rs` |
| 5.2.5 | Persistence des edges sur NATS KV | `edge.rs` |
| 5.2.6 | Tests unitaires Edge | `tests/graph/edge_test.rs` |

#### Semaine 3: WorkflowBuilder & Supersteps

**Fichiers à créer:**
- `crates/agentic-core/src/graph/builder.rs`
- `crates/agentic-core/src/graph/workflow.rs`
- `crates/agentic-core/src/graph/superstep.rs`

```rust
// API cible: WorkflowBuilder fluent
let workflow = WorkflowBuilder::new()
    .set_start_executor(type_gen)
    .add_executor(impl_gen)
    .add_executor(compile_val)
    .add_edge(&type_gen.id(), &impl_gen.id())                    // Direct
    .add_conditional_edge(&compile_val.id(), &test_val.id(),     // Conditional
        |r: &ValidationResult| r.passed, "on_success")
    .add_conditional_edge(&compile_val.id(), &impl_gen.id(),
        |r: &ValidationResult| !r.passed, "on_failure")
    .add_fan_out_edge(&splitter.id(), vec![a.id(), b.id(), c.id()])
    .add_fan_in_edge(vec![a.id(), b.id(), c.id()], &aggregator.id())
    .build(nats)?;
```

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 5.3.1 | Implémenter `WorkflowBuilder` avec API fluent | `builder.rs` |
| 5.3.2 | Implémenter validation du graph (types, connectivité) | `builder.rs` |
| 5.3.3 | Implémenter `Workflow::run()` avec Supersteps | `workflow.rs` |
| 5.3.4 | Implémenter `Workflow::run_stream()` avec events | `workflow.rs` |
| 5.3.5 | Implémenter `WorkflowEvent` enum complet | `workflow.rs` |
| 5.3.6 | Implémenter `WorkflowResult` avec outputs | `workflow.rs` |
| 5.3.7 | Checkpointing aux frontières de superstep | `superstep.rs` |
| 5.3.8 | Tests intégration workflow complet | `tests/graph/workflow_test.rs` |

### Phase 6: Agent Layer (2 semaines)

> **Objectif**: Implémenter la couche Agent avec conversation et threading

#### Semaine 4: Agent Trait & ChatAgent

**Fichiers à créer:**
- `crates/agentic-core/src/agent/trait.rs`
- `crates/agentic-core/src/agent/chat_agent.rs`
- `crates/agentic-core/src/agent/thread.rs`

```rust
// Structure cible: Agent trait
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> &AgentId;

    async fn run(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError>;

    fn run_stream(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Pin<Box<dyn Stream<Item = AgentChunk> + Send>>;

    fn tools(&self) -> &[Tool];
}
```

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 6.1.1 | Définir `Agent` trait avec `run()` et `run_stream()` | `trait.rs` |
| 6.1.2 | Implémenter `AgentThread` avec persistence NATS | `thread.rs` |
| 6.1.3 | Implémenter `ChatAgent` (wrapper LLM simple) | `chat_agent.rs` |
| 6.1.4 | Implémenter `ChatMessage` serialization/deserialization | `thread.rs` |
| 6.1.5 | Implémenter `AgentChunk` pour streaming | `trait.rs` |
| 6.1.6 | Tests ChatAgent | `tests/agent/chat_agent_test.rs` |

#### Semaine 5: WorkflowAgent & as_agent()

**Fichiers à créer:**
- `crates/agentic-core/src/agent/workflow_agent.rs`
- `crates/agentic-core/src/graph/as_agent.rs`

```rust
// Structure cible: workflow.as_agent()
impl Workflow {
    /// Convertit ce workflow en Agent
    pub fn as_agent(self, name: &str) -> WorkflowAsAgent {
        WorkflowAsAgent { id: AgentId::new(name), workflow: self }
    }
}

// Structure cible: WorkflowAgent
pub struct WorkflowAgent {
    llm: Arc<dyn LLMAdapter>,
    workflow: Workflow,
    // Détecte automatiquement si la tâche nécessite un workflow
}
```

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 6.2.1 | Implémenter `WorkflowAgent` (détection auto workflow) | `workflow_agent.rs` |
| 6.2.2 | Implémenter `Workflow::as_agent()` conversion | `as_agent.rs` |
| 6.2.3 | Implémenter `llm.as_agent()` pour wrapping auto | `chat_agent.rs` |
| 6.2.4 | Implémenter `AgentExecutor` (agents auto-wrappés dans workflows) | `agent_executor.rs` |
| 6.2.5 | Implémenter `AgentExecutorResponse` | `agent_executor.rs` |
| 6.2.6 | Tests WorkflowAgent | `tests/agent/workflow_agent_test.rs` |
| 6.2.7 | Example: `examples/agent_workflow.rs` | Example |

### Phase 7: Orchestration Patterns (2 semaines)

> **Objectif**: Implémenter les 5 patterns d'orchestration MAF

#### Semaine 6: Sequential & Concurrent

**Fichiers à créer:**
- `crates/agentic-core/src/patterns/mod.rs`
- `crates/agentic-core/src/patterns/sequential.rs`
- `crates/agentic-core/src/patterns/concurrent.rs`

```rust
// Pattern Sequential
let workflow = SequentialBuilder::new()
    .participants(vec![writer, reviewer, polisher])
    .build();

// Pattern Concurrent
let workflow = ConcurrentBuilder::new()
    .participants(vec![researcher, marketer, legal])
    .with_aggregator(|results| async { /* synthèse LLM */ })
    .build();
```

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 7.1.1 | Implémenter `SequentialBuilder` | `sequential.rs` |
| 7.1.2 | Implémenter `ConcurrentBuilder` avec aggregator | `concurrent.rs` |
| 7.1.3 | Tests Sequential pattern | `tests/patterns/sequential_test.rs` |
| 7.1.4 | Tests Concurrent pattern | `tests/patterns/concurrent_test.rs` |
| 7.1.5 | Example: `examples/sequential_pipeline.rs` | Example |

#### Semaine 7: GroupChat & Handoff

**Fichiers à créer:**
- `crates/agentic-core/src/patterns/group_chat.rs`
- `crates/agentic-core/src/patterns/handoff.rs`
- `crates/agentic-core/src/patterns/magentic.rs`

```rust
// Pattern GroupChat
let workflow = GroupChatBuilder::new()
    .participants(vec![researcher, writer, reviewer])
    .with_orchestrator_func(round_robin_selector)
    .with_termination_condition(|msgs| msgs.len() >= 6)
    .build();

// Pattern Handoff
let workflow = handoff(vec![
    general_assistant,
    code_specialist,
    math_specialist,
]);
```

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 7.2.1 | Implémenter `GroupChatBuilder` avec selection_func | `group_chat.rs` |
| 7.2.2 | Implémenter `round_robin_selector` | `group_chat.rs` |
| 7.2.3 | Implémenter `smart_selector` (basé contenu) | `group_chat.rs` |
| 7.2.4 | Implémenter `llm_selector` (LLM décide qui parle) | `group_chat.rs` |
| 7.2.5 | Implémenter `TerminationFunc` (condition d'arrêt) | `group_chat.rs` |
| 7.2.6 | Implémenter `handoff()` pattern | `handoff.rs` |
| 7.2.7 | Implémenter `MagenticBuilder` avec planner | `magentic.rs` |
| 7.2.8 | Tests tous patterns | `tests/patterns/` |

### Phase 8: Production Features (1 semaine)

> **Objectif**: Finaliser avec memory, background responses, et observabilité

#### Semaine 8: Memory & Polish

**Fichiers à créer:**
- `crates/agentic-core/src/agent/memory.rs`
- `crates/agentic-core/src/agent/background.rs`

| Tâche | Description | Livrable |
|-------|-------------|----------|
| 8.1 | Implémenter `MemoryProvider` trait | `memory.rs` |
| 8.2 | Implémenter `NatsMemoryProvider` | `memory.rs` |
| 8.3 | Implémenter `ChatReducer` (context window management) | `memory.rs` |
| 8.4 | Implémenter `ContinuationToken` pour background responses | `background.rs` |
| 8.5 | Request/Response human-in-the-loop (`ctx.request_info()`) | `context.rs` |
| 8.6 | Implémenter `ProxyAgent` (agents distants via A2A) | `proxy_agent.rs` |
| 8.7 | Documentation rustdoc complète | Tous les modules |
| 8.8 | Benchmarks graph vs linéaire | `benches/` |
| 8.9 | Example complet: TypeScript state machine | `examples/` |

---

## Mapping: Existant → Nouveau

| Existant | Devient | Notes |
|----------|---------|-------|
| `distributed::Executor` | `graph::Executor` trait | Refactor comme impl |
| `distributed::Validator` | `Executor` avec `kind = Validator` | Unifié |
| `distributed::Supervisor` | Base pour `Orchestrator` pattern | Réutilise la logique |
| `ValidatorPipeline` | Workflow avec edges conditionnels | Plus flexible |
| `SandboxValidator` | `Executor` qui utilise Sandbox | Intégration |
| `RepairEngine` | `Executor` de type Worker | Réutilise tel quel |
| `IncrementalPipeline` | `Workflow::run_stream()` | Migration |
| `BusCoordinator` | Intégré dans `WorkflowContext` | Réutilise |
| `StateStore` | `ctx.set_shared_state()` | Via NATS KV |
| `ErrorFingerprinter` | Utilisé dans conditional edges | Réutilise |

---

## Exemple Concret: Migration grounded_loop

### Avant (linéaire)
```rust
let result = supervisor
    .execute_with_validation(
        &task,
        &executor,
        Some(&validator_pipeline),
        ValidationMode::FailFast,
    )
    .await?;
```

### Après (graph)
```rust
let workflow = WorkflowBuilder::new()
    .set_start_executor(code_gen)
    .add_executor(compile_validator)
    .add_executor(test_validator)
    .add_executor(assembler)
    // Success path
    .add_edge(&code_gen.id(), &compile_validator.id())
    .add_conditional_edge(&compile_validator.id(), &test_validator.id(),
        |r| r.passed, "compile_success")
    .add_conditional_edge(&test_validator.id(), &assembler.id(),
        |r| r.passed, "test_success")
    // Failure paths (feedback loop)
    .add_conditional_edge(&compile_validator.id(), &code_gen.id(),
        |r| !r.passed, "compile_failure")
    .add_conditional_edge(&test_validator.id(), &code_gen.id(),
        |r| !r.passed, "test_failure")
    .build(nats)?;

let result = workflow.run(task).await?;
```

---

## Critères de Succès

| Phase | Métrique | Cible |
|-------|----------|-------|
| 5 | Workflow TypeScript state machine | Fonctionne end-to-end |
| 6 | ChatAgent conversation 10 turns | < 10ms overhead/turn |
| 7 | GroupChat 5 agents, 20 iterations | Converge en < 60s |
| 8 | Memory footprint idle | < 50MB par workflow |
| All | Test coverage | > 80% |
| All | Benchmarks vs linéaire | ≤ 20% overhead |

---

## Risques & Mitigations

| Risque | Probabilité | Impact | Mitigation |
|--------|-------------|--------|------------|
| Breaking changes existant | Élevé | Moyen | Feature flags, migration progressive |
| Complexité Supersteps | Moyen | Haut | Commencer simple, itérer |
| Performance edges | Faible | Moyen | Benchmark early, optimize hot paths |
| NATS saturation | Faible | Haut | Batching, déduplication existante |

---

## Timeline Résumé

```
Semaine 1  │ S2 │ S3 │ S4 │ S5 │ S6 │ S7 │ S8
───────────┼────┼────┼────┼────┼────┼────┼────
PHASE 5    │████│████│████│    │    │    │    │  Graph Core
PHASE 6    │    │    │    │████│████│    │    │  Agent Layer
PHASE 7    │    │    │    │    │    │████│████│  Patterns
PHASE 8    │    │    │    │    │    │    │████│  Polish

Effort: ~320h (8 semaines × 40h)
```

---

## Structure de Fichiers Finale

```
crates/agentic-core/src/
├── agent/
│   ├── mod.rs
│   ├── trait.rs           # NEW: Agent trait + AgentChunk
│   ├── chat_agent.rs      # NEW: ChatAgent
│   ├── workflow_agent.rs  # NEW: WorkflowAgent
│   ├── proxy_agent.rs     # NEW: ProxyAgent (agents distants A2A)
│   ├── agent_executor.rs  # NEW: AgentExecutor (wrapping auto)
│   ├── thread.rs          # NEW: AgentThread
│   ├── memory.rs          # NEW: MemoryProvider
│   └── background.rs      # NEW: ContinuationToken
├── graph/
│   ├── mod.rs             # NEW
│   ├── executor.rs        # NEW: Executor trait unifié
│   ├── edge.rs            # NEW: 5 types d'edges
│   ├── condition.rs       # NEW: EdgeCondition
│   ├── context.rs         # NEW: WorkflowContext
│   ├── builder.rs         # NEW: WorkflowBuilder
│   ├── workflow.rs        # NEW: Workflow avec Supersteps
│   ├── superstep.rs       # NEW: Superstep execution
│   └── as_agent.rs        # NEW: workflow.as_agent()
├── patterns/
│   ├── mod.rs             # NEW
│   ├── sequential.rs      # NEW: SequentialBuilder
│   ├── concurrent.rs      # NEW: ConcurrentBuilder
│   ├── group_chat.rs      # NEW: GroupChatBuilder + selectors
│   ├── selectors.rs       # NEW: round_robin, smart, llm_selector
│   ├── handoff.rs         # NEW: handoff()
│   └── magentic.rs        # NEW: MagenticBuilder
└── distributed/           # EXISTING (à migrer progressivement)
    ├── ...
```

---

## Prochaines Actions Immédiates

1. **Créer la branche** `feature/graph-architecture`
2. **Semaine 1, Jour 1**: Créer `crates/agentic-core/src/graph/mod.rs`
3. **Semaine 1, Jour 2**: Implémenter `Executor` trait de base
4. **Revue**: Valider le design avant d'aller plus loin

---

---

## Détails Additionnels (depuis GRAPH_ARCHITECTURE.md)

### Fan-Out avec Sélection Dynamique

```rust
// selection_func: choisit dynamiquement quels targets activer
builder.add_fan_out_edge_with_selection(
    priority_router,
    vec![fast_worker, medium_worker, slow_worker],
    |message: &Task, target_count: usize| -> Vec<usize> {
        match message.priority {
            Priority::High => vec![0],           // Juste fast_worker
            Priority::Normal => vec![0, 1],      // fast + medium
            Priority::Low => (0..target_count).collect(),  // Tous
        }
    }
);
```

### NATS Subjects Détaillés

```
# === AGENT LAYER ===
agentic.agent.{id}.message            # Nouveau message utilisateur
agentic.agent.{id}.response           # Réponse de l'agent
agentic.agent.{id}.tool_call          # Appel d'outil
agentic.agent.{id}.workflow.started   # Workflow démarré

# === WORKFLOW LAYER ===
agentic.workflow.{id}.started         # Workflow démarré
agentic.workflow.{id}.superstep       # Nouveau superstep
agentic.workflow.{id}.output          # Output (yield_output)
agentic.workflow.{id}.request.{req}   # Request/Response pattern

agentic.executor.{id}.started         # Executor démarre
agentic.executor.{id}.completed       # Executor terminé
agentic.executor.{id}.event.{type}    # Events custom (add_event)

agentic.edge.{id}.activated           # Edge activé

# === KV STORE ===
thread.{id}                           # Thread de conversation
executor.{id}.state                   # État executor
executor.{id}.output                  # Output executor
edge.{id}.data                        # Données edge
workflow.{id}.state.{key}             # Shared state
patterns.{type}.success               # Patterns réussis
```

### WorkflowEvent Enum

```rust
pub enum WorkflowEvent {
    Started { workflow_id: WorkflowId, input_hash: String },
    SuperstepStarted { superstep: u32, executor_count: usize },
    ExecutorCompleted { executor_id: ExecutorId, superstep: u32, duration_ms: u64 },
    ExecutorFailed { executor_id: ExecutorId, error: String, will_retry: bool },
    AgentResponseUpdate { executor_id: ExecutorId, chunk: String },  // Streaming
    AgentRunCompleted { executor_id: ExecutorId, response: AgentResponse },
    Output { data: serde_json::Value },  // yield_output
    Completed { duration_ms: u64, superstep_count: u32 },
    Failed { error: String, last_superstep: u32 },
}
```

### Agents Auto-Wrappés en Executors

```rust
// API simple: llm.as_agent() crée un AgentExecutor automatiquement
let writer = llm.as_agent("You are a content writer.", "writer");

// Utilisable directement dans WorkflowBuilder
let workflow = WorkflowBuilder::new()
    .set_start_executor(writer)   // Agent auto-wrappé en Executor
    .add_edge(&writer.id(), &reviewer.id())
    .build();

// Response structure
pub struct AgentExecutorResponse {
    pub executor_id: ExecutorId,
    pub agent_response: AgentResponse,
    pub full_conversation: Vec<ChatMessage>,
}
```

### ProxyAgent (Agents Distants)

```rust
/// Proxy pour agents distants (protocole A2A)
pub struct ProxyAgent {
    id: AgentId,
    remote_url: String,
    nats: Arc<NatsClient>,  // Communication inter-agents
}
```

### Background Responses (ContinuationToken)

```rust
pub struct ContinuationToken {
    pub task_id: TaskId,
    pub checkpoint: Vec<u8>,  // État sérialisé
    pub created_at: DateTime<Utc>,
}

pub enum AgentResponse<T> {
    Complete(T),
    InProgress {
        partial_result: Option<T>,
        continuation_token: ContinuationToken,
    },
}
```

### Stratégies de Sélection GroupChat

```rust
// Round-Robin
pub fn round_robin_selector(state: &GroupChatState) -> ExecutorId;

// Smart (basé contenu)
pub fn smart_selector(state: &GroupChatState) -> ExecutorId;

// LLM-based (le LLM décide)
pub fn llm_selector(llm: Arc<dyn LLMAdapter>) -> impl Fn(&GroupChatState) -> ExecutorId;
```

### Executor Macro (Déclaratif)

```rust
// Pattern @executor de MAF Python adapté à Rust
#[executor(id = "uppercase")]
async fn uppercase(text: String, ctx: &mut WorkflowContext<String>) {
    ctx.send_message(text.to_uppercase()).await;
}
```

---

> **Maintainer:** @rbometon
> **Créé:** Janvier 2026
> **Basé sur:** GRAPH_ARCHITECTURE.md + analyse codebase existante
