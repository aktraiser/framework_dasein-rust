# agentic-orchestrator

Multi-agent coordination and workflow execution.

## Installation

```toml
[dependencies]
agentic-orchestrator = "0.1"
```

## Orchestrator

```rust
use agentic_orchestrator::{Orchestrator, Workflow, WorkflowStep};
use agentic_core::Agent;

// Create orchestrator
let orchestrator = Orchestrator::new();

// Register agents
orchestrator.register("researcher", researcher_agent).await?;
orchestrator.register("writer", writer_agent).await?;
orchestrator.register("reviewer", reviewer_agent).await?;

// Execute single task on specific agent
let result = orchestrator.execute("researcher", task).await?;

// Execute workflow
let workflow = Workflow::new("content-pipeline")
    .add_step(WorkflowStep::new("research", "researcher", research_task))
    .add_step(WorkflowStep::new("write", "writer", write_task).depends_on("research"))
    .add_step(WorkflowStep::new("review", "reviewer", review_task).depends_on("write"));

let results = orchestrator.run_workflow(workflow).await?;
```

## Workflow

DAG-based workflow with dependency resolution.

```rust
pub struct Workflow {
    pub name: String,
    pub steps: Vec<WorkflowStep>,
}

impl Workflow {
    pub fn new(name: impl Into<String>) -> Self;
    pub fn add_step(self, step: WorkflowStep) -> Self;
}
```

## WorkflowStep

```rust
pub struct WorkflowStep {
    pub id: String,
    pub agent_id: String,
    pub task: TaskPayload,
    pub dependencies: Vec<String>,
    pub timeout_ms: Option<u64>,
}

impl WorkflowStep {
    pub fn new(id: &str, agent_id: &str, task: TaskPayload) -> Self;
    pub fn depends_on(self, step_id: &str) -> Self;
    pub fn with_timeout(self, timeout_ms: u64) -> Self;
}
```

## Execution Flow

```
┌─────────────────────────────────────────┐
│              Workflow                    │
│                                         │
│   ┌─────────┐                           │
│   │ Step A  │ (no dependencies)         │
│   └────┬────┘                           │
│        │                                │
│   ┌────▼────┐  ┌─────────┐             │
│   │ Step B  │  │ Step C  │ (parallel)   │
│   └────┬────┘  └────┬────┘             │
│        │            │                   │
│        └─────┬──────┘                   │
│              │                          │
│         ┌────▼────┐                     │
│         │ Step D  │ (waits for B & C)   │
│         └─────────┘                     │
└─────────────────────────────────────────┘
```

## Methods

```rust
impl<L: LLMAdapter, S: Sandbox> Orchestrator<L, S> {
    pub fn new() -> Self;

    pub async fn register(&self, id: &str, agent: Agent<L, S>) -> Result<(), OrchestratorError>;

    pub async fn unregister(&self, id: &str) -> Result<(), OrchestratorError>;

    pub async fn execute(
        &self,
        agent_id: &str,
        task: TaskPayload,
    ) -> Result<ResultPayload, OrchestratorError>;

    pub async fn run_workflow(
        &self,
        workflow: Workflow,
    ) -> Result<HashMap<String, ResultPayload>, OrchestratorError>;

    pub fn list_agents(&self) -> Vec<String>;
}
```

## Errors

```rust
pub enum OrchestratorError {
    AgentNotFound(String),
    AgentAlreadyExists(String),
    WorkflowFailed(String),
    DependencyCycle,
    Timeout,
}
```
