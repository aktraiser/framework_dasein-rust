//! Core type definitions for the Agentic Framework.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Mode of code execution for the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Always execute generated code
    Always,
    /// Never execute (return code only)
    Never,
    /// Let the LLM decide via markers ([EXECUTE]/[NO_EXECUTE])
    Auto,
    /// Decide based on the action type (default)
    #[default]
    Task,
}

/// Current status of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Agent is ready and waiting for tasks
    #[default]
    Idle,
    /// Agent is processing a task
    Busy,
    /// Agent encountered an error
    Error,
    /// Agent has been stopped
    Stopped,
}

/// Configuration for creating an agent.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Human-readable name for the agent
    pub name: String,
    /// Optional description of the agent's purpose
    pub description: Option<String>,
    /// System prompt defining agent behavior
    pub system_prompt: String,
    /// Code execution mode
    pub execution_mode: ExecutionMode,
    /// Default timeout for tasks in milliseconds
    pub timeout_ms: u64,
    /// Maximum retry attempts for failed tasks
    pub max_retries: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "agent".to_string(),
            description: None,
            system_prompt: "You are a helpful AI assistant.".to_string(),
            execution_mode: ExecutionMode::Task,
            timeout_ms: 30000,
            max_retries: 3,
        }
    }
}

/// Task payload sent to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPayload {
    /// Action to perform (e.g., "generate", "execute", "review")
    pub action: String,
    /// Specification/description of the task
    pub spec: serde_json::Value,
    /// Optional inputs from previous steps
    #[serde(default)]
    pub inputs: Option<serde_json::Value>,
    /// Constraints to apply
    #[serde(default)]
    pub constraints: Vec<String>,
}

/// Status of a task result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    /// Task completed successfully
    Success,
    /// Task failed
    Failure,
    /// Task partially completed
    Partial,
}

/// Type of artifact produced.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    /// Source code
    Code,
    /// File
    File,
    /// Data/JSON
    Data,
    /// Log output
    Log,
}

/// An artifact produced by task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Type of artifact
    pub artifact_type: ArtifactType,
    /// Optional path
    pub path: Option<String>,
    /// Optional content
    pub content: Option<String>,
}

/// Metrics from task execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResultMetrics {
    /// Time taken in milliseconds
    pub execution_time_ms: u64,
    /// Tokens used (if applicable)
    #[serde(default)]
    pub tokens_used: Option<u32>,
}

/// Result of task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultPayload {
    /// Status of the execution
    pub status: ResultStatus,
    /// Result data
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    /// Generated artifacts
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Execution metrics
    pub metrics: ResultMetrics,
    /// Error message if failed
    #[serde(default)]
    pub error: Option<String>,
}

/// Internal state of an agent.
#[derive(Debug, Clone)]
pub struct AgentState {
    /// Current status
    pub status: AgentStatus,
    /// ID of current task (if any)
    pub current_task_id: Option<String>,
    /// Timestamp of last activity
    pub last_activity_at: DateTime<Utc>,
    /// Number of completed tasks
    pub tasks_completed: u64,
    /// Number of failed tasks
    pub tasks_failed: u64,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            status: AgentStatus::Idle,
            current_task_id: None,
            last_activity_at: Utc::now(),
            tasks_completed: 0,
            tasks_failed: 0,
        }
    }
}

/// Performance metrics for an agent.
#[derive(Debug, Clone, Default)]
pub struct AgentMetrics {
    /// Total tasks processed
    pub total_tasks: u64,
    /// Successfully completed tasks
    pub successful_tasks: u64,
    /// Failed tasks
    pub failed_tasks: u64,
    /// Average execution time in milliseconds
    pub average_execution_time_ms: f64,
    /// Total tokens consumed
    pub total_tokens_used: u64,
    /// Time since start in milliseconds
    pub uptime_ms: u64,
}
