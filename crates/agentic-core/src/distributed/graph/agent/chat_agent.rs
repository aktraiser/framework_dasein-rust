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

#[path = "execution.rs"]
mod execution;
pub use execution::{AgentExecutionCheckpoint, AgentExecutionStore};

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
    execution_store: Option<Arc<dyn AgentExecutionStore>>,
    handlers: std::collections::HashMap<String, ToolHandler>,
    max_tool_rounds: usize,
    completion_check: Option<Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>>,
    tool_timeout: std::time::Duration,
    terminal_tools: std::collections::HashSet<String>,
    tool_filter: Option<Arc<dyn Fn(&str, usize) -> bool + Send + Sync>>,

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
            execution_store: None,
            handlers: std::collections::HashMap::default(),
            max_tool_rounds: 8,
            completion_check: None,
            tool_timeout: std::time::Duration::from_secs(30),
            terminal_tools: std::collections::HashSet::default(),
            tool_filter: None,
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

    /// Require an application-defined completion contract. Rejections are sent
    /// back to the model and consume the same bounded round budget as tools.
    /// No validator preserves the existing conversational behavior.
    pub fn with_completion_check<F>(mut self, check: F) -> Self
    where
        F: Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    {
        self.completion_check = Some(Arc::new(check));
        self
    }

    /// Bound each tool handler including its internal resource queue wait.
    pub fn with_tool_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.tool_timeout = timeout;
        self
    }

    /// Stop immediately after a successful registered terminal tool. Remaining
    /// calls in the batch receive cancellation results without executing.
    pub fn with_terminal_tool(mut self, name: impl Into<String>) -> Self {
        self.terminal_tools.insert(name.into());
        self
    }

    /// Compute allowed tools at each round, e.g. remove exhausted actions and
    /// reserve final rounds for conclusion. The default exposes all tools.
    pub fn with_tool_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&str, usize) -> bool + Send + Sync + 'static,
    {
        self.tool_filter = Some(Arc::new(filter));
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
        if let Some(store) = &self.execution_store {
            return self.run_durable(messages, thread, store.as_ref()).await;
        }
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
            let tools: Vec<_> = tools
                .iter()
                .filter(|t| {
                    self.tool_filter
                        .as_ref()
                        .map(|f| f(&t.name, round))
                        .unwrap_or(true)
                })
                .cloned()
                .collect();
            let llm_messages = self.build_llm_messages(thread);
            let response = if tools.is_empty() {
                self.llm.generate(&llm_messages).await
            } else {
                self.llm.generate_with_tools(&llm_messages, &tools).await
            }?;
            total_tokens = total_tokens.saturating_add(response.tokens_used.total);
            if response.tool_calls.is_empty() {
                if let Some(check) = &self.completion_check {
                    if let Err(reason) = check(&response.content) {
                        if round == self.max_tool_rounds {
                            return Err(AgentError::tool("completion", reason));
                        }
                        thread.add_response(&response.content, Some(response.tokens_used.total));
                        thread.add_message(
                            LLMMessage::user(format!("Completion rejected: {reason}")).into(),
                        );
                        continue;
                    }
                }
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
            if response.finish_reason == dasein_agentic_llm::FinishReason::Length {
                thread.add_response(
                    "Incomplete tool batch discarded: output limit reached.",
                    Some(response.tokens_used.total),
                );
                thread.add_message(LLMMessage::user("Your tool batch was truncated. Call one tool at a time, with shorter arguments.").into());
                continue;
            }
            if round == self.max_tool_rounds
                && !response
                    .tool_calls
                    .iter()
                    .all(|c| self.terminal_tools.contains(&c.name))
            {
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
            let mut terminal_result: Option<String> = None;
            for call in response.tool_calls {
                if terminal_result.is_some() {
                    thread.add_message(
                        LLMMessage::tool_result(
                            call.id,
                            "{\"skipped\":\"Invocation already finalized\"}",
                        )
                        .into(),
                    );
                    continue;
                }
                let handler = &self.handlers[&call.name];
                let value =
                    match tokio::time::timeout(self.tool_timeout, handler(call.arguments)).await {
                        Ok(Ok(value)) => value,
                        Ok(Err(error)) => serde_json::json!({"error": error.to_string()}),
                        Err(_) => serde_json::json!({"error": "Tool timed out"}),
                    };
                if self.terminal_tools.contains(&call.name) && value.get("error").is_none() {
                    let content = value
                        .get("conclusion")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    if self
                        .completion_check
                        .as_ref()
                        .map(|f| f(&content).is_ok())
                        .unwrap_or(true)
                    {
                        terminal_result = Some(content);
                    }
                }
                thread.add_message(LLMMessage::tool_result(call.id, value.to_string()).into());
            }
            if let Some(content) = terminal_result {
                thread.add_response(&content, None);
                return Ok(
                    AgentResponse::with_messages(content, thread.messages.clone())
                        .with_tokens(total_tokens)
                        .with_model(response.model),
                );
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
            if self.execution_store.is_some() || self.tools.iter().any(|t| t.enabled) || self.completion_check.is_some() {
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
                    details: None,
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
                    details: None,
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
                        details: None,
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

    #[tokio::test]
    async fn completion_contract_retries_with_a_bounded_budget() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let agent = ChatAgent::new("checked", MockLLM::new("final"))
            .with_max_tool_rounds(2)
            .with_completion_check(move |_| {
                if seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    Err("Call the finalization tool first".into())
                } else {
                    Ok(())
                }
            });
        let mut thread = AgentThread::new();
        let answer = agent
            .run(vec![ChatMessage::user("work")], &mut thread)
            .await
            .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(answer.tokens_used, Some(60));
        assert!(thread
            .messages
            .iter()
            .any(|m| m.content.contains("Completion rejected")));
        let refusing = ChatAgent::new("refusing", MockLLM::new("I will continue later"))
            .with_max_tool_rounds(1)
            .with_completion_check(|_| Err("Missing conclusion".into()));
        assert!(refusing
            .run(vec![ChatMessage::user("work")], &mut AgentThread::new())
            .await
            .is_err());
        use futures::StreamExt;
        let mut thread = AgentThread::new();
        let chunks: Vec<_> = refusing
            .run_stream(vec![ChatMessage::user("work")], &mut thread)
            .collect()
            .await;
        assert!(chunks.last().unwrap().error.is_some());
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
    struct ScriptLLM {
        responses: std::sync::Mutex<std::collections::VecDeque<LLMResponse>>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        offers: Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    }
    #[async_trait::async_trait]
    impl LLMAdapter for ScriptLLM {
        fn provider(&self) -> &str {
            "script"
        }
        fn model(&self) -> &str {
            "script"
        }
        fn supports_tools(&self) -> bool {
            true
        }
        async fn generate(&self, m: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
            self.generate_with_tools(m, &[]).await
        }
        async fn generate_with_tools(
            &self,
            _: &[LLMMessage],
            t: &[dasein_agentic_llm::Tool],
        ) -> Result<LLMResponse, LLMError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.offers
                .lock()
                .unwrap()
                .push(t.iter().map(|t| t.name.clone()).collect());
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model call after completion"))
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
    fn batch(names: &[&str], truncated: bool) -> LLMResponse {
        LLMResponse {
            content: String::new(),
            model: "script".into(),
            tokens_used: TokenUsage {
                details: None,
                prompt: 1,
                completion: 1,
                total: 2,
            },
            finish_reason: if truncated {
                FinishReason::Length
            } else {
                FinishReason::ToolCalls
            },
            tool_calls: names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    dasein_agentic_llm::ToolCall::new(
                        format!("call-{i}"),
                        *n,
                        serde_json::json!({}),
                    )
                })
                .collect(),
        }
    }
    fn script(responses: Vec<LLMResponse>) -> ScriptLLM {
        ScriptLLM {
            responses: std::sync::Mutex::new(responses.into()),
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            offers: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    #[tokio::test]
    async fn terminal_tool_stops_without_another_model_call_or_tail_side_effect() {
        let llm = script(vec![batch(&["finish", "mutate"], false)]);
        let calls = llm.calls.clone();
        let agent = ChatAgent::new("terminal", llm)
            .with_tool_handler(Tool::new("finish", "finish"), |_| async {
                Ok(serde_json::json!({"conclusion":"Done"}))
            })
            .with_tool_handler(Tool::new("mutate", "must never run"), |_| async {
                panic!("tail effect after finish")
            })
            .with_terminal_tool("finish")
            .with_completion_check(|s| {
                if s == "Done" {
                    Ok(())
                } else {
                    Err("invalid".into())
                }
            });
        let mut thread = AgentThread::new();
        let result = agent
            .run(vec![ChatMessage::user("work")], &mut thread)
            .await
            .unwrap();
        assert_eq!(result.content, "Done");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(thread
            .messages
            .iter()
            .any(|m| m.content.contains("Invocation already finalized")));
    }
    #[tokio::test]
    async fn failed_terminal_retries_and_only_terminal_is_offered_in_last_round() {
        let llm = script(vec![batch(&["finish"], false), batch(&["finish"], false)]);
        let offers = llm.offers.clone();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = attempts.clone();
        let agent = ChatAgent::new("retry", llm)
            .with_max_tool_rounds(1)
            .with_tool_handler(Tool::new("finish", "finish"), move |_| {
                let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    Ok(if n == 0 {
                        serde_json::json!({"error":"Missing evidence"})
                    } else {
                        serde_json::json!({"conclusion":"Done"})
                    })
                }
            })
            .with_tool_handler(Tool::new("search", "search"), |_| async {
                panic!("unused")
            })
            .with_terminal_tool("finish")
            .with_tool_filter(|name, round| round == 0 || name == "finish");
        assert_eq!(
            agent
                .run(vec![ChatMessage::user("work")], &mut AgentThread::new())
                .await
                .unwrap()
                .content,
            "Done"
        );
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(offers.lock().unwrap()[1], vec!["finish"]);
    }
    #[tokio::test]
    async fn truncated_tool_batch_has_no_side_effects() {
        let llm = script(vec![batch(&["mutate"], true), batch(&["finish"], false)]);
        let agent = ChatAgent::new("truncated", llm)
            .with_tool_handler(Tool::new("mutate", "never run"), |_| async {
                panic!("truncated action executed")
            })
            .with_tool_handler(Tool::new("finish", "finish"), |_| async {
                Ok(serde_json::json!({"conclusion":"Recovered"}))
            })
            .with_terminal_tool("finish");
        assert_eq!(
            agent
                .run(vec![ChatMessage::user("work")], &mut AgentThread::new())
                .await
                .unwrap()
                .content,
            "Recovered"
        );
    }
}
