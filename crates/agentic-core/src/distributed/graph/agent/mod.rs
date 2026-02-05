//! Agent Layer - High-level interface for conversational AI.
//!
//! This module provides the `Agent` trait and implementations for building
//! conversational agents that can use LLMs, invoke workflows, and manage tools.
//!
//! # Core Concepts
//!
//! | Concept | Description |
//! |---------|-------------|
//! | `Agent` | Trait for conversational agents with `run()` and `run_stream()` |
//! | `AgentThread` | Conversation state container (agents are stateless) |
//! | `ChatAgent` | Simple LLM wrapper agent |
//! | `WorkflowAgent` | Agent that delegates to a workflow |
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
//! # Quick Start
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::agent::{
//!     Agent, AgentExt, ChatAgent, AgentThread, ChatMessage,
//! };
//! use agentic_llm::GeminiAdapter;
//!
//! // Create an agent
//! let llm = GeminiAdapter::new("api-key", "gemini-2.0-flash");
//! let agent = ChatAgent::new("assistant", llm)
//!     .with_system_prompt("You are helpful.");
//!
//! // Create a thread
//! let mut thread = agent.new_thread();
//!
//! // Run a conversation turn
//! let response = agent.invoke("Hello!", &mut thread).await?;
//! println!("{}", response.content);
//!
//! // Continue the conversation
//! let response = agent.invoke("What's 2+2?", &mut thread).await?;
//! println!("{}", response.content);
//! ```
//!
//! # Streaming
//!
//! ```rust,ignore
//! use futures::StreamExt;
//!
//! let mut stream = agent.run_stream(
//!     vec![ChatMessage::user("Tell me a story")],
//!     &mut thread
//! );
//!
//! while let Some(chunk) = stream.next().await {
//!     if let Some(text) = chunk.content {
//!         print!("{}", text);
//!     }
//! }
//! ```
//!
//! # Workflow as Agent
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::agent::WorkflowAsAgent;
//!
//! // Convert workflow to agent
//! let agent = workflow.as_agent("Code Generator");
//!
//! // Use like any agent
//! let response = agent.invoke("Write fibonacci", &mut thread).await?;
//! ```

mod background;
mod chat_agent;
mod error;
mod memory;
mod persistence;
mod reducer;
mod response;
mod thread;
mod tools;
mod trait_def;
mod types;
mod workflow_agent;

// === Types ===
pub use types::{AgentId, ChatMessage, ChatRole, ThreadId};

// === Thread ===
pub use thread::{AgentThread, ThreadSummary};

// === Response ===
pub use response::{AgentChunk, AgentResponse, ToolCall};

// === Error ===
pub use error::{AgentError, AgentResult};

// === Tools ===
pub use tools::{Tool, ToolParam, ToolParameters};

// === Trait ===
pub use trait_def::{Agent, AgentExt, BoxedAgent, SharedAgent};

// === Implementations ===
pub use chat_agent::ChatAgent;
pub use workflow_agent::{WorkflowAgent, WorkflowAsAgent};

// === Memory ===
pub use memory::{
    BoxedMemoryProvider, InMemoryProvider, Memory, MemoryCategory, MemoryContext, MemoryError,
    MemoryProvider, MemoryResult, NoOpMemoryProvider, SharedMemoryProvider, UserMemories,
};

// === Reducers ===
pub use reducer::{
    BoxedChatReducer, ChatReducer, MessageCountingReducer, NoOpReducer, SharedChatReducer,
    SlidingWindowReducer, TokenCountingReducer,
};

// === Persistence ===
pub use persistence::{
    BoxedThreadStore, InMemoryThreadStore, NatsThreadStore, SharedThreadStore, ThreadStore,
    ThreadStoreError, ThreadStoreResult,
};

// === Background ===
pub use background::{
    BackgroundResponse, BackgroundTask, BackgroundTaskId, ContinuationError, ContinuationToken,
    InMemoryTaskStore, SharedTaskStore, TaskStatus,
};

// === NATS Memory Provider ===
pub use memory::NatsMemoryProvider;
