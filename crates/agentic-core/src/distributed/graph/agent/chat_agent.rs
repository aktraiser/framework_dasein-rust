//! ChatAgent - Simple LLM wrapper agent.
//!
//! The `ChatAgent` is the simplest agent type that wraps an LLM adapter
//! and manages conversations. It's the foundation for more complex agents.
//!
//! # Example
//!
//! ```rust,ignore
//! use agentic_core::distributed::graph::agent::{ChatAgent, AgentThread, AgentExt};
//! use agentic_llm::GeminiAdapter;
//!
//! let llm = GeminiAdapter::new("api-key", "gemini-2.0-flash");
//! let agent = ChatAgent::new("assistant", llm)
//!     .with_system_prompt("You are a helpful assistant.");
//!
//! let mut thread = agent.new_thread();
//! let response = agent.invoke("Hello!", &mut thread).await?;
//! println!("{}", response.content);
//! ```

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;

use agentic_llm::{LLMAdapter, LLMMessage};

use super::error::AgentError;
use super::response::{AgentChunk, AgentResponse};
use super::thread::AgentThread;
use super::tools::Tool;
use super::trait_def::Agent;
use super::types::{AgentId, ChatMessage};

// ============================================================================
// CHAT AGENT
// ============================================================================

/// A simple agent that wraps an LLM adapter.
///
/// ChatAgent is stateless - all conversation state is in `AgentThread`.
pub struct ChatAgent<L: LLMAdapter> {
    /// Agent identifier.
    id: AgentId,

    /// Human-readable name.
    name: String,

    /// Optional description.
    description: Option<String>,

    /// The underlying LLM adapter.
    llm: Arc<L>,

    /// System prompt for this agent.
    system_prompt: Option<String>,

    /// Available tools.
    tools: Vec<Tool>,
}

impl<L: LLMAdapter> ChatAgent<L> {
    /// Create a new ChatAgent with an LLM adapter.
    pub fn new(name: impl Into<String>, llm: L) -> Self {
        let name = name.into();
        Self {
            id: AgentId::new(&name),
            name,
            description: None,
            llm: Arc::new(llm),
            system_prompt: None,
            tools: vec![],
        }
    }

    /// Create with a specific ID.
    pub fn with_id(mut self, id: impl Into<AgentId>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add a tool.
    pub fn with_tool(mut self, tool: Tool) -> Self {
        self.tools.push(tool);
        self
    }

    /// Add multiple tools.
    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools.extend(tools);
        self
    }

    /// Get the LLM adapter.
    pub fn llm(&self) -> &L {
        &self.llm
    }

    /// Build LLM messages from thread.
    fn build_llm_messages(&self, thread: &AgentThread) -> Vec<LLMMessage> {
        let mut messages = Vec::new();

        // System prompt: agent's prompt takes precedence, then thread's
        let system_prompt = self
            .system_prompt
            .as_ref()
            .or(thread.system_prompt.as_ref());

        if let Some(prompt) = system_prompt {
            messages.push(LLMMessage::system(prompt));
        }

        // Add conversation messages
        for msg in &thread.messages {
            if !msg.is_system() {
                messages.push(msg.into());
            }
        }

        messages
    }
}

#[async_trait]
impl<L: LLMAdapter + 'static> Agent for ChatAgent<L> {
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
        &self.tools
    }

    async fn run(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
    ) -> Result<AgentResponse, AgentError> {
        // Add new messages to thread
        thread.add_messages(messages);

        // Build LLM messages
        let llm_messages = self.build_llm_messages(thread);

        // Call LLM
        let response = self
            .llm
            .generate(&llm_messages)
            .await
            .map_err(AgentError::from)?;

        // Add assistant response to thread
        thread.add_response(&response.content, Some(response.tokens_used.total));

        // Build response
        Ok(AgentResponse::with_messages(response.content, thread.messages.clone())
            .with_tokens(response.tokens_used.total)
            .with_model(response.model)
            .with_truncated(response.finish_reason == agentic_llm::FinishReason::Length))
    }

    fn run_stream<'a>(
        &'a self,
        messages: Vec<ChatMessage>,
        thread: &'a mut AgentThread,
    ) -> Pin<Box<dyn Stream<Item = AgentChunk> + Send + 'a>> {
        Box::pin(async_stream::stream! {
            // Add new messages to thread
            for msg in messages {
                thread.add_message(msg);
            }

            // Build LLM messages
            let llm_messages = self.build_llm_messages(thread);

            // Get stream from LLM
            let mut stream = self.llm.generate_stream(&llm_messages);

            // Accumulate content for thread
            let mut full_content = String::new();
            let mut final_tokens: Option<u32> = None;
            let final_model: Option<String> = None;

            use futures::StreamExt;
            while let Some(result) = stream.next().await {
                match result {
                    Ok(chunk) => {
                        // Accumulate content
                        full_content.push_str(&chunk.content);

                        // Get tokens before consuming
                        let tokens = chunk.tokens_used.map(|t| t.total);

                        // Track final chunk info
                        if chunk.done {
                            final_tokens = tokens;
                        }

                        // Yield chunk
                        yield AgentChunk {
                            content: Some(chunk.content),
                            done: chunk.done,
                            tokens_used: tokens,
                            model: final_model.clone(),
                            tool_calls: vec![],
                            error: None,
                        };
                    }
                    Err(e) => {
                        yield AgentChunk::error(e.to_string());
                        return;
                    }
                }
            }

            // Add accumulated response to thread
            if !full_content.is_empty() {
                thread.add_response(&full_content, final_tokens);
            }
        })
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_llm::{FinishReason, LLMError, LLMResponse, StreamChunk, TokenUsage};

    // Mock LLM for testing
    struct MockLLM {
        response: String,
    }

    impl MockLLM {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl LLMAdapter for MockLLM {
        fn provider(&self) -> &str {
            "mock"
        }

        fn model(&self) -> &str {
            "mock-model"
        }

        async fn generate(&self, _messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
            Ok(LLMResponse {
                content: self.response.clone(),
                tokens_used: TokenUsage {
                    prompt: 10,
                    completion: 20,
                    total: 30,
                },
                finish_reason: FinishReason::Stop,
                model: "mock-model".into(),
            })
        }

        fn generate_stream(
            &self,
            _messages: &[LLMMessage],
        ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
            let content = self.response.clone();
            Box::pin(futures::stream::once(async move {
                Ok(StreamChunk {
                    content,
                    done: true,
                    tokens_used: Some(TokenUsage {
                        prompt: 10,
                        completion: 20,
                        total: 30,
                    }),
                    finish_reason: Some(FinishReason::Stop),
                })
            }))
        }

        async fn health_check(&self) -> Result<bool, LLMError> {
            Ok(true)
        }
    }

    #[test]
    fn test_chat_agent_new() {
        let llm = MockLLM::new("Hello!");
        let agent = ChatAgent::new("test-agent", llm);

        assert_eq!(agent.id().as_str(), "test-agent");
        assert_eq!(agent.name(), "test-agent");
    }

    #[test]
    fn test_chat_agent_with_system_prompt() {
        let llm = MockLLM::new("Hi");
        let agent = ChatAgent::new("test", llm).with_system_prompt("Be helpful.");

        assert_eq!(agent.system_prompt(), Some("Be helpful."));
    }

    #[tokio::test]
    async fn test_chat_agent_run() {
        let llm = MockLLM::new("Hello, human!");
        let agent = ChatAgent::new("greeter", llm);

        let mut thread = AgentThread::new();
        let response = agent
            .run(vec![ChatMessage::user("Hi")], &mut thread)
            .await
            .unwrap();

        assert_eq!(response.content, "Hello, human!");
        assert_eq!(response.tokens_used, Some(30));
        assert_eq!(thread.message_count(), 2); // user + assistant
    }

    #[tokio::test]
    async fn test_chat_agent_run_stream() {
        use futures::StreamExt;

        let llm = MockLLM::new("Streamed response");
        let agent = ChatAgent::new("streamer", llm);

        let mut thread = AgentThread::new();
        let mut stream = agent.run_stream(vec![ChatMessage::user("Stream me")], &mut thread);

        let mut chunks = vec![];
        while let Some(chunk) = stream.next().await {
            chunks.push(chunk);
        }

        assert!(!chunks.is_empty());
        assert!(chunks.last().unwrap().done);
    }

    #[test]
    fn test_chat_agent_with_tools() {
        let llm = MockLLM::new("Using tool...");
        let tool = Tool::new("search", "Search the web");
        let agent = ChatAgent::new("searcher", llm).with_tool(tool);

        assert_eq!(agent.tools().len(), 1);
        assert_eq!(agent.tools()[0].name, "search");
    }
}
