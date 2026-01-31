# Protocole NATS - Agentic-RS

> Spécification complète des subjects et messages NATS

---

## Vue d'Ensemble

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          TOPOLOGIE NATS                                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│                         ┌─────────────────┐                                  │
│                         │  NATS CLUSTER   │                                  │
│                         │   + JetStream   │                                  │
│                         └────────┬────────┘                                  │
│                                  │                                           │
│    ┌─────────────────────────────┼─────────────────────────────┐            │
│    │                             │                             │            │
│    ▼                             ▼                             ▼            │
│  sup.>                        exe.>                         val.>           │
│  Superviseurs                 Exécuteurs                    Validateurs     │
│                                                                              │
│    │                             │                             │            │
│    └─────────────────────────────┼─────────────────────────────┘            │
│                                  │                                           │
│                                  ▼                                           │
│                              audit.>                                         │
│                         (JetStream persisté)                                 │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Hiérarchie des Subjects

```
agentic.                                    # Namespace racine (optionnel)
│
├── sup.                                    # SUPERVISEURS
│   ├── {sup_id}.inbox                      # Inbox du superviseur
│   ├── {sup_id}.outbox                     # Résultats du superviseur
│   ├── {sup_id}.status                     # Statut/heartbeat
│   ├── {sup_id}.metrics                    # Métriques
│   │
│   ├── help.                               # COORDINATION INTER-SUPERVISEURS
│   │   ├── request                         # Demande d'aide (broadcast)
│   │   └── offer                           # Offre de ressources
│   │
│   └── allocate.                           # ALLOCATION D'EXÉCUTEURS
│       └── {from_sup}.{to_sup}             # Confirmation d'allocation
│
├── exe.                                    # EXÉCUTEURS
│   ├── {sup_id}.pool                       # Pool d'exécuteurs (queue group)
│   ├── {sup_id}.{exe_id}.task              # Tâche spécifique
│   ├── {sup_id}.{exe_id}.result            # Résultat
│   ├── {sup_id}.{exe_id}.status            # Statut
│   └── {sup_id}.{exe_id}.cancel            # Annulation
│
├── val.                                    # VALIDATEURS
│   ├── {sup_id}.request                    # Demande de validation
│   ├── {sup_id}.result                     # Résultat de validation
│   └── {sup_id}.status                     # Statut
│
├── workflow.                               # WORKFLOWS
│   ├── {workflow_id}.start                 # Démarrage
│   ├── {workflow_id}.step.{step_id}        # Étape
│   ├── {workflow_id}.step.{step_id}.result # Résultat étape
│   ├── {workflow_id}.complete              # Fin
│   └── {workflow_id}.cancel                # Annulation
│
├── audit.                                  # AUDIT (JetStream)
│   ├── sup.{sup_id}                        # Events superviseur
│   ├── exe.{sup_id}.{exe_id}               # Events exécuteur
│   ├── val.{sup_id}                        # Events validateur
│   ├── workflow.{workflow_id}              # Events workflow
│   └── system                              # Events système
│
└── system.                                 # SYSTÈME
    ├── discovery                           # Découverte d'agents
    ├── health                              # Health checks
    └── shutdown                            # Arrêt coordonné
```

---

## Messages Types

### Enveloppe Standard

Tous les messages suivent cette structure:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Métadonnées
    pub metadata: MessageMetadata,

    /// Routing
    pub routing: RoutingInfo,

    /// Contenu
    pub payload: MessagePayload,

    /// Contrôle
    pub control: ControlSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMetadata {
    /// ID unique du message
    pub id: String,

    /// Timestamp de création
    pub timestamp: DateTime<Utc>,

    /// Version du protocole
    pub version: String,  // "1.0"

    /// Type de message
    pub message_type: MessageType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingInfo {
    /// Expéditeur
    pub from: AgentRef,

    /// Destinataire
    pub to: AgentRef,

    /// ID de corrélation (pour request/reply)
    pub correlation_id: Option<String>,

    /// ID du message parent (pour chaînage)
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlSettings {
    /// Priorité (0 = normal, >0 = haute)
    pub priority: i32,

    /// Time-to-live en ms
    pub ttl_ms: u64,

    /// ID de la tâche racine
    pub task_id: String,

    /// ID de trace (OpenTelemetry)
    pub trace_id: String,

    /// ID de span
    pub span_id: String,

    /// Span parent
    pub parent_span_id: Option<String>,
}
```

### Types de Messages

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePayload {
    // === TÂCHES ===
    Task(TaskMessage),
    TaskResult(TaskResultMessage),
    TaskCancel(TaskCancelMessage),

    // === COORDINATION ===
    HelpRequest(HelpRequestMessage),
    HelpOffer(HelpOfferMessage),
    AllocationGrant(AllocationGrantMessage),
    AllocationRelease(AllocationReleaseMessage),

    // === VALIDATION ===
    ValidationRequest(ValidationRequestMessage),
    ValidationResult(ValidationResultMessage),

    // === WORKFLOW ===
    WorkflowStart(WorkflowStartMessage),
    WorkflowStepComplete(WorkflowStepCompleteMessage),
    WorkflowComplete(WorkflowCompleteMessage),

    // === SYSTÈME ===
    Heartbeat(HeartbeatMessage),
    Discovery(DiscoveryMessage),
    Shutdown(ShutdownMessage),

    // === AUDIT ===
    AuditEvent(AuditEventMessage),
}
```

---

## Messages Détaillés

### Task Messages

#### TaskMessage

```rust
/// Envoyé sur: sup.{sup_id}.inbox ou exe.{sup_id}.pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMessage {
    /// ID unique de la tâche
    pub task_id: String,

    /// Action à effectuer
    pub action: String,

    /// Spécification de la tâche
    pub spec: serde_json::Value,

    /// Inputs optionnels (résultats d'étapes précédentes)
    pub inputs: Option<serde_json::Value>,

    /// Contraintes
    pub constraints: Vec<String>,

    /// Contexte d'exécution
    pub context: ExecutionContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    /// Timeout restant en ms
    pub timeout_remaining_ms: u64,

    /// Budget tokens restant
    pub token_budget: Option<u32>,

    /// Profondeur dans l'arbre d'appels
    pub depth: usize,

    /// Chemin complet (pour debug)
    pub path: Vec<String>,

    /// Variables partagées
    pub variables: HashMap<String, serde_json::Value>,

    /// Capabilities autorisées
    pub allowed_capabilities: Vec<Capability>,
}
```

**Exemple JSON:**
```json
{
  "metadata": {
    "id": "msg-abc123",
    "timestamp": "2025-01-23T10:30:00Z",
    "version": "1.0",
    "message_type": "task"
  },
  "routing": {
    "from": { "id": "client-001", "role": "client" },
    "to": { "id": "sup-code-001", "role": "supervisor" },
    "correlation_id": null,
    "parent_id": null
  },
  "payload": {
    "type": "task",
    "task_id": "task-xyz789",
    "action": "generate",
    "spec": {
      "task": "Write a fibonacci function in Rust",
      "language": "rust"
    },
    "inputs": null,
    "constraints": ["no_unsafe", "must_be_pure"],
    "context": {
      "timeout_remaining_ms": 30000,
      "token_budget": 4096,
      "depth": 0,
      "path": ["client-001"],
      "variables": {},
      "allowed_capabilities": ["code_generation"]
    }
  },
  "control": {
    "priority": 0,
    "ttl_ms": 60000,
    "task_id": "task-xyz789",
    "trace_id": "trace-111",
    "span_id": "span-001",
    "parent_span_id": null
  }
}
```

#### TaskResultMessage

```rust
/// Envoyé sur: sup.{sup_id}.outbox ou exe.{sup_id}.{exe_id}.result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultMessage {
    /// ID de la tâche
    pub task_id: String,

    /// Statut
    pub status: TaskStatus,

    /// Données résultantes
    pub data: Option<serde_json::Value>,

    /// Artefacts générés
    pub artifacts: Vec<Artifact>,

    /// Métriques d'exécution
    pub metrics: ExecutionMetrics,

    /// Erreur si échec
    pub error: Option<TaskError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    Success,
    Failure,
    Partial,
    Cancelled,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetrics {
    pub total_time_ms: u64,
    pub llm_time_ms: u64,
    pub execution_time_ms: u64,
    pub tokens_prompt: u32,
    pub tokens_completion: u32,
    pub retries: u32,
}
```

---

### Coordination Messages

#### HelpRequestMessage

```rust
/// Envoyé sur: sup.help.request (broadcast)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpRequestMessage {
    /// Superviseur demandeur
    pub from_supervisor: String,

    /// Raison de la demande
    pub reason: HelpReason,

    /// Nombre d'exécuteurs souhaités
    pub executors_needed: usize,

    /// Capabilities requises
    pub required_capabilities: Vec<Capability>,

    /// Durée estimée en ms
    pub estimated_duration_ms: u64,

    /// Priorité de la demande
    pub priority: i32,

    /// Deadline (optionnel)
    pub deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HelpReason {
    /// Queue trop longue
    Overloaded { queue_depth: usize, avg_wait_ms: u64 },

    /// Besoin d'une capability spécifique
    SpecializedTask { capability: Capability },

    /// Deadline serrée
    UrgentDeadline { remaining_ms: u64 },

    /// Burst de trafic temporaire
    TrafficBurst { requests_per_sec: f64 },
}
```

**Exemple JSON:**
```json
{
  "payload": {
    "type": "help_request",
    "from_supervisor": "sup-code-001",
    "reason": {
      "type": "overloaded",
      "queue_depth": 15,
      "avg_wait_ms": 5000
    },
    "executors_needed": 2,
    "required_capabilities": ["code_generation", "code_execution"],
    "estimated_duration_ms": 60000,
    "priority": 1,
    "deadline": null
  }
}
```

#### HelpOfferMessage

```rust
/// Envoyé sur: sup.help.offer (reply à help.request)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpOfferMessage {
    /// Superviseur offrant
    pub from_supervisor: String,

    /// Superviseur demandeur (à qui on répond)
    pub to_supervisor: String,

    /// Exécuteurs disponibles
    pub available_executors: Vec<ExecutorInfo>,

    /// Durée max de prêt
    pub max_duration_ms: u64,

    /// Conditions
    pub conditions: Option<LendingConditions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorInfo {
    pub id: String,
    pub capabilities: Vec<Capability>,
    pub current_load: f32,  // 0.0 - 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LendingConditions {
    /// Peut être rappelé avant la fin
    pub revocable: bool,

    /// Priorité minimale des tâches acceptées
    pub min_priority: i32,

    /// Capabilities restreintes
    pub restricted_capabilities: Vec<Capability>,
}
```

#### AllocationGrantMessage

```rust
/// Envoyé sur: sup.allocate.{from}.{to}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationGrantMessage {
    /// ID de l'allocation
    pub lease_id: String,

    /// Superviseur prêteur
    pub from_supervisor: String,

    /// Superviseur emprunteur
    pub to_supervisor: String,

    /// IDs des exécuteurs alloués
    pub executor_ids: Vec<String>,

    /// Nouveaux subjects pour les exécuteurs
    pub executor_subjects: HashMap<String, String>,

    /// Expiration
    pub expires_at: DateTime<Utc>,

    /// Conditions
    pub conditions: LendingConditions,
}
```

---

### Validation Messages

#### ValidationRequestMessage

```rust
/// Envoyé sur: val.{sup_id}.request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRequestMessage {
    /// ID de la tâche
    pub task_id: String,

    /// Output à valider
    pub output: serde_json::Value,

    /// Contexte original
    pub original_task: TaskMessage,

    /// Règles spécifiques à appliquer
    pub rules: Option<Vec<String>>,

    /// Tentative actuelle (pour retry)
    pub attempt: u32,
}
```

#### ValidationResultMessage

```rust
/// Envoyé sur: val.{sup_id}.result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResultMessage {
    /// ID de la tâche
    pub task_id: String,

    /// Résultat global
    pub passed: bool,

    /// Résultat par règle
    pub rule_results: Vec<RuleResult>,

    /// Score global (0-100)
    pub score: Option<u32>,

    /// Feedback si échec
    pub feedback: Option<String>,

    /// Action recommandée
    pub recommended_action: ValidationAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule: String,
    pub passed: bool,
    pub message: Option<String>,
    pub severity: Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationAction {
    Accept,
    Retry { feedback: String },
    Reject { reason: String },
    Escalate { to: String },
}
```

---

### Audit Messages

#### AuditEventMessage

```rust
/// Envoyé sur: audit.{category}.{agent_id}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEventMessage {
    /// ID de l'event
    pub event_id: String,

    /// Timestamp précis
    pub timestamp: DateTime<Utc>,

    /// Trace ID (pour corrélation)
    pub trace_id: String,

    /// Span ID
    pub span_id: String,

    /// Parent span
    pub parent_span_id: Option<String>,

    /// Agent source
    pub agent: AgentRef,

    /// Type d'event
    pub event_type: String,

    /// Niveau
    pub level: AuditLevel,

    /// Données de l'event
    pub data: serde_json::Value,

    /// Tags pour filtrage
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
```

**Exemple JSON:**
```json
{
  "event_id": "evt-12345",
  "timestamp": "2025-01-23T10:30:00.123Z",
  "trace_id": "trace-abc",
  "span_id": "span-xyz",
  "parent_span_id": "span-parent",
  "agent": {
    "id": "exe-code-001",
    "role": "executor"
  },
  "event_type": "llm_request_completed",
  "level": "info",
  "data": {
    "model": "gemini-2.0-flash",
    "tokens_prompt": 150,
    "tokens_completion": 500,
    "latency_ms": 1234
  },
  "tags": {
    "supervisor": "sup-code-001",
    "task_id": "task-xyz"
  }
}
```

---

## Patterns de Communication

### 1. Request-Reply Simple

```
┌────────┐                    ┌────────────┐                    ┌──────────┐
│ Client │                    │ Supervisor │                    │ Executor │
└───┬────┘                    └─────┬──────┘                    └────┬─────┘
    │                               │                                │
    │ ──── Task (sup.X.inbox) ────► │                                │
    │                               │                                │
    │                               │ ──── Task (exe.X.pool) ──────► │
    │                               │                                │
    │                               │ ◄──── Result (reply) ───────── │
    │                               │                                │
    │ ◄──── Result (reply) ──────── │                                │
    │                               │                                │
```

### 2. Fan-Out avec Aggregation

```
┌────────────┐                           ┌─────────────────────────┐
│ Supervisor │                           │ Executor Pool (3 exe)   │
└─────┬──────┘                           └───────────┬─────────────┘
      │                                              │
      │ ── Task 1 (exe.X.pool, queue group) ───────► │ → exe-1
      │ ── Task 2 (exe.X.pool, queue group) ───────► │ → exe-2
      │ ── Task 3 (exe.X.pool, queue group) ───────► │ → exe-3
      │                                              │
      │ ◄───────────── Results ────────────────────── │
      │                                              │
      │ (Supervisor agrège les 3 résultats)          │
      │                                              │
```

### 3. Allocation Dynamique

```
┌───────────┐              ┌──────┐              ┌───────────┐
│ Sup-Code  │              │ NATS │              │ Sup-Data  │
│ (busy)    │              │      │              │ (idle)    │
└─────┬─────┘              └──┬───┘              └─────┬─────┘
      │                       │                        │
      │ ── HelpRequest ─────► │ ── broadcast ────────► │
      │    (sup.help.request) │                        │
      │                       │                        │
      │                       │ ◄──── HelpOffer ────── │
      │ ◄──────────────────── │      (sup.help.offer)  │
      │                       │                        │
      │ ── AllocationGrant ──►│ ─────────────────────► │
      │    (sup.allocate.     │                        │
      │     data.code)        │                        │
      │                       │                        │
      │ ═══════════════════════════════════════════════│
      │   exe-data-1 et exe-data-2 répondent           │
      │   maintenant sur exe.code.pool                 │
      │ ═══════════════════════════════════════════════│
      │                       │                        │
```

### 4. Workflow Multi-Étapes

```
┌──────────────────────────────────────────────────────────────────┐
│                     Workflow: "feature-impl"                      │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  workflow.{id}.start                                              │
│       │                                                           │
│       ▼                                                           │
│  ┌─────────┐    workflow.{id}.step.plan.result    ┌─────────┐   │
│  │  PLAN   │ ─────────────────────────────────────►│ IMPLEMENT│   │
│  │(sup-code)│                                      │(sup-code)│   │
│  └─────────┘                                       └────┬────┘   │
│                                                         │        │
│                    workflow.{id}.step.implement.result  │        │
│                                                         ▼        │
│  ┌─────────┐    workflow.{id}.step.test.result    ┌─────────┐   │
│  │  REVIEW │ ◄────────────────────────────────────│  TEST   │   │
│  │(sup-rev)│                                      │(sup-code)│   │
│  └────┬────┘                                       └─────────┘   │
│       │                                                           │
│       │ (si passed)                                               │
│       ▼                                                           │
│  workflow.{id}.complete                                           │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

---

## Configuration JetStream

### Streams

```bash
# Stream pour l'audit (persisté)
nats stream add AUDIT \
  --subjects "audit.>" \
  --storage file \
  --retention limits \
  --max-msgs 10000000 \
  --max-bytes 10GB \
  --max-age 30d \
  --replicas 3

# Stream pour les workflows (persisté)
nats stream add WORKFLOWS \
  --subjects "workflow.>" \
  --storage file \
  --retention limits \
  --max-msgs 1000000 \
  --max-age 7d \
  --replicas 3
```

### Consumers

```bash
# Consumer pour audit collector
nats consumer add AUDIT audit-collector \
  --deliver all \
  --ack explicit \
  --max-deliver 3 \
  --filter "audit.>"

# Consumer par superviseur
nats consumer add AUDIT sup-code-audit \
  --deliver all \
  --filter "audit.sup.sup-code-001"
```

---

## Bonnes Pratiques

### 1. Nommage des IDs

```
Format: {type}-{domain}-{instance}

Exemples:
  sup-code-001      # Superviseur code, instance 1
  exe-code-001-01   # Exécuteur 1 du superviseur code-001
  val-code-001      # Validateur du superviseur code-001
  task-abc123       # Tâche (UUID court)
  trace-xyz789      # Trace (UUID)
```

### 2. TTL et Timeouts

```
Recommandations:
  - Message TTL: 60s (tâches normales)
  - Message TTL: 300s (tâches longues)
  - Request timeout: 30s
  - Heartbeat interval: 5s
  - Allocation lease: 60-300s
```

### 3. Queue Groups

```
Toujours utiliser des queue groups pour:
  - exe.{sup}.pool → permet load balancing automatique
  - sup.help.request → un seul superviseur répond

Éviter les queue groups pour:
  - audit.> → tous les collectors doivent recevoir
  - system.shutdown → tous les agents doivent recevoir
```

### 4. Idempotence

```rust
// Toujours inclure un ID de message pour déduplication
pub struct Message {
    pub id: String,  // Généré côté client, pas serveur
    // ...
}

// Le receiver peut déduper:
if seen_ids.contains(&msg.id) {
    return; // Déjà traité
}
seen_ids.insert(msg.id.clone());
```

---

## Debugging

### Voir tous les messages

```bash
# Tous les messages
nats sub ">"

# Messages d'un superviseur
nats sub "sup.sup-code-001.>"

# Messages d'aide
nats sub "sup.help.>"

# Audit en temps réel
nats sub "audit.>"
```

### Publier un message de test

```bash
# Tâche simple
nats pub sup.sup-code-001.inbox '{
  "metadata": {"id": "test-1", "timestamp": "2025-01-23T10:00:00Z", "version": "1.0", "message_type": "task"},
  "routing": {"from": {"id": "test", "role": "client"}, "to": {"id": "sup-code-001", "role": "supervisor"}},
  "payload": {"type": "task", "task_id": "task-test", "action": "generate", "spec": {"task": "test"}},
  "control": {"priority": 0, "ttl_ms": 60000, "task_id": "task-test", "trace_id": "trace-1", "span_id": "span-1"}
}'
```

### Vérifier les streams

```bash
# Info stream
nats stream info AUDIT

# Messages dans le stream
nats stream view AUDIT --last 10
```
