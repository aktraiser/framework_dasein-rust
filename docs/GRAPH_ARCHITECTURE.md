# Architecture Graph Multi-Agent

> **Vision**: Passer d'une architecture linéaire (Executor → Validator → Retry) à une architecture **graph dynamique** avec **Executors** (noeuds) et **Edges** (connexions).

## TL;DR

| Concept | Description | Inspiré de |
|---------|-------------|------------|
| **Executor** | Noeud générique: Worker, Validator, ou Orchestrator | [MAF Executors](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/core-concepts/executors) |
| **Edge** | Connexion typée: Direct, Conditional, Switch, FanOut, FanIn | [MAF Edges](https://learn.microsoft.com/en-us/agent-framework/user-guide/workflows/core-concepts/edges) |
| **Workflow** | Orchestration via Supersteps (Bulk Synchronous Parallel) | [Google Pregel](https://research.google/pubs/pub37252/) |
| **NATS** | Mémoire, historique, events (notre ajout) | - |

```
┌─────────────────────────────────────────────────────────────────┐
│                        NOTRE ARCHITECTURE                        │
│                                                                  │
│   Executor ─── Edge ─── Executor ─── Edge ─── Executor          │
│   (Worker)    (Direct)  (Validator) (Cond)   (Worker)           │
│       │                      │                   │               │
│       └──────────────────────┴───────────────────┘               │
│                              │                                   │
│                         NATS BUS                                 │
│                    (Events + State + Audit)                      │
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
| **Executor** | Noeud du graph. Abstraction générique (Worker, Validator, ou Orchestrator) |
| **Edge** | Connexion entre executors. Transporte data + metadata + conditions |
| **Graph** | L'ensemble. Peut être récursif (sub-graphs) |

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
    /// Type d'output produit
    type Output: Serialize + DeserializeOwned;

    /// Identifiant unique
    fn id(&self) -> &ExecutorId;

    /// Type d'executor (Worker, Validator, Orchestrator)
    fn kind(&self) -> ExecutorKind;

    /// Traite un message et utilise le contexte pour envoyer les résultats
    async fn handle(
        &self,
        input: Self::Input,
        ctx: &mut WorkflowContext<Self::Output>,
    ) -> Result<(), ExecutorError>;
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

### WorkflowContext: Communication

```rust
/// Contexte fourni à chaque Executor pour communiquer
pub struct WorkflowContext<TOutput> {
    /// ID de l'executor courant
    executor_id: ExecutorId,
    /// Client NATS pour mémoire/historique
    nats: Arc<NatsClient>,
    /// Messages à envoyer aux edges sortants
    outgoing_messages: Vec<TOutput>,
    /// Outputs à retourner au workflow caller
    outputs: Vec<WorkflowOutput>,
    /// Historique (lu depuis NATS)
    history: ExecutorHistory,
}

impl<TOutput: Serialize> WorkflowContext<TOutput> {
    /// Envoie un message aux executors connectés (via edges)
    pub async fn send_message(&mut self, message: TOutput) -> Result<(), ContextError> {
        self.outgoing_messages.push(message);
        Ok(())
    }

    /// Produit un output visible par le caller du workflow
    pub async fn yield_output<O: Serialize>(&mut self, output: O) -> Result<(), ContextError> {
        self.outputs.push(WorkflowOutput::new(output));
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

    /// Log un événement sur NATS
    pub async fn log(&self, level: LogLevel, message: &str) -> Result<(), ContextError> {
        self.nats.publish(
            &format!("agentic.executor.{}.log", self.executor_id),
            &LogEvent { level, message: message.to_string(), timestamp: Utc::now() }
        ).await?;
        Ok(())
    }
}
```

### Exemples d'Executors

```rust
// === WORKER: Génère du code ===
pub struct CodeGenerator {
    id: ExecutorId,
    llm: Arc<dyn LLMAdapter>,
    system_prompt: String,
}

#[async_trait]
impl Executor for CodeGenerator {
    type Input = GenerationRequest;
    type Output = GeneratedCode;

    fn id(&self) -> &ExecutorId { &self.id }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Worker }

    async fn handle(
        &self,
        input: GenerationRequest,
        ctx: &mut WorkflowContext<GeneratedCode>,
    ) -> Result<(), ExecutorError> {
        // Consulter l'historique pour éviter les mêmes erreurs
        let previous_errors = ctx.previous_errors();
        let prompt = self.build_prompt_with_context(&input, previous_errors);

        // Appeler le LLM
        let response = self.llm.generate(&self.system_prompt, &prompt).await?;

        // Envoyer le résultat aux edges sortants
        ctx.send_message(GeneratedCode {
            code: response.content,
            language: input.language,
        }).await?;

        Ok(())
    }
}

// === VALIDATOR: Vérifie la compilation ===
pub struct CompileValidator {
    id: ExecutorId,
    sandbox: Arc<dyn Sandbox>,
}

#[async_trait]
impl Executor for CompileValidator {
    type Input = CodeToValidate;
    type Output = ValidationResult;

    fn id(&self) -> &ExecutorId { &self.id }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Validator }

    async fn handle(
        &self,
        input: CodeToValidate,
        ctx: &mut WorkflowContext<ValidationResult>,
    ) -> Result<(), ExecutorError> {
        // Compiler dans le sandbox
        let result = self.sandbox.compile(&input.code, &input.language).await?;

        // Envoyer le résultat (les edges conditionnels routeront selon passed)
        ctx.send_message(ValidationResult {
            passed: result.success,
            errors: result.errors,
            feedback: self.build_feedback(&result),
        }).await?;

        Ok(())
    }
}

// === ORCHESTRATOR: Coordonne un sub-workflow ===
pub struct SubWorkflowOrchestrator {
    id: ExecutorId,
    sub_workflow: Workflow,
}

#[async_trait]
impl Executor for SubWorkflowOrchestrator {
    type Input = SubTaskRequest;
    type Output = SubTaskResult;

    fn id(&self) -> &ExecutorId { &self.id }
    fn kind(&self) -> ExecutorKind { ExecutorKind::Orchestrator }

    async fn handle(
        &self,
        input: SubTaskRequest,
        ctx: &mut WorkflowContext<SubTaskResult>,
    ) -> Result<(), ExecutorError> {
        // Exécuter le sub-workflow (récursif!)
        let result = self.sub_workflow.run(input).await?;

        // Envoyer le résultat agrégé
        ctx.send_message(SubTaskResult {
            outputs: result.outputs,
            passed: result.all_passed(),
        }).await?;

        Ok(())
    }
}
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
builder.add_conditional_edge(
    compile_validator,
    test_validator,
    |result| result.passed,
    "on_compile_success"
);

// 3. Switch-Case Edge - routing selon le type d'erreur
builder.add_switch_edge(
    error_analyzer,
    vec![
        Case::new(|e| e.is_type_error(), type_fixer),
        Case::new(|e| e.is_impl_error(), impl_fixer),
        Case::default(general_fixer),
    ]
);

// 4. Fan-Out Edge - paralléliser la génération
builder.add_fan_out_edge(
    task_splitter,
    vec![type_generator, test_generator]  // En parallèle!
);

// 5. Fan-In Edge - agréger les résultats
builder.add_fan_in_edge(
    vec![type_generator, impl_generator, test_generator],
    assembler
);
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
       ┌─────────────┐              ┌─────────────┐              ┌─────────────┐
       │ORCHESTRATOR │              │  EXECUTORS  │              │  VALIDATOR  │
       │             │              │             │              │             │
       │ Reads:      │              │ Reads:      │              │ Reads:      │
       │ - All events│              │ - Own past  │              │ - Attempts  │
       │ - State     │              │ - Deps output│             │ - Patterns  │
       │             │              │ - Errors    │              │             │
       │ Writes:     │              │             │              │ Writes:     │
       │ - Dispatch  │              │ Writes:     │              │ - Results   │
       │ - Decisions │              │ - Output    │              │ - Feedback  │
       │ - Graph state│             │ - Progress  │              │             │
       └─────────────┘              └─────────────┘              └─────────────┘
```

---

## Executors

### Définition

Un **Executor** est un noeud du graph qui:
1. Reçoit des inputs via ses **edges entrants**
2. Fait un travail (LLM call, merge, validation, etc.)
3. Produit des outputs via ses **edges sortants**
4. Consulte/publie sur **NATS** pour le contexte et l'historique

### Types d'Executors

```rust
/// Un Executor dans le graph
pub struct GraphExecutor {
    /// Identifiant unique
    pub id: ExecutorId,
    /// Type d'executor
    pub executor_type: ExecutorType,
    /// Configuration
    pub config: ExecutorConfig,
    /// Client NATS pour mémoire/communication
    pub nats: Arc<NatsClient>,
    /// Edges entrants (d'où viennent les inputs)
    pub inputs: Vec<EdgeId>,
    /// Edges sortants (où vont les outputs)
    pub outputs: Vec<EdgeId>,
    /// État courant
    pub state: ExecutorState,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutorType {
    // === Code Generation ===
    /// Génère types/interfaces
    TypeGenerator,
    /// Génère implémentation
    ImplGenerator,
    /// Génère tests (INDÉPENDAMMENT du code!)
    TestGenerator,
    /// Génère documentation
    DocGenerator,

    // === Processing ===
    /// Assemble les morceaux de code
    Assembler,
    /// Exécute dans sandbox
    SandboxRunner,

    // === Validation ===
    /// Boucle de compilation
    CompileValidator,
    /// Boucle de tests
    TestValidator,
    /// Boucle de lint
    LintValidator,

    // === Control ===
    /// Prend des décisions de routing
    Router,
    /// Agrège des résultats
    Aggregator,
}

#[derive(Clone)]
pub enum ExecutorState {
    /// En attente d'inputs
    Waiting,
    /// En cours d'exécution
    Running { started_at: Instant },
    /// Terminé avec succès
    Completed { output: ExecutorOutput },
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

## Edges

### Définition

Un **Edge** est une connexion entre deux executors qui:
1. Transporte des **data** (code, résultats, erreurs)
2. Porte des **metadata** (timestamps, provenance, validity)
3. Peut avoir des **conditions** (seulement si X réussi)
4. Est **persisté sur NATS** pour l'historique

### Structure

```rust
/// Une connexion entre deux executors
pub struct Edge {
    /// Identifiant unique
    pub id: EdgeId,
    /// Executor source
    pub from: ExecutorId,
    /// Executor destination
    pub to: ExecutorId,
    /// Type de données transportées
    pub data_type: EdgeDataType,
    /// Condition d'activation
    pub condition: Option<EdgeCondition>,
    /// Données actuelles (si activé)
    pub data: Option<EdgeData>,
    /// Métadonnées
    pub metadata: EdgeMetadata,
}

#[derive(Clone)]
pub enum EdgeDataType {
    /// Code source
    Code { language: String },
    /// Résultat de validation
    ValidationResult,
    /// Erreurs enrichies
    Errors,
    /// Feedback pour retry
    Feedback,
    /// Signal de contrôle
    Signal,
}

#[derive(Clone)]
pub enum EdgeCondition {
    /// Toujours actif
    Always,
    /// Seulement si source succeeded
    OnSuccess,
    /// Seulement si source failed
    OnFailure,
    /// Condition custom
    Custom(String),
}

#[derive(Clone)]
pub struct EdgeData {
    pub id: String,
    /// Payload
    pub payload: serde_json::Value,
    /// Hash pour déduplication
    pub content_hash: String,
    /// Timestamp
    pub created_at: DateTime<Utc>,
    /// Provenance
    pub source_executor: ExecutorId,
    /// Trace ID pour distributed tracing
    pub trace_id: String,
}

#[derive(Clone)]
pub struct EdgeMetadata {
    /// Nombre de fois que cet edge a été activé
    pub activation_count: u32,
    /// Dernière activation
    pub last_activated: Option<DateTime<Utc>>,
    /// Latence moyenne
    pub avg_latency_ms: f64,
}
```

### Edge persisté sur NATS

```rust
impl Edge {
    /// Active l'edge et transporte les données
    pub async fn activate(
        &mut self,
        data: EdgeData,
        nats: &NatsClient
    ) -> Result<(), EdgeError> {
        // 1. Sauvegarder les données sur NATS
        nats.kv_put(&format!("edge.{}.data", self.id), &data).await?;

        // 2. Publier l'événement d'activation
        nats.publish(&format!("agentic.edge.{}.activated", self.id), &EdgeEvent::Activated {
            edge_id: self.id.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
            data_hash: data.content_hash.clone(),
        }).await?;

        // 3. Mettre à jour metadata
        self.metadata.activation_count += 1;
        self.metadata.last_activated = Some(Utc::now());
        self.data = Some(data);

        Ok(())
    }

    /// Vérifie si l'edge doit être activé
    pub fn should_activate(&self, source_state: &ExecutorState) -> bool {
        match &self.condition {
            None | Some(EdgeCondition::Always) => true,
            Some(EdgeCondition::OnSuccess) => matches!(source_state, ExecutorState::Completed { .. }),
            Some(EdgeCondition::OnFailure) => matches!(source_state, ExecutorState::Failed { .. }),
            Some(EdgeCondition::Custom(expr)) => self.evaluate_condition(expr, source_state),
        }
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
agentic.orchestrator.task.started     # Nouvelle tâche
agentic.orchestrator.task.completed   # Tâche terminée
agentic.orchestrator.decision         # Décision prise

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
executor.{id}.state                   # État d'un executor
executor.{id}.output                  # Output d'un executor
executor.{id}.attempts                # Historique attempts
executor.{id}.errors                  # Erreurs accumulées

edge.{id}.data                        # Données sur un edge
edge.{id}.metadata                    # Metadata d'un edge

graph.{task_id}.state                 # État du graph
graph.{task_id}.executors             # Liste executors
graph.{task_id}.edges                 # Liste edges

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

## Prochaines Étapes

### Phase 1: Core Abstractions
- [ ] `Executor` trait avec `WorkflowContext`
- [ ] `Edge` enum (Direct, Conditional, Switch, FanOut, FanIn)
- [ ] `Workflow` struct avec exécution Superstep

### Phase 2: WorkflowBuilder
- [ ] API fluent pour construction du graph
- [ ] Validation du graph (types, connectivité)
- [ ] Intégration NATS pour persistence

### Phase 3: Executors de base
- [ ] `CodeGenerator` (Worker) - génération LLM
- [ ] `CodeAssembler` (Worker) - merge du code
- [ ] `CompileValidator` (Validator) - compilation
- [ ] `TestValidator` (Validator) - tests
- [ ] `SubWorkflowOrchestrator` (Orchestrator) - sub-graphs

### Phase 4: NATS Integration
- [ ] Persistence des edges/outputs dans KV
- [ ] Events pub/sub pour monitoring
- [ ] Historique et replay
- [ ] Checkpointing aux frontières de superstep

### Phase 5: Feedback & Learning
- [ ] Context enrichment depuis NATS (erreurs passées)
- [ ] Pattern learning (ce qui marche/échoue)
- [ ] Retry intelligent avec historique

### Phase 6: Tests E2E
- [ ] Exemple TypeScript state machine
- [ ] Exemple Rust async cache
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
- [LangGraph](https://docs.langchain.com/oss/python/langgraph/) - Graph-based agent orchestration
- [Google Pregel](https://research.google/pubs/pub37252/) - Bulk Synchronous Parallel model

### Recherche
- [AgentCoder](https://arxiv.org/abs/2312.13010) - Multi-agent avec tests indépendants
- [LLMLOOP](https://valerio-terragni.github.io/assets/pdf/ravi-icsme-2025.pdf) - Boucles spécialisées
- [TDD for Code Generation](https://arxiv.org/abs/2402.13521) - Tests d'abord

### Patterns
- [Temporal Workflows](https://temporal.io/blog/building-an-agentic-system-thats-actually-production-ready) - Durability patterns
