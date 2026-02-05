//! WorkflowAgent - Agent that delegates to a Workflow.
//!
//! `WorkflowAgent` wraps a `Workflow` and exposes it via the `Agent` interface.
//! This enables using workflows wherever agents are expected.
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::{Workflow, WorkflowBuilder};
//! use agentic_core::distributed::graph::agent::{WorkflowAgent, AgentThread, AgentExt};
//!
//! // Create a workflow
//! let workflow = WorkflowBuilder::<String>::new("code-gen")
//!     .set_start("generator")
//!     .add_direct_edge("generator", "validator")
//!     .build()?;
//!
//! // Wrap as agent
//! let agent = WorkflowAgent::new("code-agent", workflow);
//!
//! // Use like any agent
//! let mut thread = agent.new_thread();
//! let response = agent.invoke("Write fibonacci", &mut thread).await?;
//! ```
//!
//! # workflow.as_agent()
//!
//! You can also convert a workflow to an agent directly:
//!
//! ```rust,ignore
//! let agent = workflow.as_agent("My Workflow Agent");
//! ```

use async_trait::async_trait;
use futures::Stream;
use serde::{de::DeserializeOwned, Serialize};
use std::pin::Pin;
use std::sync::Arc;

use super::super::workflow::{Workflow, WorkflowStreamEvent};
use super::error::AgentError;
use super::response::{AgentChunk, AgentResponse};
use super::thread::AgentThread;
use super::tools::Tool;
use super::trait_def::Agent;
use super::types::{AgentId, ChatMessage};

// ============================================================================
// WORKFLOW AGENT
// ============================================================================

/// An agent that delegates to a workflow.
///
/// The workflow is executed with the last user message as input.
/// Workflow outputs are collected and returned as the agent response.
pub struct WorkflowAgent<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    TOutput: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    /// Agent identifier.
    id: AgentId,

    /// Human-readable name.
    name: String,

    /// Optional description.
    description: Option<String>,

    /// The underlying workflow.
    workflow: Arc<Workflow<TMessage, TOutput>>,

    /// System prompt (optional context for the workflow).
    system_prompt: Option<String>,
}

impl<TMessage, TOutput> WorkflowAgent<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    TOutput: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    /// Create a new WorkflowAgent.
    pub fn new(name: impl Into<String>, workflow: Workflow<TMessage, TOutput>) -> Self {
        let name = name.into();
        Self {
            id: AgentId::new(&name),
            name,
            description: None,
            workflow: Arc::new(workflow),
            system_prompt: None,
        }
    }

    /// Create with a shared workflow.
    pub fn from_arc(name: impl Into<String>, workflow: Arc<Workflow<TMessage, TOutput>>) -> Self {
        let name = name.into();
        Self {
            id: AgentId::new(&name),
            name,
            description: None,
            workflow,
            system_prompt: None,
        }
    }

    /// Set a custom ID.
    pub fn with_id(mut self, id: impl Into<AgentId>) -> Self {
        self.id = id.into();
        self
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Get the underlying workflow.
    pub fn workflow(&self) -> &Workflow<TMessage, TOutput> {
        &self.workflow
    }

    /// Get workflow ID.
    pub fn workflow_id(&self) -> &super::super::types::WorkflowId {
        self.workflow.id()
    }
}

#[async_trait]
impl<TMessage, TOutput> Agent for WorkflowAgent<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    TOutput: Serialize + DeserializeOwned + Clone + Send + Sync + ToString + 'static,
{
    fn id(&self) -> &AgentId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    fn tools(&self) -> &[Tool] {
        &[] // Workflows don't expose tools directly
    }

    fn supports_streaming(&self) -> bool {
        true // Workflow has run_stream()
    }

    async fn run(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError> {
        // Add messages to thread
        thread.add_messages(messages.clone());

        // Extract input from last user message
        let input = messages
            .iter()
            .rev()
            .find(|m| m.is_user())
            .map(|m| m.content.clone())
            .unwrap_or_default();

        // Run workflow
        let result = self
            .workflow
            .run(input.clone())
            .await
            .map_err(|e| AgentError::workflow(e.to_string()))?;

        // Build response content from outputs
        let content = if result.outputs.is_empty() {
            format!("Workflow completed ({} supersteps)", result.superstep_count)
        } else {
            result
                .outputs
                .iter()
                .map(|o| o.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };

        // Add response to thread
        thread.add_message(ChatMessage::assistant(&content));

        Ok(AgentResponse::with_messages(content, thread.messages.clone()))
    }

    fn run_stream<'a>(
        &'a self,
        messages: Vec<ChatMessage>,
        thread: &'a mut AgentThread,
    ) -> Pin<Box<dyn Stream<Item = AgentChunk> + Send + 'a>> {
        Box::pin(async_stream::stream! {
            // Add messages to thread
            for msg in messages.clone() {
                thread.add_message(msg);
            }

            // Extract input
            let input = messages
                .iter()
                .rev()
                .find(|m| m.is_user())
                .map(|m| m.content.clone())
                .unwrap_or_default();

            // Get workflow stream
            let mut stream = self.workflow.run_stream(input);

            use futures::StreamExt;
            let mut accumulated_output = String::new();

            while let Some(event) = stream.next().await {
                match event {
                    WorkflowStreamEvent::Started { workflow_id, .. } => {
                        yield AgentChunk::content(format!("[Workflow {} started]\n", workflow_id));
                    }
                    WorkflowStreamEvent::SuperstepStarted { superstep, .. } => {
                        yield AgentChunk::content(format!("[Superstep {}]\n", superstep));
                    }
                    WorkflowStreamEvent::Output { data, .. } => {
                        let output_str = serde_json::to_string(&data).unwrap_or_default();
                        accumulated_output.push_str(&output_str);
                        accumulated_output.push('\n');
                        yield AgentChunk::content(output_str);
                    }
                    WorkflowStreamEvent::Completed { duration_ms, superstep_count, .. } => {
                        yield AgentChunk::content(
                            format!("\n[Completed in {}ms, {} supersteps]", duration_ms, superstep_count)
                        );
                        yield AgentChunk::done();
                    }
                    WorkflowStreamEvent::Failed { error, .. } => {
                        yield AgentChunk::error(error);
                        return;
                    }
                    _ => {}
                }
            }

            // Add accumulated response to thread
            if !accumulated_output.is_empty() {
                thread.add_message(ChatMessage::assistant(&accumulated_output));
            }
        })
    }
}

// ============================================================================
// WORKFLOW AS AGENT EXTENSION
// ============================================================================

/// Extension trait to convert workflows to agents.
pub trait WorkflowAsAgent<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    TOutput: Serialize + DeserializeOwned + Clone + Send + Sync + ToString + 'static,
{
    /// Convert this workflow into an agent.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let agent = workflow.as_agent("Content Pipeline");
    /// let response = agent.invoke("Write about AI", &mut thread).await?;
    /// ```
    fn as_agent(self, name: impl Into<String>) -> WorkflowAgent<TMessage, TOutput>;
}

impl<TMessage, TOutput> WorkflowAsAgent<TMessage, TOutput> for Workflow<TMessage, TOutput>
where
    TMessage: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    TOutput: Serialize + DeserializeOwned + Clone + Send + Sync + ToString + 'static,
{
    fn as_agent(self, name: impl Into<String>) -> WorkflowAgent<TMessage, TOutput> {
        WorkflowAgent::new(name, self)
    }
}

impl<TMessage, TOutput> WorkflowAsAgent<TMessage, TOutput> for Arc<Workflow<TMessage, TOutput>>
where
    TMessage: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    TOutput: Serialize + DeserializeOwned + Clone + Send + Sync + ToString + 'static,
{
    fn as_agent(self, name: impl Into<String>) -> WorkflowAgent<TMessage, TOutput> {
        WorkflowAgent::from_arc(name, self)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_agent_id() {
        // We can't easily create a workflow in tests without full setup,
        // so we just test the type structure compiles correctly.
        let _agent_id = AgentId::new("test-workflow");
    }

    // Integration tests would require a full workflow setup
    // See examples/executors_demo.rs for end-to-end tests
}
