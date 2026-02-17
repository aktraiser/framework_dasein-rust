//! Sub-Workflow Executor - Orchestrates nested workflows (MAF Pattern)
//!
//! Uses dynamic dispatch with `serde_json::Value` for MAF compatibility.
//! Enables hierarchical workflow composition by embedding a complete
//! workflow inside another workflow as a single executor node.
//!
//! # Input Schema
//!
//! Any JSON input that the child workflow accepts.
//!
//! # Output Schema
//!
//! ```json
//! {
//!   "outputs": [...],          // All outputs from child workflow
//!   "superstep_count": 5,
//!   "duration_ms": 1500,
//!   "success": true,
//!   "error": null
//! }
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! // Create a sub-workflow for validation
//! let validation_def = WorkflowBuilder::<Value>::new("validation-subwf")
//!     .set_start("compile")
//!     .add_executor_id("test")
//!     .add_direct_edge("compile", "test")
//!     .build()?;
//!
//! let mut validation_registry = ExecutorRegistry::new();
//! validation_registry.register(CompileValidatorExecutor::new("compile", sandbox.clone()));
//! validation_registry.register(TestValidatorExecutor::new("test", sandbox));
//!
//! let validation_workflow = Workflow::new(validation_def, validation_registry);
//!
//! // Wrap as executor for parent workflow
//! let sub_executor = SubWorkflowExecutor::new("validate", validation_workflow);
//! parent_registry.register(sub_executor);
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::distributed::graph::{
    Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind, Workflow, WorkflowResult,
};

// ============================================================================
// INPUT/OUTPUT TYPES
// ============================================================================

/// Input to a sub-workflow executor (type-safe helper).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubWorkflowInput {
    /// The input to pass to the child workflow (any JSON).
    pub input: Value,
    /// Optional: override the task ID (for tracing).
    #[serde(default)]
    pub task_id: Option<String>,
}

impl SubWorkflowInput {
    /// Create new sub-workflow input.
    pub fn new(input: Value) -> Self {
        Self {
            input,
            task_id: None,
        }
    }

    /// Create from any serializable type.
    pub fn from<T: Serialize>(value: &T) -> Self {
        Self {
            input: serde_json::to_value(value).unwrap(),
            task_id: None,
        }
    }

    /// Set a custom task ID.
    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    /// Convert to JSON Value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

/// Output from a sub-workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubWorkflowOutput {
    /// All outputs yielded by the child workflow (as JSON).
    pub outputs: Vec<Value>,
    /// Number of supersteps executed.
    pub superstep_count: u32,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the child workflow succeeded.
    pub success: bool,
    /// Error message if failed.
    #[serde(default)]
    pub error: Option<String>,
}

impl SubWorkflowOutput {
    /// Create from a WorkflowResult.
    pub fn from_result<T: Serialize>(result: WorkflowResult<T>) -> Self {
        let outputs: Vec<Value> = result
            .outputs
            .iter()
            .filter_map(|o| serde_json::to_value(o).ok())
            .collect();

        Self {
            outputs,
            superstep_count: result.superstep_count,
            duration_ms: result.duration_ms,
            success: result.success,
            error: result.error,
        }
    }

    /// Check if the sub-workflow succeeded.
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Get the first output, if any.
    pub fn first_output(&self) -> Option<&Value> {
        self.outputs.first()
    }

    /// Get the last output (often the final result).
    pub fn last_output(&self) -> Option<&Value> {
        self.outputs.last()
    }

    /// Convert to JSON Value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

// ============================================================================
// SUB-WORKFLOW EXECUTOR
// ============================================================================

/// Graph Executor that runs a nested workflow.
///
/// Uses MAF-style dynamic dispatch with `serde_json::Value`.
/// This enables hierarchical workflow composition.
pub struct SubWorkflowExecutor {
    id: ExecutorId,
    /// The child workflow to execute.
    workflow: Arc<Workflow<Value, String>>,
    /// Whether to propagate child failure as executor error.
    fail_on_child_failure: bool,
    /// Optional description override.
    description: Option<String>,
}

impl SubWorkflowExecutor {
    /// Create a new sub-workflow executor.
    ///
    /// The child workflow must use `Value` as its message type
    /// for MAF compatibility.
    pub fn new(id: impl Into<String>, workflow: Workflow<Value, String>) -> Self {
        Self {
            id: ExecutorId::new(id),
            workflow: Arc::new(workflow),
            fail_on_child_failure: true,
            description: None,
        }
    }

    /// Create from an Arc<Workflow> (for sharing).
    pub fn from_arc(id: impl Into<String>, workflow: Arc<Workflow<Value, String>>) -> Self {
        Self {
            id: ExecutorId::new(id),
            workflow,
            fail_on_child_failure: true,
            description: None,
        }
    }

    /// Set whether to fail the executor when child workflow fails.
    ///
    /// If true (default), a child workflow failure causes an ExecutorError.
    /// If false, the failure is captured in SubWorkflowOutput and the
    /// executor continues (useful for handling failures via edges).
    pub fn fail_on_child_failure(mut self, fail: bool) -> Self {
        self.fail_on_child_failure = fail;
        self
    }

    /// Set a custom description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Get the child workflow ID.
    pub fn child_workflow_id(&self) -> &crate::distributed::graph::WorkflowId {
        self.workflow.id()
    }
}

#[async_trait]
impl Executor for SubWorkflowExecutor {
    // MAF Pattern: All types are Value for dynamic dispatch
    type Input = Value;
    type Message = Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Orchestrator
    }

    fn name(&self) -> &str {
        "Sub-Workflow"
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref().or(Some(
            "Executes a nested workflow as a single orchestration step",
        ))
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Log start event
        ctx.add_event(
            "sub_workflow_start",
            serde_json::json!({
                "child_workflow_id": self.workflow.id().to_string(),
            }),
        )
        .await?;

        // Run the child workflow
        let result = self.workflow.run(input).await.map_err(|e| {
            ExecutorError::new(self.id.clone(), format!("Child workflow error: {}", e))
        })?;

        // Convert to output
        let output = SubWorkflowOutput::from_result(result);

        // Log completion event
        ctx.add_event(
            "sub_workflow_complete",
            serde_json::json!({
                "success": output.success,
                "superstep_count": output.superstep_count,
                "duration_ms": output.duration_ms,
                "output_count": output.outputs.len(),
                "error": output.error.clone(),
            }),
        )
        .await?;

        // Check for failure
        if self.fail_on_child_failure && !output.success {
            let error_msg = output
                .error
                .clone()
                .unwrap_or_else(|| "Sub-workflow failed".to_string());
            return Err(ExecutorError::new(self.id.clone(), error_msg));
        }

        // Yield status output
        let status = if output.success {
            format!(
                "Sub-workflow: COMPLETED ({} outputs, {} supersteps, {}ms)",
                output.outputs.len(),
                output.superstep_count,
                output.duration_ms
            )
        } else {
            format!(
                "Sub-workflow: FAILED after {} supersteps ({})",
                output.superstep_count,
                output.error.as_deref().unwrap_or("unknown error")
            )
        };
        ctx.yield_output(status).await?;

        // Send output as Value to next executors
        ctx.send_message(output.to_value()).await?;

        Ok(())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sub_workflow_input() {
        let input = SubWorkflowInput::new(serde_json::json!({"test": "data"}))
            .with_task_id("task-123");

        assert_eq!(input.task_id, Some("task-123".into()));
        assert_eq!(input.input["test"], "data");
    }

    #[test]
    fn test_sub_workflow_input_from() {
        #[derive(Serialize)]
        struct MyInput {
            name: String,
        }

        let input = SubWorkflowInput::from(&MyInput {
            name: "test".into(),
        });

        assert_eq!(input.input["name"], "test");
    }

    #[test]
    fn test_sub_workflow_output() {
        let output = SubWorkflowOutput {
            outputs: vec![serde_json::json!("out1"), serde_json::json!("out2")],
            superstep_count: 3,
            duration_ms: 100,
            success: true,
            error: None,
        };

        assert!(output.is_success());
        assert_eq!(output.first_output(), Some(&serde_json::json!("out1")));
        assert_eq!(output.last_output(), Some(&serde_json::json!("out2")));
    }

    #[test]
    fn test_sub_workflow_output_failed() {
        let output = SubWorkflowOutput {
            outputs: vec![],
            superstep_count: 2,
            duration_ms: 50,
            success: false,
            error: Some("Something went wrong".into()),
        };

        assert!(!output.is_success());
        assert!(output.first_output().is_none());
        assert_eq!(output.error, Some("Something went wrong".into()));
    }

    #[test]
    fn test_sub_workflow_output_to_value() {
        let output = SubWorkflowOutput {
            outputs: vec![serde_json::json!("test")],
            superstep_count: 1,
            duration_ms: 50,
            success: true,
            error: None,
        };

        let value = output.to_value();
        assert_eq!(value["success"], true);
        assert_eq!(value["superstep_count"], 1);
    }
}
