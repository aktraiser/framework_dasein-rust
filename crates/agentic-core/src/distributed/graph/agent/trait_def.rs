//! Agent Trait Definition
//!
//! The core abstraction for agents in the framework. Agents are high-level
//! interfaces that manage conversations, invoke workflows, and use tools.
//!
//! # Agent vs Executor
//!
//! | Aspect | Agent | Executor |
//! |--------|-------|----------|
//! | Level | High (user interface) | Low (workflow node) |
//! | Role | Manages conversation | Atomic work unit |
//! | Lifecycle | `run()`, `run_stream()` | `handle(input, ctx)` |
//! | State | Thread (external) | Context (internal) |
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::agent::{Agent, ChatMessage, AgentThread};
//!
//! async fn chat_with_agent(agent: &impl Agent) {
//!     let mut thread = AgentThread::new();
//!
//!     // Single turn
//!     let response = agent.run(
//!         vec![ChatMessage::user("Hello!")],
//!         &mut thread
//!     ).await?;
//!
//!     // Streaming
//!     let stream = agent.run_stream(
//!         vec![ChatMessage::user("Tell me a story")],
//!         &mut thread
//!     );
//!     while let Some(chunk) = stream.next().await {
//!         print!("{}", chunk.content.unwrap_or_default());
//!     }
//! }
//! ```

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

use super::error::AgentError;
use super::response::{AgentChunk, AgentResponse};
use super::thread::AgentThread;
use super::tools::Tool;
use super::types::{AgentId, ChatMessage};

// ============================================================================
// AGENT TRAIT
// ============================================================================

/// Core trait for agents.
///
/// Agents are stateless - conversation state is carried in `AgentThread`.
/// This allows:
/// - Multiple concurrent conversations with the same agent
/// - Thread handoff between agents
/// - Thread persistence and resumption
#[async_trait]
pub trait Agent: Send + Sync {
    /// Get the agent's unique identifier.
    fn id(&self) -> &AgentId;

    /// Get the agent's human-readable name.
    fn name(&self) -> &str;

    /// Get the agent's description (optional).
    fn description(&self) -> Option<&str> {
        None
    }

    /// Run a conversation turn.
    ///
    /// # Arguments
    ///
    /// * `messages` - New messages to add to the conversation
    /// * `thread` - Mutable reference to the conversation thread
    ///
    /// # Returns
    ///
    /// The agent's response, which also gets added to the thread.
    async fn run(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError>;

    /// Run a conversation turn with streaming response.
    ///
    /// # Arguments
    ///
    /// * `messages` - New messages to add to the conversation
    /// * `thread` - Mutable reference to the conversation thread
    ///
    /// # Returns
    ///
    /// A stream of response chunks.
    fn run_stream<'a>(
        &'a self,
        messages: Vec<ChatMessage>,
        thread: &'a mut AgentThread,
    ) -> Pin<Box<dyn Stream<Item = AgentChunk> + Send + 'a>>;

    /// Get the tools available to this agent.
    ///
    /// Default: no tools.
    fn tools(&self) -> &[Tool] {
        &[]
    }

    /// Check if agent supports streaming.
    ///
    /// Default: true (most agents support streaming).
    fn supports_streaming(&self) -> bool {
        true
    }

    /// Get the system prompt (if any).
    ///
    /// This is used when the thread doesn't have its own system prompt.
    fn system_prompt(&self) -> Option<&str> {
        None
    }
}

// ============================================================================
// CONVENIENCE METHODS (Extension trait)
// ============================================================================

/// Extension trait with convenience methods for agents.
#[async_trait]
pub trait AgentExt: Agent {
    /// Run with a single user message (convenience method).
    async fn invoke(
        &self,
        message: impl Into<String> + Send,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError> {
        self.run(vec![ChatMessage::user(message)], thread).await
    }

    /// Create a new thread for this agent.
    fn new_thread(&self) -> AgentThread {
        let mut thread = AgentThread::for_agent(self.id().clone());
        if let Some(prompt) = self.system_prompt() {
            thread = thread.with_system_prompt(prompt);
        }
        thread
    }

    /// Run with a new thread (one-shot conversation).
    async fn run_once(
        &self,
        message: impl Into<String> + Send,
    ) -> Result<AgentResponse, AgentError> {
        let mut thread = self.new_thread();
        self.invoke(message, &mut thread).await
    }
}

// Blanket implementation for all Agent types
impl<T: Agent + ?Sized> AgentExt for T {}

// ============================================================================
// BOXED AGENT (Type-erased)
// ============================================================================

/// Type alias for a boxed agent (useful for collections).
pub type BoxedAgent = Box<dyn Agent>;

/// Type alias for an Arc-wrapped agent (for sharing).
pub type SharedAgent = std::sync::Arc<dyn Agent>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Mock agent for testing
    struct MockAgent {
        id: AgentId,
        name: String,
    }

    impl MockAgent {
        fn new(name: &str) -> Self {
            Self {
                id: AgentId::new(name),
                name: name.to_string(),
            }
        }
    }

    #[async_trait]
    impl Agent for MockAgent {
        fn id(&self) -> &AgentId {
            &self.id
        }

        fn name(&self) -> &str {
            &self.name
        }

        async fn run(
            &self,
            messages: Vec<ChatMessage>,
            thread: &mut AgentThread,
        ) -> Result<AgentResponse, AgentError> {
            // Add input messages to thread
            thread.add_messages(messages);

            // Generate response
            let response_content = format!("Mock response from {}", self.name);
            thread.add_response(&response_content, Some(10));

            Ok(AgentResponse::new(response_content).with_tokens(10))
        }

        fn run_stream<'a>(
            &'a self,
            messages: Vec<ChatMessage>,
            thread: &'a mut AgentThread,
        ) -> Pin<Box<dyn Stream<Item = AgentChunk> + Send + 'a>> {
            Box::pin(futures::stream::once(async move {
                // Add messages to thread
                for msg in messages {
                    thread.add_message(msg);
                }
                AgentChunk::final_content(format!("Streamed from {}", self.name))
            }))
        }
    }

    #[tokio::test]
    async fn test_agent_trait() {
        let agent = MockAgent::new("test-agent");
        assert_eq!(agent.id().as_str(), "test-agent");
        assert_eq!(agent.name(), "test-agent");
    }

    #[tokio::test]
    async fn test_agent_run() {
        let agent = MockAgent::new("test");
        let mut thread = AgentThread::new();

        let response = agent
            .run(vec![ChatMessage::user("Hello")], &mut thread)
            .await
            .unwrap();

        assert!(response.content.contains("Mock response"));
        assert_eq!(thread.message_count(), 2); // user + assistant
    }

    #[tokio::test]
    async fn test_agent_ext_invoke() {
        let agent = MockAgent::new("test");
        let mut thread = AgentThread::new();

        let response = agent.invoke("Hello", &mut thread).await.unwrap();
        assert!(response.content.contains("Mock response"));
    }

    #[tokio::test]
    async fn test_agent_ext_new_thread() {
        let agent = MockAgent::new("writer");
        let thread = agent.new_thread();

        assert_eq!(thread.created_by, Some(AgentId::new("writer")));
    }

    #[tokio::test]
    async fn test_agent_ext_run_once() {
        let agent = MockAgent::new("test");
        let response = agent.run_once("Hello").await.unwrap();
        assert!(response.content.contains("Mock response"));
    }
}
