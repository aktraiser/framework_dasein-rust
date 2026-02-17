//! Orchestrator implementation.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, info, instrument};

use dasein_agentic_core::{
    types::{ResultPayload, ResultStatus, TaskPayload},
    Agent,
};
use dasein_agentic_llm::LLMAdapter;
use dasein_agentic_sandbox::Sandbox;

use crate::{
    error::OrchestratorError,
    workflow::{Workflow, WorkflowResult, WorkflowStep},
};

/// Reference to a registered agent.
#[derive(Debug, Clone)]
pub struct AgentRef {
    /// Agent ID
    pub id: String,
    /// Agent name
    pub name: String,
}

/// Multi-agent orchestrator.
pub struct Orchestrator<L: LLMAdapter + 'static, S: Sandbox + 'static> {
    agents: Arc<RwLock<HashMap<String, Arc<Agent<L, S>>>>>,
    default_timeout_ms: u64,
    max_parallel_tasks: usize,
}

impl<L: LLMAdapter + 'static, S: Sandbox + 'static> Orchestrator<L, S> {
    /// Create a new orchestrator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            default_timeout_ms: 60000,
            max_parallel_tasks: 10,
        }
    }

    /// Set default timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.default_timeout_ms = timeout_ms;
        self
    }

    /// Set max parallel tasks.
    #[must_use]
    pub fn with_max_parallel(mut self, max: usize) -> Self {
        self.max_parallel_tasks = max;
        self
    }

    /// Register an agent.
    pub async fn register_agent(&self, agent: Agent<L, S>) {
        let id = agent.id().to_string();
        let name = agent.name().to_string();

        self.agents
            .write()
            .await
            .insert(id.clone(), Arc::new(agent));

        info!(agent_id = %id, agent_name = %name, "Agent registered");
    }

    /// Unregister an agent.
    pub async fn unregister_agent(&self, agent_id: &str) -> Option<Arc<Agent<L, S>>> {
        self.agents.write().await.remove(agent_id)
    }

    /// List registered agents.
    pub async fn list_agents(&self) -> Vec<AgentRef> {
        self.agents
            .read()
            .await
            .iter()
            .map(|(id, agent)| AgentRef {
                id: id.clone(),
                name: agent.name().to_string(),
            })
            .collect()
    }

    /// Send a task to an agent.
    #[instrument(skip(self, task), fields(agent_id = %agent_id))]
    pub async fn send_task(
        &self,
        agent_id: &str,
        task: TaskPayload,
    ) -> Result<ResultPayload, OrchestratorError> {
        let agents = self.agents.read().await;
        let agent = agents
            .get(agent_id)
            .ok_or_else(|| OrchestratorError::AgentNotFound(agent_id.to_string()))?;

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(self.default_timeout_ms),
            agent.run(task),
        )
        .await
        .map_err(|_| OrchestratorError::Timeout)?
        .map_err(|e| OrchestratorError::AgentError(e.to_string()))?;

        Ok(result)
    }

    /// Execute a workflow.
    #[instrument(skip(self, workflow), fields(workflow_name = %workflow.name))]
    pub async fn execute(&self, workflow: Workflow) -> Result<WorkflowResult, OrchestratorError> {
        let start = std::time::Instant::now();
        let workflow_id = uuid::Uuid::new_v4().to_string();

        info!(workflow_id = %workflow_id, "Starting workflow");

        let mut results: HashMap<String, ResultPayload> = HashMap::new();
        let mut pending_steps: Vec<&WorkflowStep> = workflow.steps.iter().collect();

        while !pending_steps.is_empty() {
            // Find steps ready to execute (dependencies satisfied)
            let ready_steps: Vec<_> = pending_steps
                .iter()
                .filter(|step| step.depends_on.iter().all(|dep| results.contains_key(dep)))
                .copied()
                .collect();

            if ready_steps.is_empty() && !pending_steps.is_empty() {
                return Err(OrchestratorError::CircularDependency);
            }

            // Execute ready steps in parallel
            let mut handles = vec![];

            for step in &ready_steps {
                let step_id = format!("{}:{}", step.agent, step.action);
                let task = TaskPayload {
                    action: step.action.clone(),
                    spec: step.inputs.clone().unwrap_or_else(|| serde_json::json!({})),
                    inputs: None,
                    constraints: vec![],
                };

                let agent_id = self
                    .find_agent_by_name(&step.agent)
                    .await
                    .ok_or_else(|| OrchestratorError::AgentNotFound(step.agent.clone()))?;

                let agents = self.agents.clone();
                let timeout = self.default_timeout_ms;

                handles.push(tokio::spawn(async move {
                    let agents = agents.read().await;
                    let agent = agents.get(&agent_id).unwrap();

                    let result = tokio::time::timeout(
                        std::time::Duration::from_millis(timeout),
                        agent.run(task),
                    )
                    .await;

                    (step_id, result)
                }));
            }

            // Collect results
            for handle in handles {
                let (step_id, result) = handle
                    .await
                    .map_err(|e| OrchestratorError::JoinError(e.to_string()))?;

                let result = result
                    .map_err(|_| OrchestratorError::Timeout)?
                    .map_err(|e| OrchestratorError::AgentError(e.to_string()))?;

                debug!(step_id = %step_id, status = ?result.status, "Step completed");
                results.insert(step_id, result);
            }

            // Remove executed steps
            pending_steps.retain(|step| {
                let step_id = format!("{}:{}", step.agent, step.action);
                !results.contains_key(&step_id)
            });
        }

        // Determine overall status
        let all_success = results.values().all(|r| r.status == ResultStatus::Success);
        let any_success = results.values().any(|r| r.status == ResultStatus::Success);

        let status = if all_success {
            ResultStatus::Success
        } else if any_success {
            ResultStatus::Partial
        } else {
            ResultStatus::Failure
        };

        info!(
            workflow_id = %workflow_id,
            status = ?status,
            duration_ms = %start.elapsed().as_millis(),
            "Workflow completed"
        );

        Ok(WorkflowResult {
            workflow_id,
            status,
            results,
            total_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Find agent by name.
    async fn find_agent_by_name(&self, name: &str) -> Option<String> {
        self.agents
            .read()
            .await
            .iter()
            .find(|(_, agent)| agent.name() == name)
            .map(|(id, _)| id.clone())
    }
}

impl<L: LLMAdapter + 'static, S: Sandbox + 'static> Default for Orchestrator<L, S> {
    fn default() -> Self {
        Self::new()
    }
}
