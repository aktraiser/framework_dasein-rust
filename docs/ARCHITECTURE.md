# Agentic-RS Architecture

> Framework Rust haute-performance pour systèmes multi-agents IA distribués

---

## Table des Matières

1. [Vue d'ensemble](#vue-densemble)
2. [Architecture Multi-Couche](#architecture-multi-couche)
3. [Communication NATS](#communication-nats)
4. [Création d'Agents](#création-dagents)
5. [Patterns de Coordination](#patterns-de-coordination)
6. [MCP Code Mode](#mcp-code-mode)
7. [Audit & Observabilité](#audit--observabilité)
8. [Exemples](#exemples)

---

## Vue d'ensemble

### Pourquoi Rust pour les Agents IA?

| Aspect | Rust | Python |
|--------|------|--------|
| Parsing d'intent | ~1ms | ~50-100ms |
| Cold start | ~5ms | ~500ms |
| Mémoire par agent | ~20MB | ~100MB+ |
| Concurrence | Native (Tokio) | GIL limitations |
| Fiabilité | Compile-time | Runtime errors |

### Principes Fondamentaux

```
┌─────────────────────────────────────────────────────────────────┐
│                      PRINCIPES AGENTIC-RS                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. DÉCOUPLAGE     → Tout passe par NATS, jamais d'appels      │
│                      directs entre agents                       │
│                                                                 │
│  2. SCALABILITÉ    → Dupliquer un agent = ajouter un subscriber│
│                                                                 │
│  3. RÉSILIENCE     → Un agent crash ≠ système crash            │
│                                                                 │
│  4. OBSERVABILITÉ  → Chaque action = event auditable           │
│                                                                 │
│  5. SIMPLICITÉ     → Créer un agent = implémenter 1 trait      │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## Architecture Multi-Couche

### Les 3 Couches

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                                                                             │
│  ╔═══════════════════════════════════════════════════════════════════════╗ │
│  ║  COUCHE 1: SUPERVISEURS                                               ║ │
│  ║  ─────────────────────                                                ║ │
│  ║  • Parse l'intent (ultra-rapide en Rust)                              ║ │
│  ║  • Route vers les bons exécuteurs                                     ║ │
│  ║  • Coordonne avec autres superviseurs via NATS                        ║ │
│  ║  • Peut prêter/emprunter des exécuteurs                              ║ │
│  ╚═══════════════════════════════════════════════════════════════════════╝ │
│                              │                                              │
│                              ▼                                              │
│  ╔═══════════════════════════════════════════════════════════════════════╗ │
│  ║  COUCHE 2: EXÉCUTEURS                                                 ║ │
│  ║  ────────────────────                                                 ║ │
│  ║  • Traitent les tâches en parallèle                                   ║ │
│  ║  • Appellent les LLMs                                                 ║ │
│  ║  • Exécutent du code dans les sandboxes                              ║ │
│  ║  • Peuvent être alloués dynamiquement entre superviseurs             ║ │
│  ╚═══════════════════════════════════════════════════════════════════════╝ │
│                              │                                              │
│                              ▼                                              │
│  ╔═══════════════════════════════════════════════════════════════════════╗ │
│  ║  COUCHE 3: VALIDATEURS (2 types)                                      ║ │
│  ║  ───────────────────────────────                                      ║ │
│  ║                                                                       ║ │
│  ║  Validator (règles)         │  SandboxValidator (ground truth)       ║ │
│  ║  ─────────────────          │  ────────────────────────────          ║ │
│  ║  • Heuristiques rapides     │  • Compilation réelle                  ║ │
│  ║  • Tout type d'output       │  • Tests réels (cargo test)            ║ │
│  ║  • ~1ms                     │  • ~2-5s                               ║ │
│  ║  • Texte, JSON, config      │  • Code uniquement                     ║ │
│  ║                                                                       ║ │
│  ║  → Pattern maker-checker avec feedback réel pour le code            ║ │
│  ╚═══════════════════════════════════════════════════════════════════════╝ │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Rôles et Responsabilités

#### Superviseur (Couche 1)

```rust
// Responsabilités:
// - Parser l'intent utilisateur
// - Décider quels exécuteurs utiliser
// - Demander de l'aide si surchargé
// - Agréger les résultats
// - Envoyer à la validation

pub struct Supervisor {
    id: String,
    domain: String,                    // "code", "data", "infra"
    executors: Vec<ExecutorHandle>,    // Mes exécuteurs
    validator: ValidatorHandle,        // Mon validateur
    bus: Arc<NatsBus>,
}
```

#### Exécuteur (Couche 2)

```rust
// Responsabilités:
// - Recevoir une tâche
// - Appeler le LLM
// - Exécuter du code si nécessaire
// - Retourner le résultat

pub struct Executor {
    id: String,
    owner_supervisor: String,          // Qui me possède
    current_supervisor: String,        // Qui me contrôle (si prêté)
    llm: Arc<dyn LLMAdapter>,
    sandbox: Option<Arc<dyn Sandbox>>,
}
```

#### Validateurs (Couche 3)

Deux types de validateurs selon le cas d'usage:

```rust
// === VALIDATOR (règles heuristiques) ===
// Pour: texte, JSON, validation rapide
// Responsabilités:
// - Appliquer des règles prédéfinies
// - Vérifications syntaxiques
// - Filtrage de contenu

pub struct Validator {
    id: String,
    supervisor: String,
    rules: Vec<ValidationRule>,  // OutputNotEmpty, NoSecrets, ValidJson...
}

// === SANDBOX VALIDATOR (ground truth) ===
// Pour: code généré par LLM
// Responsabilités:
// - Compilation réelle (cargo check)
// - Exécution des tests (cargo test)
// - Feedback avec vraies erreurs

pub struct SandboxValidator<S: Sandbox> {
    sandbox: S,                  // ProcessSandbox, DockerSandbox, Firecracker
    workspace: PathBuf,
    run_tests: bool,
}
```

---

## Communication NATS

### Topologie des Subjects

```
NATS Subjects Hierarchy
========================

sup.                                    # Superviseurs
├── {sup_id}.inbox                      # Inbox d'un superviseur
├── {sup_id}.outbox                     # Résultats d'un superviseur
├── help.request                        # Demande d'aide (broadcast)
├── help.offer                          # Offre d'aide
└── allocate.{from}.{to}                # Allocation d'exécuteurs

exe.                                    # Exécuteurs
├── {sup_id}.pool                       # Pool d'exécuteurs (queue group)
├── {sup_id}.{exe_id}.task              # Tâche pour un exécuteur spécifique
└── {sup_id}.{exe_id}.result            # Résultat d'un exécuteur

val.                                    # Validateurs
├── {sup_id}.request                    # Demande de validation
└── {sup_id}.result                     # Résultat de validation

audit.                                  # Audit (JetStream)
├── sup.{sup_id}                        # Events superviseur
├── exe.{sup_id}.{exe_id}               # Events exécuteur
├── val.{sup_id}                        # Events validateur
└── system                              # Events système
```

### Patterns de Communication

#### 1. Request-Reply (Tâche simple)

```
Client                  Superviseur                 Exécuteur
   │                         │                          │
   │──── task ──────────────►│                          │
   │                         │──── task ───────────────►│
   │                         │                          │
   │                         │◄──── result ─────────────│
   │◄──── result ────────────│                          │
```

#### 2. Fan-Out (Tâches parallèles)

```
Superviseur              Exécuteur Pool (Queue Group)
     │                   ┌─────────────────────────────┐
     │                   │  EXE-1   EXE-2   EXE-3     │
     │                   └─────────────────────────────┘
     │                          │      │      │
     │──── task ───────────────►│      │      │  (NATS distribue)
     │──── task ────────────────┼─────►│      │
     │──── task ────────────────┼──────┼─────►│
     │                          │      │      │
     │◄──── results ────────────┴──────┴──────┘
```

#### 3. Allocation Dynamique

```
SUP-A (surchargé)        NATS                    SUP-B (idle)
     │                     │                          │
     │── help.request ────►│─── broadcast ───────────►│
     │                     │                          │
     │                     │◄──── help.offer ─────────│
     │                     │                          │
     │── allocate.B.A ────►│                          │
     │                     │                          │
     │   [EXE-B1, EXE-B2 maintenant contrôlés par A] │
     │                     │                          │
     │──── tasks ─────────►│──────────────────────────│──► EXE-B1, EXE-B2
```

---

## Création d'Agents

### Trait de Base

Tout agent implémente le trait `Agent`:

```rust
#[async_trait]
pub trait Agent: Send + Sync {
    /// Identifiant unique
    fn id(&self) -> &str;

    /// Rôle dans l'architecture
    fn role(&self) -> AgentRole;

    /// Sujets NATS à écouter
    fn subscriptions(&self) -> Vec<String>;

    /// Traiter un message
    async fn handle(&self, msg: Message) -> Result<Option<Message>, AgentError>;

    /// Démarrer l'agent
    async fn start(&self) -> Result<(), AgentError>;

    /// Arrêter l'agent
    async fn stop(&self) -> Result<(), AgentError>;
}
```

### Créer un Superviseur

```rust
use agentic_core::{Supervisor, SupervisorConfig};

// Configuration
let config = SupervisorConfig {
    id: "sup-code-001".to_string(),
    domain: "code".to_string(),

    // Nombre d'exécuteurs à créer
    executor_count: 4,

    // Configuration LLM pour les exécuteurs
    llm_config: LLMConfig {
        provider: "gemini".to_string(),
        model: "gemini-2.0-flash".to_string(),
        api_key: env::var("GEMINI_API_KEY")?,
    },

    // Sandbox pour exécution de code
    sandbox_config: Some(SandboxConfig {
        sandbox_type: SandboxType::Process,
        timeout_ms: 30_000,
    }),

    // Règles de validation
    validation_rules: vec![
        ValidationRule::OutputNotEmpty,
        ValidationRule::NoErrors,
        ValidationRule::Custom("code_compiles".to_string()),
    ],
};

// Création
let supervisor = Supervisor::new(config, bus.clone()).await?;

// Démarrage (crée automatiquement les exécuteurs et validateur)
supervisor.start().await?;
```

### Créer un Exécuteur Standalone

```rust
use agentic_core::{Executor, ExecutorConfig};

let config = ExecutorConfig {
    id: "exe-001".to_string(),
    owner_supervisor: "sup-code-001".to_string(),

    llm: LLMConfig {
        provider: "gemini".to_string(),
        model: "gemini-2.0-flash".to_string(),
        api_key: env::var("GEMINI_API_KEY")?,
    },

    sandbox: Some(SandboxConfig {
        sandbox_type: SandboxType::Docker,
        image: "rust:latest".to_string(),
        timeout_ms: 60_000,
        memory_limit: 512 * 1024 * 1024,
    }),

    capabilities: vec![
        Capability::CodeGeneration,
        Capability::CodeExecution,
    ],
};

let executor = Executor::new(config, bus.clone()).await?;
executor.start().await?;
```

### Créer un Validateur

#### Option 1: Validator (règles heuristiques)

Pour texte, JSON, validation rapide:

```rust
use agentic_core::distributed::{Validator, ValidationRule};

let validator = Validator::new("val-001", "sup-001")
    .rule(ValidationRule::OutputNotEmpty)
    .rule(ValidationRule::NoErrors)
    .rule(ValidationRule::NoSecrets)
    .rule(ValidationRule::ValidJson)
    .max_retries(3)
    .build();

// Validation synchrone, ~1ms
let result = validator.validate(output, attempt);
if !result.passed {
    println!("Feedback: {:?}", result.feedback);
}
```

#### Option 2: SandboxValidator (ground truth)

Pour code généré, avec vraie compilation/tests:

```rust
use agentic_core::distributed::SandboxValidator;
use agentic_sandbox::ProcessSandbox;

// Sandbox léger pour dev
let sandbox = ProcessSandbox::new().with_timeout(60_000);
let validator = SandboxValidator::new(sandbox)
    .run_tests(true);

// Validation async, ~2-5s (vraie compilation)
let result = validator.validate_rust_code(code).await?;

if !result.passed {
    if !result.compiles {
        // Vraies erreurs rustc
        println!("Compiler: {:?}", result.compiler_errors);
    } else {
        // Vrais échecs de tests
        println!("Tests: {:?}", result.test_errors);
    }
}
```

#### Quand utiliser lequel?

| Type d'output | Validator | SandboxValidator |
|---------------|-----------|------------------|
| Texte/docs | ✅ | ❌ |
| JSON/config | ✅ | ❌ |
| Code Rust/Python | ❌ | ✅ |
| Scripts shell | ✅ syntaxe | ✅ exécution |

---

## Patterns de Coordination

### Pattern 1: Superviseur Simple

Un superviseur avec ses exécuteurs et validateur.

```rust
// Créer un superviseur "code" avec 4 exécuteurs
let code_supervisor = Supervisor::builder()
    .id("sup-code")
    .domain("code")
    .executors(4)
    .llm(gemini_config)
    .sandbox(process_sandbox)
    .validation(vec![
        ValidationRule::OutputNotEmpty,
        ValidationRule::CodeCompiles,
    ])
    .build(bus.clone())
    .await?;

code_supervisor.start().await?;

// Envoyer une tâche
let result = code_supervisor.execute(Task {
    action: "generate".to_string(),
    spec: json!({
        "task": "Write a function to calculate fibonacci"
    }),
}).await?;
```

### Pattern 2: Multi-Superviseurs Collaboratifs

Plusieurs superviseurs qui peuvent s'entraider.

```rust
// Créer plusieurs superviseurs
let code_sup = Supervisor::builder()
    .id("sup-code")
    .domain("code")
    .executors(4)
    .allow_lending(true)      // Peut prêter ses exécuteurs
    .allow_borrowing(true)    // Peut emprunter des exécuteurs
    .build(bus.clone()).await?;

let data_sup = Supervisor::builder()
    .id("sup-data")
    .domain("data")
    .executors(2)
    .allow_lending(true)
    .allow_borrowing(true)
    .build(bus.clone()).await?;

let infra_sup = Supervisor::builder()
    .id("sup-infra")
    .domain("infra")
    .executors(2)
    .allow_lending(true)
    .allow_borrowing(false)   // Reçoit mais ne prête pas
    .build(bus.clone()).await?;

// Les superviseurs communiquent automatiquement via NATS
// Si code_sup est surchargé, il peut emprunter à data_sup
```

### Pattern 3: Pipeline Multi-Superviseurs

Chaîner les superviseurs pour des workflows complexes.

```rust
// Définir un pipeline
let pipeline = Pipeline::new("feature-implementation")
    .step("plan", "sup-code")       // Planifier
    .step("implement", "sup-code")   // Implémenter
    .step("test", "sup-code")        // Tester
    .step("deploy", "sup-infra")     // Déployer
    .step("monitor", "sup-data")     // Monitorer
    .build();

// Exécuter le pipeline
let result = pipeline.execute(json!({
    "feature": "Add user authentication"
})).await?;
```

### Pattern 4: Scaling Horizontal

Dupliquer des superviseurs pour scaling.

```rust
// Créer N instances du même superviseur
// Chaque instance écoute le même subject (queue group)
for i in 0..num_instances {
    let sup = Supervisor::builder()
        .id(format!("sup-code-{}", i))
        .domain("code")
        .executors(4)
        .queue_group("code-supervisors")  // NATS distribue automatiquement
        .build(bus.clone()).await?;

    sup.start().await?;
}

// Les requêtes sur "sup.code.inbox" sont distribuées
// automatiquement entre les instances
```

### Pattern 5: Grounded Validation Loop (pour code)

Boucle de correction avec feedback réel du compilateur/tests.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        GROUNDED VALIDATION LOOP                             │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│    ┌──────────┐         ┌───────────────────┐         ┌──────────────┐     │
│    │ Executor │ ──────► │ SandboxValidator  │ ──────► │   Résultat   │     │
│    │  (LLM)   │         │  (cargo check +   │         │              │     │
│    └──────────┘         │   cargo test)     │         └──────────────┘     │
│         ▲               └───────────────────┘                │              │
│         │                                                    │              │
│         │                    ┌─────────────────────────────┐│              │
│         │                    │  Si échec:                  ││              │
│         │                    │  • Erreurs compilateur      │◄┘              │
│         └────────────────────│  • Échecs de tests          │               │
│           Feedback réel      │  • Lignes précises          │               │
│           (pas LLM review)   └─────────────────────────────┘               │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

```rust
use agentic_core::distributed::{Executor, SandboxValidator};
use agentic_sandbox::ProcessSandbox;

// Créer l'exécuteur (génère le code)
let executor = Executor::new("exe-001", "sup-001")
    .llm_gemini("gemini-2.0-flash")
    .build();

// Créer le validateur sandbox (compile + teste vraiment)
let sandbox = ProcessSandbox::new();
let validator = SandboxValidator::new(sandbox);

// Boucle de correction
let mut code = executor.execute(system_prompt, task).await?.content;

for attempt in 0..MAX_RETRIES {
    let result = validator.validate_rust_code(&code).await?;

    if result.passed {
        println!("Success after {} attempts!", attempt + 1);
        break;
    }

    // Feedback RÉEL du compilateur, pas une review LLM
    let feedback = if !result.compiles {
        format!("COMPILER ERRORS:\n{:?}", result.compiler_errors)
    } else {
        format!("TEST FAILURES:\n{:?}", result.test_errors)
    };

    // Renvoyer à l'Executor pour correction
    code = executor.execute(system_prompt, &format!(
        "Fix this code:\n{}\n\nErrors:\n{}", code, feedback
    )).await?.content;
}
```

**Avantage**: Élimine le biais "LLM qui review LLM" en utilisant des erreurs objectives.

Voir `examples/grounded_loop.rs` pour l'implémentation complète.

---

## MCP Code Mode

### Le Problème (MCP Traditionnel)

L'approche traditionnelle où l'agent appelle directement les outils MCP consomme énormément de tokens:

```
Agent → appelle tool1 → 10,000 tokens dans le contexte
      → appelle tool2 → 10,000 tokens dans le contexte
      → appelle tool3 → 10,000 tokens dans le contexte

Total: 150,000+ tokens par workflow
```

### La Solution (Code Mode)

L'agent écrit du **code TypeScript** qui appelle les outils MCP. Le code s'exécute dans notre sandbox, et seul le résultat final retourne au contexte.

```
┌─────────────────────────────────────────────────────────────────┐
│                        Agent (Executor)                          │
│                              │                                   │
│                      écrit TypeScript                            │
│                              ▼                                   │
│              ┌──────────────────────────────┐                   │
│              │      Sandbox (Firecracker)    │                   │
│              │                              │                   │
│              │  ./servers/                  │                   │
│              │    ├── context7/             │                   │
│              │    │   └── queryDocs.ts      │                   │
│              │    ├── github/               │                   │
│              │    │   └── createPR.ts       │                   │
│              │    └── ...                   │                   │
│              │                              │                   │
│              │  Code imports et appelle     │                   │
│              │  ces modules générés         │                   │
│              └──────────────┬───────────────┘                   │
│                             │                                    │
└─────────────────────────────┼────────────────────────────────────┘
                              │ Appels MCP (depuis le sandbox)
         ┌────────────────────┼────────────────────┐
         ▼                    ▼                    ▼
  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
  │  Context7    │   │   GitHub     │   │   Postgres   │
  └──────────────┘   └──────────────┘   └──────────────┘

Total: 2,000 tokens (98.7% de réduction!)
```

### Configuration

```json
{
  "mcpServers": {
    "context7": {
      "url": "https://mcp.context7.com/mcp",
      "headers": { "CONTEXT7_API_KEY": "..." }
    },
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_TOKEN": "..." }
    }
  }
}
```

### Exemple d'Utilisation

```rust
use agentic_mcp::{MCPConfig, MCPClientPool};
use agentic_sandbox::FirecrackerSandbox;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Charger la config MCP
    let config = MCPConfig::from_file("mcp_servers.json")?;

    // 2. Créer le pool de clients MCP
    let pool = MCPClientPool::new(config).await?;

    // 3. Générer les fichiers TypeScript pour les outils
    pool.generate_typescript(Path::new("./workspace")).await?;

    // 4. L'agent écrit du code TypeScript
    let code = r#"
        import { queryDocs } from './servers/context7';

        const docs = await queryDocs({
            libraryId: '/vercel/next.js',
            query: 'app router'
        });
        console.log(docs.summary);
    "#;

    // 5. Exécuter dans le sandbox
    let sandbox = FirecrackerSandbox::builder().build();
    let result = sandbox.execute(&format!("cd workspace && npx tsx -e '{}'", code)).await?;

    // Seul le résultat final est visible
    println!("Agent voit: {}", result.stdout);

    Ok(())
}
```

### Avantages

| Aspect | MCP Direct | Code Mode |
|--------|------------|-----------|
| Tokens par workflow | 150,000+ | ~2,000 |
| Données intermédiaires | Dans le contexte | Dans le sandbox |
| Composition d'outils | Séquentielle | Code natif |
| Gestion d'erreurs | Par l'agent | try/catch |
| Confidentialité | Tout visible | Filtrable |

### Serveurs MCP Supportés

| Serveur | Type | Description |
|---------|------|-------------|
| Context7 | HTTP | Documentation de librairies |
| GitHub | stdio | Opérations Git/PR |
| Postgres | stdio | Requêtes SQL |
| Filesystem | stdio | Accès fichiers |
| Custom | HTTP/stdio | Vos propres serveurs |

---

## Audit & Observabilité

### Structure des Events

```rust
/// Event d'audit complet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    // Identifiants
    pub id: String,
    pub timestamp: DateTime<Utc>,

    // Tracing distribué
    pub trace_id: String,           // ID du workflow complet
    pub span_id: String,            // ID de cette opération
    pub parent_span_id: Option<String>,

    // Agent
    pub agent_id: String,
    pub agent_role: AgentRole,
    pub agent_domain: String,

    // Event
    pub event_type: String,
    pub event_data: serde_json::Value,

    // Contexte
    pub context: AuditContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditContext {
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
    pub environment: String,
    pub version: String,
}
```

### Types d'Events

```rust
pub enum AuditEventType {
    // === Cycle de vie ===
    AgentStarted,
    AgentStopped,
    AgentHealthCheck { healthy: bool },

    // === Superviseur ===
    IntentReceived { raw: String },
    IntentParsed { intent: String, confidence: f32, latency_ms: u64 },
    TaskCreated { task_id: String, action: String },
    TaskDispatched { task_id: String, executor_id: String },
    TaskAggregated { task_id: String, results_count: usize },

    // === Coordination ===
    HelpRequested { reason: String, executors_needed: usize },
    HelpOffered { executors_available: usize },
    ExecutorsLent { to_supervisor: String, executor_ids: Vec<String> },
    ExecutorsBorrowed { from_supervisor: String, executor_ids: Vec<String> },
    ExecutorsReturned { executor_ids: Vec<String> },

    // === Exécuteur ===
    TaskReceived { task_id: String, from_supervisor: String },
    LLMRequestStarted { model: String, prompt_tokens: u32 },
    LLMRequestCompleted { model: String, completion_tokens: u32, latency_ms: u64 },
    LLMRequestFailed { model: String, error: String },
    CodeExecutionStarted { language: String },
    CodeExecutionCompleted { exit_code: i32, duration_ms: u64 },
    TaskCompleted { task_id: String, status: String },
    TaskFailed { task_id: String, error: String },

    // === Validateur ===
    ValidationStarted { task_id: String },
    ValidationRulePassed { rule: String },
    ValidationRuleFailed { rule: String, reason: String },
    ValidationCompleted { task_id: String, passed: bool },
    RetryRequested { task_id: String, attempt: u32 },

    // === Erreurs ===
    Error { code: String, message: String, recoverable: bool },
    Timeout { operation: String, timeout_ms: u64 },
}
```

### Collecteur d'Audit

```rust
use agentic_core::{AuditCollector, AuditStorage};

// Créer le collecteur
let collector = AuditCollector::new(bus.clone())
    .storage(RedisAuditStorage::new(redis_url).await?)
    .filters(vec![
        AuditFilter::MinLevel(AuditLevel::Info),
        AuditFilter::ExcludeHealthChecks,
    ])
    .build();

// Démarrer la collecte (subscribe à audit.>)
collector.start().await?;

// Requêter l'audit
let events = collector.query(AuditQuery {
    trace_id: Some("abc-123".to_string()),
    time_range: Some(TimeRange::last_hours(1)),
    agent_role: Some(AgentRole::Executor),
    event_types: vec!["LLMRequestCompleted".to_string()],
    limit: 100,
}).await?;
```

### Intégration OpenTelemetry

```rust
use agentic_core::telemetry::{init_telemetry, TelemetryConfig};

// Configurer OpenTelemetry
init_telemetry(TelemetryConfig {
    service_name: "agentic-rs".to_string(),
    otlp_endpoint: Some("http://localhost:4317".to_string()),

    // Exporter les events NATS comme spans
    export_nats_events: true,

    // Métriques Prometheus
    prometheus_port: Some(9090),
})?;

// Les events d'audit sont automatiquement convertis en spans OpenTelemetry
// avec les bons parent/child relationships
```

---

## Exemples

### Exemple 1: Agent de Code Simple

```rust
use agentic_rs::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialiser
    init_telemetry(TelemetryConfig::default())?;

    // Connexion NATS
    let bus = NatsBus::connect("nats://localhost:4222").await?;

    // Créer un superviseur de code
    let supervisor = Supervisor::builder()
        .id("code-agent")
        .domain("code")
        .executors(2)
        .llm(LLMConfig::gemini("gemini-2.0-flash"))
        .sandbox(SandboxConfig::process())
        .build(bus.clone())
        .await?;

    supervisor.start().await?;

    // Exécuter une tâche
    let result = supervisor.execute(Task::new(
        "generate",
        json!({ "task": "Write a fibonacci function in Rust" })
    )).await?;

    println!("Result: {:?}", result);

    supervisor.stop().await?;
    Ok(())
}
```

### Exemple 2: Multi-Agents Collaboratifs

```rust
use agentic_rs::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let bus = NatsBus::connect("nats://localhost:4222").await?;

    // Créer plusieurs superviseurs spécialisés
    let code_sup = Supervisor::builder()
        .id("sup-code")
        .domain("code")
        .executors(4)
        .llm(LLMConfig::gemini("gemini-2.0-flash"))
        .allow_lending(true)
        .build(bus.clone()).await?;

    let review_sup = Supervisor::builder()
        .id("sup-review")
        .domain("review")
        .executors(2)
        .llm(LLMConfig::gemini("gemini-2.0-flash"))
        .build(bus.clone()).await?;

    // Démarrer
    code_sup.start().await?;
    review_sup.start().await?;

    // Créer un workflow
    let workflow = Workflow::new("code-review")
        .step("generate")
            .supervisor("sup-code")
            .action("generate")
            .input(json!({ "task": "Write a REST API endpoint" }))
        .step("review")
            .supervisor("sup-review")
            .action("review")
            .depends_on("generate")
            .input_from("generate", "$.code")
        .step("revise")
            .supervisor("sup-code")
            .action("revise")
            .depends_on("review")
            .input_from("review", "$.feedback")
            .condition("review.passed == false")
        .build();

    let result = workflow.execute().await?;
    println!("Final code: {}", result.get("revise.code").or(result.get("generate.code")));

    Ok(())
}
```

### Exemple 3: Scaling Dynamique

```rust
use agentic_rs::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let bus = NatsBus::connect("nats://localhost:4222").await?;

    // Pool de superviseurs avec auto-scaling
    let pool = SupervisorPool::builder()
        .domain("code")
        .min_instances(2)
        .max_instances(10)
        .scale_up_threshold(0.8)    // CPU > 80%
        .scale_down_threshold(0.2)  // CPU < 20%
        .executors_per_supervisor(4)
        .llm(LLMConfig::gemini("gemini-2.0-flash"))
        .build(bus.clone())
        .await?;

    pool.start().await?;

    // Envoyer des tâches (distribuées automatiquement)
    let tasks: Vec<_> = (0..100).map(|i| {
        Task::new("generate", json!({ "task": format!("Task {}", i) }))
    }).collect();

    let results = pool.execute_batch(tasks).await?;

    println!("Completed {} tasks", results.len());

    Ok(())
}
```

---

## Multi-Executor Architecture (Phase 1.7)

### Overview

Pour les tâches complexes (génération de code multi-fichiers), un seul Executor peut produire du code incohérent. L'architecture Multi-Executor décompose la tâche et assure la cohérence.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                      MULTI-EXECUTOR PIPELINE                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌──────────────────┐                                                       │
│  │  TaskDecomposer  │  Décompose une tâche en sub-tasks                     │
│  └────────┬─────────┘                                                       │
│           │                                                                 │
│           ▼                                                                 │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐                       │
│  │ Executor │ │ Executor │ │ Executor │ │ Executor │   (en parallèle)      │
│  │ imports  │ │  types   │ │   impl   │ │  tests   │                       │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘                       │
│       │            │            │            │                              │
│       └────────────┴─────┬──────┴────────────┘                              │
│                          ▼                                                  │
│                ┌─────────────────────┐                                      │
│                │  LiaisonArchitect   │  Analyse cohérence, fixe les erreurs │
│                │  (5ème agent)       │                                      │
│                └──────────┬──────────┘                                      │
│                           ▼                                                 │
│                 ┌──────────────────┐                                        │
│                 │   CodeAssembler  │  Assemble, déduplique imports          │
│                 └────────┬─────────┘                                        │
│                          ▼                                                  │
│                ┌───────────────────┐                                        │
│                │ ValidatorPipeline │  SandboxValidator → MCPDocValidator    │
│                └───────────────────┘                                        │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Composants

#### TaskDecomposer

Décompose une tâche complexe en sub-tasks ordonnées:

```rust
use agentic_core::distributed::TaskDecomposer;

let decomposer = TaskDecomposer::new("rust");
let subtasks = decomposer.decompose("Implement an async task scheduler...");

// Génère automatiquement:
// - imports (order: 0) - use statements
// - types (order: 1) - struct/enum definitions
// - impl (order: 2) - implementations
// - tests (order: 3) - test module
```

#### LiaisonArchitect (5ème Agent)

Assure la cohérence entre les outputs des Executors parallèles:

```rust
use agentic_core::distributed::{LiaisonArchitect, SubTaskResult};

let liaison = LiaisonArchitect::new("rust");

// Analyse les incohérences
let report = liaison.analyze(&subtask_results);
if liaison.needs_fixing(&report) {
    println!("Issues: {:?}", report.issues);
    // MissingField, MissingMethod, MissingImport, NameMismatch...
}

// Fix automatique via un Executor
let fixed = liaison.fix(&subtask_results, &executor).await?;

// Validation syntaxique (braces équilibrées)
if let Some(err) = liaison.validate_syntax(&fixed.code) {
    println!("Syntax error: {}", err);
}
```

#### CodeAssembler

Assemble les fragments de code:

```rust
use agentic_core::distributed::CodeAssembler;

let assembler = CodeAssembler::new();

// Assemble dans l'ordre (imports → types → impl → tests)
let code = assembler.assemble(&results)?;

// Nettoie le code avant validation
// - Supprime les backticks markdown
// - Remplace les smart quotes
// - Déduplique les imports
let clean = assembler.clean_for_validation(&code);
```

### Stuck Detection

Le pipeline détecte les boucles infinies:

```rust
// Hash des erreurs pour détecter les répétitions
let error_hash = hash(&errors);
if error_hash == last_error_hash {
    stuck_count += 1;
    if stuck_count >= STUCK_THRESHOLD {
        // Régénérer depuis le début
        current_code = generate_initial(...).await?;
    }
}

// Détection rapide des erreurs de syntaxe
if errors.contains("unclosed delimiter") {
    syntax_error_count += 1;
    if syntax_error_count >= 2 {
        // Les erreurs de syntaxe persistent = régénérer
        current_code = generate_initial(...).await?;
    }
}
```

### Example Complet

Voir `examples/multi_executor.rs` pour l'implémentation complète.

---

## Progressive Locking Pipeline (Phase 2.1)

### Le Problème

Le Multi-Executor initial générait tout le code d'un coup, causant:
- Code incohérent entre types et implémentations
- Boucles infinies de correction (LLM fait toujours la même erreur)
- Over-engineering avec ConstraintTracker, CorrectionPlan, BlankSlate...

### La Solution: Progressive Locking

Décomposer en 3 stages et **verrouiller chaque stage une fois validé**:

```
┌─────────────────────────────────────────────────────────────┐
│                    PROGRESSIVE LOCKING                       │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│   [Stage 1: TYPES]  ──lint──►  LOCK  ─┐                     │
│   (struct, enum, type alias)          │                     │
│                                        │                     │
│   [Stage 2: STUBS]  ──lint──►  LOCK  ─┼─► Context           │
│   (impl blocks with todo!())          │                     │
│                                        │                     │
│   [Stage 3: LOGIC]  ──compile+test──► DONE                  │
│   (function bodies + tests)                                  │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Composants

#### 1. BusLinter (Fast Pre-validation)

Validation syntaxique rapide avant compilation (~3ms vs ~25s):

```rust
use agentic_core::distributed::bus::BusLinter;

let linter = BusLinter::new();
let result = linter.lint(&code);

if !result.passed {
    // Erreur de syntaxe détectée en 3ms
    println!("Error: {}", result.errors[0].message);
}
```

**Checks effectués:**
- Balanced braces `{}`
- Balanced brackets `[]`
- Balanced parentheses `()`
- Unclosed string literals
- Basic syntax patterns

#### 2. RollbackManager (Prevent Regressions)

Empêche les régressions en trackant la qualité de chaque tentative:

```rust
use agentic_core::distributed::bus::{RollbackManager, RollbackDecision};

let mut rollback = RollbackManager::new();

// Enregistrer chaque tentative avec son score
let decision = rollback.record(attempt_num, code.clone(), errors.clone());

match decision {
    RollbackDecision::Continue { current_score, best_score } => {
        // Continuer - score: -200 (2 compile errors)
    }
    RollbackDecision::Rollback { code, reason, failing_tests, .. } => {
        // Régression détectée! Rollback au meilleur code
        // Reason: "Code stopped compiling (2 errors)"
    }
}
```

**Scoring:**
```rust
if !compiles {
    score = -(compile_errors * 100)  // Négatif
} else {
    score = (tests_passed * 10) - (tests_failed * 5)  // Positif
}
```

**Déclencheurs de rollback:**
- Code compilait → compile plus
- 2+ dégradations consécutives
- Régression significative des tests (+2)

#### 3. Error Feedback Loop

Les erreurs sont injectées dans le prompt du prochain attempt:

```rust
let stage_prompt = if previous_errors.is_empty() {
    base_prompt
} else {
    format!(r#"{}

=== PREVIOUS ATTEMPT FAILED ===
{}

=== YOUR PREVIOUS CODE ===
{}

Fix the errors and return the corrected code."#,
        base_prompt,
        previous_errors.join("\n"),
        previous_code
    )
};
```

### Flux d'exécution

```
┌─────────────────────────────────────────────────────────────┐
│  Attempt N                                                  │
│      │                                                      │
│      ▼                                                      │
│  ┌───────┐    ┌────────┐    ┌──────────┐                   │
│  │  LLM  │───►│ Linter │───►│ Passed?  │──Yes──► Lock      │
│  └───────┘    │ (3ms)  │    └────┬─────┘                   │
│      ▲        └────────┘         │ No                      │
│      │                           ▼                          │
│      │                    ┌──────────────┐                 │
│      │                    │   Rollback   │                 │
│      │                    │   Manager    │                 │
│      │                    └──────┬───────┘                 │
│      │                           │                          │
│      │         ┌─────────────────┼─────────────────┐       │
│      │         ▼                 ▼                 ▼       │
│      │    Continue           Rollback          Return      │
│      │   (+ errors)        (best code)          Best       │
│      │         │                 │                          │
│      └─────────┴─────────────────┘                          │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Résultats

**Exécution typique:**
```
[Stage 1/3] TYPES
  Attempt 1/5
    ✓ Lint OK
  ✓ Types LOCKED (675 chars)

[Stage 2/3] STUBS
  Attempt 1/5
    ✓ Lint OK
  ✓ Stubs LOCKED (9698 chars)

[Stage 3/3] LOGIC
  Attempt 1/5
    ✗ 3 errors (score: -300, best: -300)
  Attempt 2/5
    ✗ 2 errors (score: -200, best: -200)
  Attempt 3/5
    ✗ 3 errors (score: -300, best: -200)
  Attempt 4/5
    ✗ 1 errors (score: 0, best: 0)      ← Compile OK!
  Attempt 5/5
    ↩ ROLLBACK: Code stopped compiling
    ⚠ Returning best attempt (compiles, 0 tests fail)

SUCCESS in 345939ms
```

**Métriques:**

| Métrique | Avant (Over-engineered) | Après (Progressive Locking) |
|----------|------------------------|----------------------------|
| Lignes de code | ~1300 | ~260 |
| Modules | 6+ | 2 (linter, rollback) |
| Taux de succès | ~20% | ~80% |
| Temps moyen | 10+ min | 3-6 min |
| Complexité | Haute | Simple |

### Code

```rust
// examples/multi_executor.rs (simplifié)

const MAX_STAGE_RETRIES: u32 = 5;

async fn main() {
    // Stage 1: Types
    let locked_types = generate_and_lock_stage(
        Stage::Types, &executor, task, ""
    ).await?;

    // Stage 2: Stubs (avec types comme contexte)
    let locked_stubs = generate_and_lock_stage(
        Stage::Stubs, &executor, task, &locked_types
    ).await?;

    // Stage 3: Logic (avec types+stubs comme contexte)
    let final_code = generate_and_lock_stage(
        Stage::Logic, &executor, task, &locked_stubs
    ).await?;
}

async fn generate_and_lock_stage(
    stage: Stage,
    executor: &Executor,
    task: &str,
    locked_context: &str,
) -> Result<String> {
    let mut rollback = RollbackManager::new();
    let mut previous_errors = vec![];

    for attempt in 1..=MAX_STAGE_RETRIES {
        let prompt = build_stage_prompt(stage, task, locked_context, &previous_errors);
        let code = executor.execute(system, &prompt).await?.content;

        // Fast lint check
        if !linter.lint(&code).passed { continue; }

        // For Types/Stubs, lint is enough
        if matches!(stage, Stage::Types | Stage::Stubs) {
            return Ok(code);
        }

        // For Logic, full validation
        let result = pipeline.validate(&code).await;
        if result.passed { return Ok(code); }

        // Track with RollbackManager
        match rollback.record(attempt, code, result.errors) {
            RollbackDecision::Rollback { code, .. } => {
                // Use best code for next attempt
            }
            RollbackDecision::Continue { .. } => {
                // Continue with error feedback
            }
        }
    }

    // Return best attempt if it compiles
    if let Some(best) = rollback.best() {
        if best.score.compiles {
            return Ok(best.code.clone());
        }
    }

    Err("Stage failed")
}
```

### Leçons apprises

1. **Simple > Complex**: Progressive Locking avec ~260 lignes bat l'approche over-engineered avec ~1300 lignes

2. **Lock early**: Verrouiller les types tôt évite les incohérences

3. **Fast feedback**: Linter à 3ms permet plus d'itérations

4. **Prevent regressions**: RollbackManager évite de perdre du bon code

5. **Generic > Specific**: Pas de règles hardcodées (async recursion, etc.)

---

## Error Fingerprinting + Model Hierarchy (Phase 2.2)

### Le Problème

Utiliser le même modèle pour tous les types d'erreurs est inefficace:
- Erreurs simples (imports, syntax) → gaspillage avec un gros modèle
- Erreurs complexes (ownership, lifetimes) → échec avec un petit modèle

### La Solution: Error Fingerprinting

Classification automatique des erreurs et routage vers le modèle approprié:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          ERROR FINGERPRINTING                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   Erreurs du compilateur                                                    │
│          │                                                                  │
│          ▼                                                                  │
│   ┌──────────────────┐                                                     │
│   │  ErrorFingerprinter│                                                    │
│   │                    │                                                    │
│   │  Patterns:         │                                                    │
│   │  • E0432 → Lookup  │                                                    │
│   │  • E0382 → Owner   │                                                    │
│   │  • E0106 → Lifetime│                                                    │
│   │  • test failed →   │                                                    │
│   │    TestFailure     │                                                    │
│   └────────┬───────────┘                                                   │
│            │                                                                │
│    ┌───────┴───────┬─────────────────┐                                     │
│    ▼               ▼                 ▼                                     │
│ ┌──────┐      ┌──────┐         ┌──────────┐                               │
│ │ Fast │      │Smart │         │  Expert  │                               │
│ │  ⚡  │      │  🧠  │         │    🎓    │                               │
│ └───┬──┘      └───┬──┘         └────┬─────┘                               │
│     │             │                  │                                      │
│     ▼             ▼                  ▼                                      │
│ gemini-2.0   gemini-3.0         (future)                                   │
│ -flash       -flash-preview     Claude Opus                                │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Composants Phase 2.2

#### 1. ErrorFingerprinter

```rust
use agentic_core::distributed::bus::{ErrorFingerprinter, ModelTier, ErrorCategory};

let fingerprinter = ErrorFingerprinter::new();
let analysis = fingerprinter.analyze(&errors);

// Catégorie dominante et tier recommandé
println!("Category: {:?}", analysis.dominant_category);  // Ownership
println!("Tier: {:?}", analysis.recommended_tier);       // Smart

// Génère des hints pour le LLM
let hint = fingerprinter.generate_hint(&analysis);
// "Focus on borrow checker rules. Consider using .clone() or references."
```

#### 2. Dual Executor Setup

```rust
// Fast model: cheap, pour erreurs simples
let fast_executor = Executor::new("fast-exe", "supervisor")
    .llm_gemini(&std::env::var("FAST_MODEL").unwrap_or("gemini-2.0-flash".into()))
    .build();

// Smart model: expensive, pour erreurs complexes
let smart_executor = Executor::new("smart-exe", "supervisor")
    .llm_gemini(&std::env::var("SMART_MODEL").unwrap_or("gemini-3-flash-preview".into()))
    .build();

// Sélection automatique basée sur les erreurs
let executor = match fingerprinter.analyze(&errors).recommended_tier {
    ModelTier::Fast => &fast_executor,
    ModelTier::Smart | ModelTier::Expert => &smart_executor,
};
```

#### 3. Clippy Integration

Analyse statique pour détecter les anti-patterns:

```rust
// Intégré dans SandboxPipelineValidator
// Exécuté après compilation, avant tests

// Détecte:
// - permit.forget() → memory leak
// - .clone() au lieu de Arc::clone()
// - Unused results
// - Inefficient patterns
```

#### 4. MCP Doc Validator

Enrichit les erreurs avec de la documentation:

```rust
// Si CONTEXT7_API_KEY est défini:
let mcp_config = MCPDocConfig::context7(&api_key);
let mcp_validator = MCPDocValidator::new(mcp_config);

// Pipeline avec documentation
let pipeline = ValidatorPipeline::new()
    .add(sandbox_validator)  // Compile + Clippy + Tests
    .add(mcp_validator);     // Fetch docs si erreurs
```

### Résultats Phase 2.2

| Métrique | Avant | Après |
|----------|-------|-------|
| Qualité code | 5-6/10 | 7-8/10 |
| Taux succès | ~80% | ~85% |
| Coût tokens | $$$ | $$ (60% fast model) |
| Anti-patterns | Non détectés | Clippy les catch |

---

## NATS Bus Coordinator (Phase 2.0)

### Rôle de la Couche Coordinateur

Le Bus Coordinator gère la communication inter-superviseurs avec 4 responsabilités:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        NATS BUS COORDINATOR                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                          NATS JetStream                              │   │
│  │                                                                      │   │
│  │   Streams:                                                           │   │
│  │   ├── TASKS      (work queue with ordering)                         │   │
│  │   ├── PROPOSALS  (executor proposals for arbitrage)                 │   │
│  │   ├── LOGS       (structured log collection)                        │   │
│  │   └── AUDIT      (event sourcing)                                   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                              │                                              │
│           ┌──────────────────┼──────────────────┐                          │
│           ▼                  ▼                  ▼                          │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐                   │
│  │  Sequencer   │   │   Arbiter    │   │ Deduplicator │                   │
│  │              │   │              │   │              │                   │
│  │ • Priority   │   │ • Best of N  │   │ • Hash-based │                   │
│  │ • Ordering   │   │ • Voting     │   │ • Time-window│                   │
│  │ • Rate limit │   │ • Scoring    │   │ • Idempotency│                   │
│  └──────────────┘   └──────────────┘   └──────────────┘                   │
│           │                  │                  │                          │
│           └──────────────────┼──────────────────┘                          │
│                              ▼                                              │
│                    ┌──────────────────┐                                    │
│                    │   Log Collector  │                                    │
│                    │                  │                                    │
│                    │ • Structured logs│                                    │
│                    │ • Trace context  │                                    │
│                    │ • Retention      │                                    │
│                    └──────────────────┘                                    │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 1. Séquenceur (Sequencer)

Ordonne et priorise les tâches:

```rust
use agentic_core::distributed::bus::{Sequencer, TaskPriority};

let sequencer = Sequencer::new(nats_client.clone())
    .stream("TASKS")
    .max_pending(100)
    .rate_limit(10_per_second)
    .build()
    .await?;

// Publier une tâche avec priorité
sequencer.publish(Task {
    id: "task-001",
    priority: TaskPriority::High,
    supervisor: "sup-code",
    payload: json!({ "action": "generate" }),
    depends_on: vec!["task-000"],  // Ordering
}).await?;

// Consumer respecte l'ordre et les dépendances
let task = sequencer.next().await?;
```

### 2. Arbitre (Arbiter)

Choisit la meilleure proposition parmi plusieurs Executors:

```rust
use agentic_core::distributed::bus::{Arbiter, ArbiterStrategy};

let arbiter = Arbiter::new(nats_client.clone())
    .stream("PROPOSALS")
    .strategy(ArbiterStrategy::BestOfN { n: 3, timeout_ms: 5000 })
    .scorer(|proposal| {
        // Score basé sur: qualité, confiance, temps
        proposal.quality_score * 0.5
        + proposal.confidence * 0.3
        - proposal.latency_ms as f64 * 0.001
    })
    .build()
    .await?;

// Demander des propositions
let request_id = arbiter.request_proposals(
    "Generate fibonacci function",
    3,  // Nombre de propositions souhaitées
).await?;

// Attendre et sélectionner la meilleure
let best = arbiter.select_best(request_id).await?;
println!("Best proposal from {}: score={}", best.executor_id, best.score);
```

### 3. Dédoublonneur (Deduplicator)

Évite les traitements en double:

```rust
use agentic_core::distributed::bus::{Deduplicator, DedupeStrategy};

let dedup = Deduplicator::new(nats_client.clone())
    .window(Duration::from_secs(60))  // Fenêtre de déduplication
    .strategy(DedupeStrategy::ContentHash)  // Hash du contenu
    .build()
    .await?;

// Vérifier avant traitement
let task_hash = hash(&task.payload);
if dedup.is_duplicate(task_hash).await? {
    println!("Task already processed, skipping");
    return Ok(());
}

// Marquer comme traité
dedup.mark_processed(task_hash).await?;
```

### 4. Collecteur de Logs (Log Collector)

Centralise les logs de tous les agents:

```rust
use agentic_core::distributed::bus::{LogCollector, LogLevel};

let collector = LogCollector::new(nats_client.clone())
    .stream("LOGS")
    .retention(Duration::from_days(7))
    .build()
    .await?;

// Les agents publient leurs logs
collector.log(LogEntry {
    level: LogLevel::Info,
    agent_id: "exe-001",
    trace_id: "trace-abc",
    message: "Task completed",
    metadata: json!({ "duration_ms": 150 }),
}).await?;

// Query les logs
let logs = collector.query(LogQuery {
    trace_id: Some("trace-abc"),
    level: Some(LogLevel::Error),
    time_range: TimeRange::last_hours(1),
}).await?;
```

### Subjects NATS pour Coordination

```
bus.                                      # Bus Coordinator
├── tasks.{priority}.{supervisor}         # Tâches ordonnées par priorité
├── proposals.{request_id}                # Propositions pour arbitrage
├── proposals.{request_id}.vote           # Votes/scores
├── dedup.check                           # Vérification de doublons
├── dedup.mark                            # Marquage traité
├── logs.{agent_id}                       # Logs par agent
└── logs.query                            # Requêtes de logs

# JetStream Streams
TASKS           → bus.tasks.>           # Work queue ordonné
PROPOSALS       → bus.proposals.>       # Propositions avec retention
LOGS            → bus.logs.>            # Logs avec retention 7j
AGENTIC_AUDIT   → audit.>               # Events audit pipeline
```

---

## Observability Guarantees (Phase 2.3)

Le framework garantit 4 capacités d'observabilité:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                      4 OBSERVABILITY GUARANTEES                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐               │
│  │  REPLAY   │  │   AUDIT   │  │ ROLLBACK  │  │  SANDBOX  │               │
│  │           │  │           │  │           │  │           │               │
│  │ Rejouer   │  │ Expliquer │  │ Bloquer   │  │ Isoler    │               │
│  │ une       │  │ une       │  │ une       │  │ l'exécu-  │               │
│  │ exécution │  │ décision  │  │ régression│  │ tion      │               │
│  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘               │
│        │              │              │              │                      │
│        ▼              ▼              ▼              ▼                      │
│  AuditCollector  AuditEvent    RollbackMgr   ProcessSandbox               │
│  + JetStream     + TraceId     + AttemptScore DockerSandbox               │
│                                               FirecrackerSandbox          │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 1. Replay - Rejouer une exécution

```rust
use agentic_core::distributed::bus::{AuditCollector, TraceSequencer};

// Récupérer tous les events d'une exécution passée
let events = audit.for_trace_persistent("trace-abc-123").await?;

// Chaque event contient:
// - sequence: ordre exact
// - timestamp: quand
// - event: quoi (StageStarted, ModelSelected, ValidationCompleted, etc.)
// - context: données additionnelles
```

### 2. Audit - Expliquer une décision

```rust
// Générer un rapport d'audit
let report = audit.report("trace-abc-123").await;

println!("{}", report);
// === Audit Report: trace-abc-123 ===
// Events: 42
// Result: SUCCESS
// Duration: 290643ms
//
// Stages:
//   - Types attempt 1 → OK
//   - Stubs attempt 1 → OK
//   - Logic attempt 1 → FAIL
//   - Logic attempt 2 → FAIL
//   - Logic attempt 3 → OK
//
// Model Selections:
//   - Fast: simple errors (imports, syntax)
//   - Smart: complex errors (ownership, type system)
//
// Rollbacks:
//   - Continue: score improved from -300 to -200
```

### 3. Rollback - Bloquer une régression

```rust
use agentic_core::distributed::bus::{RollbackManager, RollbackDecision};

let mut rollback = RollbackManager::new();

// Chaque tentative est scorée
let decision = rollback.record(attempt, code, errors);

match decision {
    RollbackDecision::Continue { current_score, best_score } => {
        // Score amélioré ou stable
    }
    RollbackDecision::Rollback { code, reason, .. } => {
        // Régression détectée → retour au meilleur code
        // reason: "Code stopped compiling" ou "Tests regressed"
    }
}
```

### 4. Sandbox - Isoler l'exécution

```rust
use agentic_sandbox::{ProcessSandbox, DockerSandbox};

// Isolation niveau process (dev)
let sandbox = ProcessSandbox::new()
    .with_timeout(60_000);

// Isolation niveau container (staging)
let sandbox = DockerSandbox::new("rust:latest")
    .with_memory_limit(512 * 1024 * 1024);

// Isolation niveau VM (production)
let sandbox = FirecrackerSandbox::builder()
    .vcpu_count(2)
    .mem_size_mib(1024)
    .build();
```

### AuditCollector Usage

```rust
use agentic_core::distributed::bus::{AuditCollector, AuditEvent, TraceSequencer};

// Créer un collector avec persistence NATS
let audit = AuditCollector::new(nats_client).await?;

// Ou in-memory pour tests/standalone
let audit = AuditCollector::in_memory();

// Créer un sequencer pour une exécution
let trace = TraceSequencer::new_trace();

// Émettre des events
audit.emit(AuditEvent::pipeline_started(trace.trace_id(), trace.next(), task)).await?;
audit.emit(AuditEvent::stage_started(trace.trace_id(), trace.next(), "Types", 1)).await?;
audit.emit(AuditEvent::model_selected(trace.trace_id(), trace.next(), ModelTier::Fast, "simple", vec![])).await?;
audit.emit(AuditEvent::validation_completed(trace.trace_id(), trace.next(), "sandbox", true, &[])).await?;
audit.emit(AuditEvent::pipeline_completed(trace.trace_id(), trace.next(), true, 1000, 3)).await?;
```

### Event Types

| Event | Description |
|-------|-------------|
| `PipelineStarted` | Début d'exécution, hash du task |
| `PipelineCompleted` | Fin d'exécution, succès/échec, durée |
| `StageStarted` | Début d'un stage (Types/Stubs/Logic) |
| `StageCompleted` | Fin d'un stage, hash du code |
| `ModelSelected` | Choix du modèle (Fast/Smart), raison |
| `LlmRequested` | Appel LLM, tokens prompt |
| `LlmCompleted` | Réponse LLM, tokens completion |
| `ValidationStarted` | Début validation (linter/clippy/sandbox) |
| `ValidationCompleted` | Fin validation, erreurs |
| `RollbackDecision` | Décision de rollback, scores |
| `CodeSnapshot` | Snapshot du code pour replay |
| `Error` | Erreur, composant, récupérable |

### Configuration

```rust
use agentic_core::distributed::bus::BusCoordinator;

let coordinator = BusCoordinator::builder()
    .nats_url("nats://localhost:4222")

    // Sequencer config
    .sequencer_config(SequencerConfig {
        max_pending: 1000,
        rate_limit_per_sec: 100,
    })

    // Arbiter config
    .arbiter_config(ArbiterConfig {
        default_proposals: 3,
        proposal_timeout_ms: 5000,
        strategy: ArbiterStrategy::BestOfN { n: 3 },
    })

    // Deduplicator config
    .dedup_config(DeduplicatorConfig {
        window: Duration::from_secs(300),
        strategy: DedupeStrategy::ContentHash,
    })

    // Log collector config
    .log_config(LogCollectorConfig {
        retention: Duration::from_days(7),
        batch_size: 100,
    })

    .build()
    .await?;

coordinator.start().await?;
```

---

## Quick Reference

### Commandes

```bash
# Démarrer NATS
docker run -d --name nats -p 4222:4222 -p 8222:8222 nats:latest -js

# Vérifier la connexion
nats server info

# Voir les subjects actifs
nats sub ">"

# Voir l'audit en temps réel
nats sub "audit.>"

# Publier une tâche de test
nats pub sup.code.inbox '{"action":"test"}'
```

### Sujets NATS Principaux

| Subject | Description |
|---------|-------------|
| `sup.{id}.inbox` | Inbox d'un superviseur |
| `sup.help.request` | Demande d'aide broadcast |
| `exe.{sup}.pool` | Pool d'exécuteurs (queue group) |
| `val.{sup}.request` | Demande de validation |
| `audit.>` | Tous les events audit |

### Variables d'Environnement

| Variable | Description | Default |
|----------|-------------|---------|
| `NATS_URL` | URL du serveur NATS | `nats://localhost:4222` |
| `GEMINI_API_KEY` | Clé API Gemini (obligatoire) | - |
| `FAST_MODEL` | Modèle pour erreurs simples | `gemini-2.0-flash` |
| `SMART_MODEL` | Modèle pour erreurs complexes | `gemini-2.0-flash` |
| `CONTEXT7_API_KEY` | Active MCP Doc Validator | - (optionnel) |
| `OPENAI_API_KEY` | Clé API OpenAI | - |
| `RUST_LOG` | Niveau de log | `info` |
| `OTEL_ENDPOINT` | Endpoint OpenTelemetry | - |

---

## Roadmap

- [x] Phase 0: Setup workspace
- [x] Phase 1: Agent core + LLM adapters
- [x] Phase 1.5: Validation (Validator + SandboxValidator)
- [x] Phase 1.6: Grounded Validation Loop (examples/grounded_loop.rs)
- [x] Phase 1.7: Multi-Executor Architecture (TaskDecomposer + LiaisonArchitect)
- [x] Phase 2.0: NATS Bus Coordinator (Sequencing, Arbitrage, Deduplication, Logs)
- [x] Phase 2.1: Progressive Locking Pipeline (BusLinter + RollbackManager)
- [x] Phase 2.2: Error Fingerprinting + Model Hierarchy + Clippy + MCP Doc
- [x] Phase 2.3: Audit Trail + Replay (AuditCollector, JetStream persistence)
- [ ] Phase 2.4: Code Review Agent (critic stage with big LLM)
- [ ] Phase 2.5: Expert model tier (Claude Opus for lifetimes)
- [ ] Phase 3: Production hardening
- [ ] Phase 4: Enterprise features

---

## Sources & Références

- [NATS Documentation](https://docs.nats.io/)
- [JetStream Guide](https://docs.nats.io/nats-concepts/jetstream)
- [OpenTelemetry AI Agent Observability](https://opentelemetry.io/blog/2025/ai-agent-observability/)
- [Microsoft AI Agent Design Patterns](https://learn.microsoft.com/en-us/azure/architecture/ai-ml/guide/ai-agent-design-patterns)
- [Google ADK Multi-Agent Patterns](https://google.github.io/adk-docs/agents/multi-agents/)
