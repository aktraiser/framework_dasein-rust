//! ChatAgent - Simple LLM wrapper agent.
//!
//! The `ChatAgent` is the simplest agent type that wraps an LLM adapter
//! and manages conversations. It's the foundation for more complex agents.
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::agent::{ChatAgent, AgentThread, AgentExt};
//! use dasein_agentic_llm::GeminiAdapter;
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

use dasein_agentic_llm::{LLMAdapter, LLMMessage};

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
type ToolHandler = Arc<
    dyn Fn(
            serde_json::Value,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, AgentError>> + Send>,
        > + Send
        + Sync,
>;

pub struct ChatAgent<L: LLMAdapter> {
    handlers: std::collections::HashMap<String, ToolHandler>,
    max_tool_rounds: usize,

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
            handlers: std::collections::HashMap::default(),
            max_tool_rounds: 8,
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

    /// Register a tool and the implementation allowed to execute it.
    pub fn with_tool_handler<F, Fut>(mut self, tool: Tool, handler: F) -> Self
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<serde_json::Value, AgentError>> + Send + 'static,
    {
        self.handlers.insert(
            tool.name.clone(),
            Arc::new(move |args| Box::pin(handler(args))),
        );
        self.tools.retain(|existing| existing.name != tool.name);
        self.tools.push(tool);
        self
    }

    /// Bound model/tool exchanges per invocation.
    pub fn with_max_tool_rounds(mut self, rounds: usize) -> Self {
        self.max_tool_rounds = rounds;
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

        let tools: Vec<dasein_agentic_llm::Tool> = self
            .tools
            .iter()
            .filter(|t| t.enabled)
            .map(|t| {
                Ok(dasein_agentic_llm::Tool {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: serde_json::to_value(&t.parameters)?,
                })
            })
            .collect::<Result<_, AgentError>>()?;
        if !tools.is_empty() && !self.llm.supports_tools() {
            return Err(AgentError::tool(
                "configuration",
                "LLM adapter does not support tools",
            ));
        }
        for tool in &tools {
            if !self.handlers.contains_key(&tool.name) {
                return Err(AgentError::tool(
                    &tool.name,
                    "No execution handler registered",
                ));
            }
        }
        let mut total_tokens: u32 = 0;
        for round in 0..=self.max_tool_rounds {
            let llm_messages = self.build_llm_messages(thread);
            let response = if tools.is_empty() {
                self.llm.generate(&llm_messages).await
            } else {
                self.llm.generate_with_tools(&llm_messages, &tools).await
            }?;
            total_tokens = total_tokens.saturating_add(response.tokens_used.total);
            if response.tool_calls.is_empty() {
                thread.add_response(&response.content, Some(response.tokens_used.total));
                return Ok(
                    AgentResponse::with_messages(response.content, thread.messages.clone())
                        .with_tokens(total_tokens)
                        .with_model(response.model)
                        .with_truncated(
                            response.finish_reason == dasein_agentic_llm::FinishReason::Length,
                        ),
                );
            }
            if round == self.max_tool_rounds {
                return Err(AgentError::tool("limit", "Maximum tool rounds reached"));
            }
            // Validate the complete batch before allowing any side effects.
            for call in &response.tool_calls {
                if !tools.iter().any(|t| t.name == call.name)
                    || !call.arguments.is_object()
                    || call.id.is_empty()
                {
                    return Err(AgentError::tool(
                        &call.name,
                        "Unregistered tool or invalid call",
                    ));
                }
            }
            thread.add_message(
                LLMMessage::assistant_with_tool_calls(
                    response.content,
                    response.tool_calls.clone(),
                )
                .into(),
            );
            for call in response.tool_calls {
                let handler = &self.handlers[&call.name];
                let value = match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    handler(call.arguments),
                )
                .await
                {
                    Ok(Ok(value)) => value,
                    Ok(Err(error)) => serde_json::json!({"error": error.to_string()}),
                    Err(_) => serde_json::json!({"error": "Tool timed out"}),
                };
                thread.add_message(LLMMessage::tool_result(call.id, value.to_string()).into());
            }
        }
        Err(AgentError::internal("Tool loop exhausted"))
    }

    fn run_stream<'a>(
        &'a self,
        messages: Vec<ChatMessage>,
        thread: &'a mut AgentThread,
    ) -> Pin<Box<dyn Stream<Item = AgentChunk> + Send + 'a>> {
        Box::pin(async_stream::stream! {
            if self.tools.iter().any(|t| t.enabled) {
                match self.run(messages, thread).await {
                    Ok(response) => yield AgentChunk { content: Some(response.content), done: true, tokens_used: response.tokens_used, model: response.model, tool_calls: vec![], error: None },
                    Err(error) => yield AgentChunk::error(error.to_string()),
                }
                return;
            }
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
    struct ToolLLM;
    #[async_trait::async_trait]
    impl dasein_agentic_llm::LLMAdapter for ToolLLM {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model(&self) -> &str {
            "mock"
        }
        fn supports_tools(&self) -> bool {
            true
        }
        async fn generate(&self, _: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
            panic!("tools were not forwarded")
        }
        async fn generate_with_tools(
            &self,
            messages: &[LLMMessage],
            tools: &[dasein_agentic_llm::Tool],
        ) -> Result<LLMResponse, LLMError> {
            assert_eq!(tools.len(), 1);
            let returned = messages.last().unwrap().role == dasein_agentic_llm::Role::Tool;
            if returned {
                assert_eq!(
                    messages.last().unwrap().tool_call_id.as_deref(),
                    Some("call1")
                );
                assert_eq!(messages.last().unwrap().content, "42");
                assert_eq!(messages[messages.len() - 2].tool_calls[0].id, "call1");
            }
            Ok(LLMResponse {
                content: if returned {
                    "answer: 42".into()
                } else {
                    String::new()
                },
                tokens_used: TokenUsage {
                    prompt: 1,
                    completion: 1,
                    total: 2,
                },
                finish_reason: FinishReason::Stop,
                model: "mock".into(),
                tool_calls: if returned {
                    vec![]
                } else {
                    vec![dasein_agentic_llm::ToolCall::new(
                        "call1",
                        "lookup",
                        serde_json::json!({}),
                    )]
                },
            })
        }
        fn generate_stream(
            &self,
            _: &[LLMMessage],
        ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
            Box::pin(futures::stream::empty())
        }
        async fn health_check(&self) -> Result<bool, LLMError> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn executes_tool_and_preserves_history() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let agent = ChatAgent::new("tools", ToolLLM).with_tool_handler(
            Tool::new("lookup", "Find answer"),
            move |_| {
                observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { Ok(serde_json::json!(42)) }
            },
        );
        let mut thread = AgentThread::new();
        let response = agent
            .run(vec![ChatMessage::user("find")], &mut thread)
            .await
            .unwrap();
        assert_eq!(response.content, "answer: 42");
        assert_eq!(response.tokens_used, Some(4));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let restored: AgentThread =
            serde_json::from_str(&serde_json::to_string(&thread).unwrap()).unwrap();
        assert_eq!(restored.messages[1].tool_calls[0].id, "call1");
        assert_eq!(restored.messages[2].tool_call_id.as_deref(), Some("call1"));
    }

    #[tokio::test]
    async fn tool_limit_prevents_side_effects() {
        let agent = ChatAgent::new("tools", ToolLLM)
            .with_max_tool_rounds(0)
            .with_tool_handler(Tool::new("lookup", "Find answer"), |_| async {
                panic!("tool ran beyond limit")
            });
        assert!(agent
            .run(vec![ChatMessage::user("find")], &mut AgentThread::new())
            .await
            .is_err());
    }

    use super::*;
    use dasein_agentic_llm::{FinishReason, LLMError, LLMResponse, StreamChunk, TokenUsage};

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
                tool_calls: vec![],
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
