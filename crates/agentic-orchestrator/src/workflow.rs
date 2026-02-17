//! Workflow types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use dasein_agentic_core::types::{ResultPayload, ResultStatus};

/// A step in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Agent to execute this step
    pub agent: String,
    /// Action to perform
    pub action: String,
    /// Input data
    #[serde(default)]
    pub inputs: Option<serde_json::Value>,
    /// Steps that must complete before this one
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// A workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Workflow name
    pub name: String,
    /// Steps to execute
    pub steps: Vec<WorkflowStep>,
    /// Optional timeout in milliseconds
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Result of workflow execution.
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    /// Workflow ID
    pub workflow_id: String,
    /// Overall status
    pub status: ResultStatus,
    /// Results from each step (keyed by "agent:action")
    pub results: HashMap<String, ResultPayload>,
    /// Total execution time
    pub total_time_ms: u64,
}
