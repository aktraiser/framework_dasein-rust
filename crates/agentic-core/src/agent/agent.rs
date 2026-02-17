//! Agent implementation - the core autonomous entity.

use std::sync::Arc;

use chrono::Utc;
use regex::Regex;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument};

use dasein_agentic_llm::{LLMAdapter, LLMMessage};
use dasein_agentic_sandbox::Sandbox;

use crate::{
    error::AgentError,
    types::{
        AgentConfig, AgentMetrics, AgentState, AgentStatus, Artifact, ArtifactType, ExecutionMode,
        ResultMetrics, ResultPayload, ResultStatus, TaskPayload,
    },
};

/// Actions that trigger code execution by default.
const EXECUTABLE_ACTIONS: &[&str] = &[
    "execute", "run", "test", "build", "deploy", "install", "validate",
];

/// Actions that do NOT trigger execution by default.
const NON_EXECUTABLE_ACTIONS: &[&str] = &[
    "review", "analyze", "explain", "document", "generate", "refactor", "suggest", "plan",
];

/// Agent - an autonomous entity with LLM + Sandbox.
///
/// The Agent follows the cycle: RECEIVE → THINK → EXECUTE → RESPOND
///
/// - **RECEIVE**: Accept a task payload
/// - **THINK**: Use LLM to reason and generate code
/// - **EXECUTE**: Run code in sandbox (if applicable)
/// - **RESPOND**: Return results
pub struct Agent<L: LLMAdapter, S: Sandbox> {
    id: String,
    config: AgentConfig,
    llm: Arc<L>,
    sandbox: Option<Arc<S>>,
    state: Arc<RwLock<AgentState>>,
    metrics: Arc<RwLock<AgentMetrics>>,
    start_time: Arc<RwLock<Option<std::time::Instant>>>,
}

impl<L: LLMAdapter, S: Sandbox> Agent<L, S> {
    /// Create a new agent.
    ///
    /// # Arguments
    ///
    /// * `config` - Agent configuration
    /// * `llm` - LLM adapter for reasoning
    /// * `sandbox` - Optional sandbox for code execution
    #[must_use]
    pub fn new(config: AgentConfig, llm: L, sandbox: Option<S>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            config,
            llm: Arc::new(llm),
            sandbox: sandbox.map(Arc::new),
            state: Arc::new(RwLock::new(AgentState::default())),
            metrics: Arc::new(RwLock::new(AgentMetrics::default())),
            start_time: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the agent ID.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get the agent name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Get the agent configuration.
    #[must_use]
    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// Start the agent.
    ///
    /// Verifies that all dependencies (LLM, sandbox) are available.
    ///
    /// # Errors
    ///
    /// Returns an error if dependencies are not ready.
    #[instrument(skip(self), fields(agent_name = %self.config.name, agent_id = %self.id))]
    pub async fn start(&self) -> Result<(), AgentError> {
        info!("Starting agent");

        // Check LLM health
        let llm_healthy = self
            .llm
            .health_check()
            .await
            .map_err(|e| AgentError::LLMError(e.to_string()))?;

        if !llm_healthy {
            return Err(AgentError::LLMNotAvailable);
        }
        debug!("LLM health check passed");

        // Check sandbox if needed
        if self.config.execution_mode != ExecutionMode::Never {
            if let Some(ref sandbox) = self.sandbox {
                let sandbox_ready = sandbox.is_ready().await?;
                if !sandbox_ready {
                    return Err(AgentError::SandboxNotReady);
                }
                debug!("Sandbox health check passed");
            }
        }

        // Update state
        {
            let mut state = self.state.write().await;
            state.status = AgentStatus::Idle;
        }

        // Record start time
        {
            let mut start_time = self.start_time.write().await;
            *start_time = Some(std::time::Instant::now());
        }

        info!("Agent started successfully");
        Ok(())
    }

    /// Stop the agent.
    ///
    /// # Errors
    ///
    /// Returns an error if cleanup fails.
    #[instrument(skip(self), fields(agent_name = %self.config.name))]
    pub async fn stop(&self) -> Result<(), AgentError> {
        info!("Stopping agent");

        if let Some(ref sandbox) = self.sandbox {
            sandbox.stop().await?;
        }

        {
            let mut state = self.state.write().await;
            state.status = AgentStatus::Stopped;
        }

        info!("Agent stopped");
        Ok(())
    }

    /// Run a task.
    ///
    /// # Arguments
    ///
    /// * `task` - The task to execute
    ///
    /// # Errors
    ///
    /// Returns an error if execution fails.
    #[instrument(skip(self, task), fields(agent_name = %self.config.name, action = %task.action))]
    pub async fn run(&self, task: TaskPayload) -> Result<ResultPayload, AgentError> {
        let start = std::time::Instant::now();
        let task_id = uuid::Uuid::new_v4().to_string();

        info!(task_id = %task_id, "Starting task");

        // Update state
        {
            let mut state = self.state.write().await;
            state.status = AgentStatus::Busy;
            state.current_task_id = Some(task_id.clone());
        }

        // Increment task counter
        {
            let mut metrics = self.metrics.write().await;
            metrics.total_tasks += 1;
        }

        // Execute the task
        let result = self.execute_task(&task, &task_id, start).await;

        // Update state and metrics
        self.update_state_after_task(&result, start).await;

        match &result {
            Ok(r) => {
                info!(
                    task_id = %task_id,
                    status = ?r.status,
                    duration_ms = %start.elapsed().as_millis(),
                    "Task completed"
                );
            }
            Err(e) => {
                error!(task_id = %task_id, error = %e, "Task failed");
            }
        }

        result
    }

    /// Get the current state.
    pub async fn get_state(&self) -> AgentState {
        self.state.read().await.clone()
    }

    /// Get the current metrics.
    pub async fn get_metrics(&self) -> AgentMetrics {
        let mut metrics = self.metrics.read().await.clone();

        // Calculate uptime
        if let Some(start) = *self.start_time.read().await {
            metrics.uptime_ms = start.elapsed().as_millis() as u64;
        }

        metrics
    }

    // =========================================================================
    // Private Methods
    // =========================================================================

    /// Execute a task.
    async fn execute_task(
        &self,
        task: &TaskPayload,
        task_id: &str,
        start: std::time::Instant,
    ) -> Result<ResultPayload, AgentError> {
        // 1. THINK: Ask LLM to generate response
        debug!(task_id = %task_id, "THINK: Generating LLM response");
        let messages = self.build_messages(task);
        let llm_response = self
            .llm
            .generate(&messages)
            .await
            .map_err(|e| AgentError::LLMError(e.to_string()))?;

        debug!(
            task_id = %task_id,
            tokens = llm_response.tokens_used.total,
            "LLM response received"
        );

        // 2. EXTRACT: Extract code from response
        let code = self.extract_code(&llm_response.content);
        debug!(
            task_id = %task_id,
            has_code = code.is_some(),
            "Code extraction complete"
        );

        // 3. DECIDE: Should we execute?
        let should_execute = self.should_execute(task, &llm_response.content);

        if !should_execute || code.is_none() {
            // Return response without execution
            return Ok(ResultPayload {
                status: ResultStatus::Success,
                data: Some(serde_json::json!({
                    "output": llm_response.content,
                    "code": code,
                    "executed": false,
                })),
                artifacts: vec![],
                metrics: ResultMetrics {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    tokens_used: Some(llm_response.tokens_used.total),
                },
                error: None,
            });
        }

        // 4. EXECUTE: Run in sandbox
        debug!(task_id = %task_id, "EXECUTE: Running code in sandbox");
        let sandbox = self.sandbox.as_ref().ok_or(AgentError::SandboxRequired)?;

        let code_to_run = code.unwrap();
        let exec_result = sandbox.execute(&code_to_run).await?;

        debug!(
            task_id = %task_id,
            exit_code = exec_result.exit_code,
            "Sandbox execution complete"
        );

        // 5. RESPOND: Build result
        Ok(ResultPayload {
            status: if exec_result.exit_code == 0 {
                ResultStatus::Success
            } else {
                ResultStatus::Failure
            },
            data: Some(serde_json::json!({
                "output": exec_result.stdout,
                "stderr": exec_result.stderr,
                "code": code_to_run,
                "executed": true,
                "exit_code": exec_result.exit_code,
            })),
            artifacts: exec_result
                .artifacts
                .into_iter()
                .map(|a| Artifact {
                    artifact_type: ArtifactType::File,
                    path: Some(a.path),
                    content: Some(a.content),
                })
                .collect(),
            metrics: ResultMetrics {
                execution_time_ms: start.elapsed().as_millis() as u64,
                tokens_used: Some(llm_response.tokens_used.total),
            },
            error: if exec_result.exit_code != 0 {
                Some(exec_result.stderr)
            } else {
                None
            },
        })
    }

    /// Build messages for the LLM.
    fn build_messages(&self, task: &TaskPayload) -> Vec<LLMMessage> {
        let system_prompt = self.build_system_prompt();
        let user_prompt = self.build_user_prompt(task);

        vec![
            LLMMessage::system(system_prompt),
            LLMMessage::user(user_prompt),
        ]
    }

    /// Build the system prompt.
    fn build_system_prompt(&self) -> String {
        let execution_instructions = match self.config.execution_mode {
            ExecutionMode::Always => "The code you generate will ALWAYS be executed in a sandbox.",
            ExecutionMode::Never => {
                "The code you generate will NOT be executed. Provide explanations."
            }
            ExecutionMode::Auto => {
                "Use [EXECUTE] or [NO_EXECUTE] markers to indicate if code should run."
            }
            ExecutionMode::Task => "Code execution depends on the action type.",
        };

        format!(
            "{}\n\n\
            ## Instructions\n\n\
            You are an agent that generates CODE to accomplish tasks.\n\n\
            RULES:\n\
            1. Respond with executable code when appropriate\n\
            2. Code must be complete and functional\n\
            3. Use println!() or console.log() for output\n\
            4. Handle errors properly\n\n\
            {}\n\n\
            FORMAT:\n\
            ```typescript\n// or rust\n// Your code here\n```",
            self.config.system_prompt, execution_instructions
        )
    }

    /// Build the user prompt.
    fn build_user_prompt(&self, task: &TaskPayload) -> String {
        let mut parts = vec![
            format!("## Action: {}", task.action),
            format!(
                "## Specification:\n{}",
                serde_json::to_string_pretty(&task.spec).unwrap_or_default()
            ),
        ];

        if let Some(inputs) = &task.inputs {
            parts.push(format!(
                "## Inputs:\n{}",
                serde_json::to_string_pretty(inputs).unwrap_or_default()
            ));
        }

        if !task.constraints.is_empty() {
            parts.push(format!(
                "## Constraints:\n- {}",
                task.constraints.join("\n- ")
            ));
        }

        parts.join("\n\n")
    }

    /// Extract code from markdown code blocks.
    fn extract_code(&self, content: &str) -> Option<String> {
        let re =
            Regex::new(r"```(?:rust|typescript|javascript|python|bash|sh)\n([\s\S]*?)```").ok()?;

        re.captures(content)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
    }

    /// Determine if code should be executed.
    fn should_execute(&self, task: &TaskPayload, content: &str) -> bool {
        match self.config.execution_mode {
            ExecutionMode::Always => true,
            ExecutionMode::Never => false,
            ExecutionMode::Auto => {
                // Check for markers
                if content.contains("[NO_EXECUTE]") || content.contains("<!-- NO_EXECUTE -->") {
                    return false;
                }
                if content.contains("[EXECUTE]") || content.contains("<!-- EXECUTE -->") {
                    return true;
                }
                false
            }
            ExecutionMode::Task => {
                let action = task.action.to_lowercase();

                // Check executable actions
                if EXECUTABLE_ACTIONS.iter().any(|a| action.contains(a)) {
                    return true;
                }

                // Check non-executable actions
                if NON_EXECUTABLE_ACTIONS.iter().any(|a| action.contains(a)) {
                    return false;
                }

                // Default: don't execute (principle of least privilege)
                false
            }
        }
    }

    /// Update state after task completion.
    async fn update_state_after_task(
        &self,
        result: &Result<ResultPayload, AgentError>,
        start: std::time::Instant,
    ) {
        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Update state
        {
            let mut state = self.state.write().await;
            state.current_task_id = None;
            state.last_activity_at = Utc::now();

            match result {
                Ok(r) if r.status == ResultStatus::Success => {
                    state.status = AgentStatus::Idle;
                    state.tasks_completed += 1;
                }
                _ => {
                    state.status = AgentStatus::Error;
                    state.tasks_failed += 1;
                }
            }
        }

        // Update metrics
        {
            let mut metrics = self.metrics.write().await;

            match result {
                Ok(r) => {
                    if r.status == ResultStatus::Success {
                        metrics.successful_tasks += 1;
                    } else {
                        metrics.failed_tasks += 1;
                    }

                    // Rolling average
                    let n = metrics.total_tasks as f64;
                    metrics.average_execution_time_ms =
                        (metrics.average_execution_time_ms * (n - 1.0) + elapsed_ms as f64) / n;

                    if let Some(tokens) = r.metrics.tokens_used {
                        metrics.total_tokens_used += u64::from(tokens);
                    }
                }
                Err(_) => {
                    metrics.failed_tasks += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests would go here with mock LLM and sandbox
}
