//! Agent Demo - Phase 6 Agent Layer Verification
//!
//! Demonstrates the Agent Layer:
//! - `Agent` trait with `run()` and `run_stream()`
//! - `ChatAgent` wrapping an LLM adapter
//! - `AgentThread` for conversation state
//! - `AgentExt` convenience methods (`invoke()`, `run_once()`, `new_thread()`)
//! - Tool definitions for function calling
//! - Streaming with `AgentChunk`
//!
//! Environment variables:
//! - GEMINI_API_KEY: For real LLM tests (optional, uses mock if not set)
//!
//! Run with: cargo run --example agent_demo

use dasein_agentic_core::distributed::graph::agent::{
    Agent, AgentChunk, AgentError, AgentExt, AgentResponse, AgentThread, ChatAgent, ChatMessage,
    ChatRole, ThreadSummary, Tool, ToolParam,
};
use dasein_agentic_llm::{
    FinishReason, LLMAdapter, LLMError, LLMMessage, LLMResponse, StreamChunk, TokenUsage,
};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use std::time::Instant;

// ============================================================================
// MOCK LLM ADAPTER (for testing without API keys)
// ============================================================================

struct MockLLM {
    name: String,
    responses: Vec<String>,
    call_count: std::sync::atomic::AtomicUsize,
}

impl MockLLM {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            responses: vec![
                "Hello! I'm a helpful assistant. How can I help you today?".into(),
                "That's a great question! Let me think about it...".into(),
                "Based on my analysis, I would recommend the following approach.".into(),
                "Is there anything else you'd like to know?".into(),
            ],
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn with_responses(mut self, responses: Vec<&str>) -> Self {
        self.responses = responses.iter().map(|s| s.to_string()).collect();
        self
    }

    fn get_response(&self) -> String {
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.responses[count % self.responses.len()].clone()
    }
}

#[async_trait]
impl LLMAdapter for MockLLM {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model(&self) -> &str {
        &self.name
    }

    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        // Simulate some latency
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let content = self.get_response();
        let input_tokens = messages.iter().map(|m| m.content.len() / 4).sum::<usize>() as u32;
        let output_tokens = (content.len() / 4) as u32;

        Ok(LLMResponse {
            content,
            tokens_used: TokenUsage {
                prompt: input_tokens,
                completion: output_tokens,
                total: input_tokens + output_tokens,
            },
            finish_reason: FinishReason::Stop,
            model: self.name.clone(),
        })
    }

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        let content = self.get_response();
        let input_tokens = messages.iter().map(|m| m.content.len() / 4).sum::<usize>() as u32;

        // Split content into chunks for streaming
        let chunks: Vec<String> = content
            .chars()
            .collect::<Vec<_>>()
            .chunks(10)
            .map(|c| c.iter().collect())
            .collect();

        let total_chunks = chunks.len();
        let output_tokens = (content.len() / 4) as u32;

        Box::pin(futures::stream::iter(
            chunks.into_iter().enumerate().map(move |(i, chunk)| {
                let is_last = i == total_chunks - 1;
                Ok(StreamChunk {
                    content: chunk,
                    done: is_last,
                    tokens_used: if is_last {
                        Some(TokenUsage {
                            prompt: input_tokens,
                            completion: output_tokens,
                            total: input_tokens + output_tokens,
                        })
                    } else {
                        None
                    },
                    finish_reason: if is_last {
                        Some(FinishReason::Stop)
                    } else {
                        None
                    },
                })
            }),
        ))
    }

    async fn health_check(&self) -> Result<bool, LLMError> {
        Ok(true)
    }
}

// ============================================================================
// TEST 1: ChatMessage and ChatRole
// ============================================================================

fn test_chat_messages() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 1: ChatMessage and ChatRole");
    println!("{}", "=".repeat(70));

    // Create messages with different roles
    let system_msg = ChatMessage::system("You are a helpful coding assistant.");
    let user_msg = ChatMessage::user("Write a function to calculate factorial");
    let assistant_msg = ChatMessage::assistant("Here's a factorial function in Rust...");
    let tool_msg = ChatMessage::tool("Result: 120").with_name("calculator");

    println!("\n  ✓ ChatMessage::system()");
    println!("    role: {:?}", system_msg.role);
    println!("    content: {} chars", system_msg.content.len());
    assert!(system_msg.is_system());

    println!("\n  ✓ ChatMessage::user()");
    println!("    role: {:?}", user_msg.role);
    println!("    is_user: {}", user_msg.is_user());
    assert!(user_msg.is_user());

    println!("\n  ✓ ChatMessage::assistant()");
    println!("    role: {:?}", assistant_msg.role);
    println!("    is_assistant: {}", assistant_msg.is_assistant());
    assert!(assistant_msg.is_assistant());

    println!("\n  ✓ ChatMessage::tool().with_name()");
    println!("    role: {:?}", tool_msg.role);
    println!("    name: {:?}", tool_msg.name);
    assert_eq!(tool_msg.role, ChatRole::Tool);

    // Test Display
    println!("\n  ✓ ChatMessage Display");
    println!("    {}", user_msg);

    // Test serialization
    let json = serde_json::to_string(&user_msg).unwrap();
    let restored: ChatMessage = serde_json::from_str(&json).unwrap();
    println!("\n  ✓ Serialization round-trip");
    println!("    original: {}", user_msg.content);
    println!("    restored: {}", restored.content);
    assert_eq!(user_msg.content, restored.content);

    println!("\n  ✅ TEST 1 PASSED");
}

// ============================================================================
// TEST 2: AgentThread Management
// ============================================================================

fn test_agent_thread() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 2: AgentThread Management");
    println!("{}", "=".repeat(70));

    // Create thread
    let mut thread = AgentThread::new();
    println!("\n  ✓ AgentThread::new()");
    println!("    id: {}", thread.id.as_str());
    println!("    is_empty: {}", thread.is_empty());
    assert!(thread.is_empty());

    // Add messages
    thread.add_message(ChatMessage::user("Hello!"));
    thread.add_message(ChatMessage::assistant("Hi there!"));
    thread.add_message(ChatMessage::user("How are you?"));
    thread.add_message(ChatMessage::assistant("I'm doing great!"));

    println!("\n  ✓ add_message() x4");
    println!("    message_count: {}", thread.message_count());
    println!("    turn_count: {}", thread.turn_count);
    assert_eq!(thread.message_count(), 4);
    assert_eq!(thread.turn_count, 2); // 2 user messages = 2 turns

    // Last messages
    let last_two = thread.last_messages(2);
    println!("\n  ✓ last_messages(2)");
    println!("    [0]: {}", last_two[0].content);
    println!("    [1]: {}", last_two[1].content);
    assert_eq!(last_two.len(), 2);

    // Last user/assistant
    println!("\n  ✓ last_user_message()");
    println!(
        "    content: {:?}",
        thread.last_user_message().map(|m| &m.content)
    );

    println!("\n  ✓ last_assistant_message()");
    println!(
        "    content: {:?}",
        thread.last_assistant_message().map(|m| &m.content)
    );

    // Metadata
    thread.set_metadata("user_id", serde_json::json!("user-123"));
    thread.set_metadata("session", serde_json::json!({"started": "2024-01-15"}));

    println!("\n  ✓ set_metadata()");
    println!("    user_id: {:?}", thread.get_metadata("user_id"));
    println!("    session: {:?}", thread.get_metadata("session"));

    // System prompt
    let thread_with_prompt = AgentThread::new().with_system_prompt("You are a coding expert.");
    println!("\n  ✓ with_system_prompt()");
    println!("    system_prompt: {:?}", thread_with_prompt.system_prompt);

    // Serialization
    let json = thread.to_json().unwrap();
    let restored = AgentThread::from_json(&json).unwrap();
    println!("\n  ✓ to_json() / from_json()");
    println!("    original messages: {}", thread.message_count());
    println!("    restored messages: {}", restored.message_count());
    assert_eq!(thread.message_count(), restored.message_count());

    // Thread summary
    let summary = ThreadSummary::from(&thread);
    println!("\n  ✓ ThreadSummary::from()");
    println!("    id: {}", summary.id.as_str());
    println!("    message_count: {}", summary.message_count);
    println!("    turn_count: {}", summary.turn_count);
    println!("    preview: {:?}", summary.preview);

    println!("\n  ✅ TEST 2 PASSED");
}

// ============================================================================
// TEST 3: Tool Definitions
// ============================================================================

fn test_tools() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 3: Tool Definitions");
    println!("{}", "=".repeat(70));

    // Simple tool
    let simple_tool = Tool::new("get_weather", "Get current weather for a location");
    println!("\n  ✓ Tool::new()");
    println!("    name: {}", simple_tool.name);
    println!("    description: {}", simple_tool.description);
    println!("    enabled: {}", simple_tool.is_enabled());

    // Tool with parameters
    let calculator = Tool::new("calculator", "Perform mathematical calculations")
        .with_param(
            "expression",
            ToolParam::string("Mathematical expression to evaluate"),
        )
        .with_optional_param("precision", ToolParam::integer("Decimal precision (default: 2)"));

    println!("\n  ✓ Tool with parameters");
    println!("    name: {}", calculator.name);
    println!(
        "    properties: {} params",
        calculator.parameters.properties.len()
    );
    println!("    required: {:?}", calculator.parameters.required);

    // Tool with enum param
    let search = Tool::new("search", "Search for information")
        .with_param("query", ToolParam::string("Search query"))
        .with_param(
            "type",
            ToolParam::string_enum("Search type", vec!["web", "images", "news"]),
        );

    println!("\n  ✓ Tool with enum param");
    println!("    name: {}", search.name);
    let type_param = search.parameters.properties.get("type").unwrap();
    println!("    type.enum_values: {:?}", type_param.enum_values);

    // Tool with array param
    let batch = Tool::new("batch_process", "Process multiple items")
        .with_param(
            "items",
            ToolParam::array("Items to process", ToolParam::string("Item")),
        );

    println!("\n  ✓ Tool with array param");
    println!("    name: {}", batch.name);
    let items_param = batch.parameters.properties.get("items").unwrap();
    println!("    items.type: {}", items_param.param_type);
    println!(
        "    items.items: {:?}",
        items_param.items.as_ref().map(|p| &p.param_type)
    );

    // Disabled tool
    let disabled = Tool::new("dangerous_action", "Do something dangerous").disabled();
    println!("\n  ✓ Tool.disabled()");
    println!("    enabled: {}", disabled.is_enabled());
    assert!(!disabled.is_enabled());

    // Serialization
    let json = serde_json::to_string_pretty(&calculator).unwrap();
    println!("\n  ✓ Tool serialization");
    println!("    JSON preview: {}...", &json[..100.min(json.len())]);

    println!("\n  ✅ TEST 3 PASSED");
}

// ============================================================================
// TEST 4: ChatAgent with Mock LLM
// ============================================================================

async fn test_chat_agent() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 4: ChatAgent with Mock LLM");
    println!("{}", "=".repeat(70));

    // Create mock LLM
    let llm = MockLLM::new("mock-gpt-4").with_responses(vec![
        "Hello! I'm ready to help you with coding.",
        "Here's a simple factorial function:\n```rust\nfn factorial(n: u64) -> u64 {\n    (1..=n).product()\n}\n```",
        "Great question! The function uses Rust's iterator methods.",
    ]);

    // Create agent
    let agent = ChatAgent::new("coding-assistant", llm)
        .with_system_prompt("You are a helpful coding assistant specializing in Rust.")
        .with_description("A coding assistant that helps with Rust programming");

    println!("\n  ✓ ChatAgent::new()");
    println!("    id: {}", agent.id().as_str());
    println!("    name: {}", agent.name());
    println!("    description: {:?}", agent.description());
    println!(
        "    system_prompt: {:?}",
        agent.system_prompt().map(|s| &s[..30.min(s.len())])
    );

    // Create thread
    let mut thread = agent.new_thread();
    println!("\n  ✓ agent.new_thread()");
    println!("    thread_id: {}", thread.id.as_str());

    // First turn: greeting
    let start = Instant::now();
    let response = agent
        .run(vec![ChatMessage::user("Hello!")], &mut thread)
        .await
        .unwrap();

    println!("\n  ✓ agent.run() - Turn 1");
    println!("    duration: {:?}", start.elapsed());
    println!(
        "    content: {}...",
        &response.content[..50.min(response.content.len())]
    );
    println!("    tokens_used: {:?}", response.tokens_used);
    println!("    thread messages: {}", thread.message_count());

    // Second turn: code request
    let response = agent
        .run(
            vec![ChatMessage::user("Write a factorial function in Rust")],
            &mut thread,
        )
        .await
        .unwrap();

    println!("\n  ✓ agent.run() - Turn 2");
    println!(
        "    content: {}...",
        &response.content[..50.min(response.content.len())]
    );
    println!("    thread messages: {}", thread.message_count());
    println!("    turn_count: {}", thread.turn_count);

    // Third turn: follow-up
    let response = agent
        .run(
            vec![ChatMessage::user("Can you explain how it works?")],
            &mut thread,
        )
        .await
        .unwrap();

    println!("\n  ✓ agent.run() - Turn 3");
    println!(
        "    content: {}...",
        &response.content[..50.min(response.content.len())]
    );
    println!("    total messages: {}", thread.message_count());
    println!("    total turns: {}", thread.turn_count);
    println!("    total tokens: {}", thread.total_tokens);

    assert_eq!(thread.turn_count, 3);
    assert_eq!(thread.message_count(), 6); // 3 user + 3 assistant

    println!("\n  ✅ TEST 4 PASSED");
}

// ============================================================================
// TEST 5: AgentExt Convenience Methods
// ============================================================================

async fn test_agent_ext() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 5: AgentExt Convenience Methods");
    println!("{}", "=".repeat(70));

    let llm = MockLLM::new("mock-model");
    let agent = ChatAgent::new("helper", llm);

    // invoke() - single message convenience
    let mut thread = AgentThread::new();
    let response = agent.invoke("Hello there!", &mut thread).await.unwrap();

    println!("\n  ✓ agent.invoke()");
    println!("    input: \"Hello there!\"");
    println!(
        "    response: {}...",
        &response.content[..40.min(response.content.len())]
    );
    println!("    thread messages: {}", thread.message_count());

    // run_once() - stateless single turn
    let response = agent.run_once("What is 2+2?").await.unwrap();

    println!("\n  ✓ agent.run_once()");
    println!("    input: \"What is 2+2?\"");
    println!(
        "    response: {}...",
        &response.content[..40.min(response.content.len())]
    );
    println!("    (no thread needed)");

    // new_thread() - create thread for agent
    let new_thread = agent.new_thread();
    println!("\n  ✓ agent.new_thread()");
    println!("    thread_id: {}", new_thread.id.as_str());
    println!("    created_by: {:?}", new_thread.created_by);

    println!("\n  ✅ TEST 5 PASSED");
}

// ============================================================================
// TEST 6: Streaming with run_stream()
// ============================================================================

async fn test_streaming() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 6: Streaming with run_stream()");
    println!("{}", "=".repeat(70));

    let llm = MockLLM::new("mock-streaming")
        .with_responses(vec!["This is a streaming response that will be sent in chunks."]);

    let agent = ChatAgent::new("streamer", llm);
    let mut thread = AgentThread::new();

    let mut stream = agent.run_stream(vec![ChatMessage::user("Stream me a response")], &mut thread);

    println!("\n  ✓ agent.run_stream()");
    println!("    Receiving chunks:");

    let mut chunk_count = 0;
    let mut full_content = String::new();
    let mut final_done = false;

    while let Some(chunk) = stream.next().await {
        chunk_count += 1;
        if let Some(content) = &chunk.content {
            full_content.push_str(content);
            print!("    chunk {}: \"{}\"", chunk_count, content);
        }
        if chunk.done {
            final_done = true;
            println!(" [DONE]");
            if let Some(tokens) = chunk.tokens_used {
                println!("    tokens_used: {}", tokens);
            }
        } else {
            println!();
        }
    }

    println!("\n  Summary:");
    println!("    total chunks: {}", chunk_count);
    println!(
        "    final content: {}...",
        &full_content[..40.min(full_content.len())]
    );
    println!("    received done: {}", final_done);

    assert!(final_done);
    assert!(chunk_count > 1);

    // Drop stream to release mutable borrow
    drop(stream);

    println!(
        "    thread messages after stream: {}",
        thread.message_count()
    );

    println!("\n  ✅ TEST 6 PASSED");
}

// ============================================================================
// TEST 7: Agent with Tools
// ============================================================================

async fn test_agent_with_tools() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 7: Agent with Tools");
    println!("{}", "=".repeat(70));

    let llm = MockLLM::new("mock-tools");

    // Create tools
    let weather_tool = Tool::new("get_weather", "Get weather for a location")
        .with_param("location", ToolParam::string("City name"))
        .with_optional_param(
            "units",
            ToolParam::string_enum("Temperature units", vec!["celsius", "fahrenheit"]),
        );

    let calc_tool = Tool::new("calculator", "Evaluate math expressions")
        .with_param("expression", ToolParam::string("Math expression"));

    // Create agent with tools
    let agent = ChatAgent::new("assistant", llm)
        .with_tool(weather_tool)
        .with_tool(calc_tool)
        .with_system_prompt("You can use tools to help users.");

    println!("\n  ✓ ChatAgent with tools");
    println!("    tool_count: {}", agent.tools().len());
    for tool in agent.tools() {
        println!("    - {}: {}", tool.name, tool.description);
    }

    // Verify tools are accessible
    assert_eq!(agent.tools().len(), 2);
    assert_eq!(agent.tools()[0].name, "get_weather");
    assert_eq!(agent.tools()[1].name, "calculator");

    println!("\n  ✅ TEST 7 PASSED");
}

// ============================================================================
// TEST 8: AgentResponse and AgentChunk
// ============================================================================

fn test_response_types() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 8: AgentResponse and AgentChunk");
    println!("{}", "=".repeat(70));

    // AgentResponse
    let messages = vec![
        ChatMessage::user("Hello"),
        ChatMessage::assistant("Hi there!"),
    ];

    let response = AgentResponse::with_messages("Hi there!", messages)
        .with_tokens(50)
        .with_model("gpt-4")
        .with_truncated(false);

    println!("\n  ✓ AgentResponse::with_messages()");
    println!("    content: {}", response.content);
    println!("    tokens_used: {:?}", response.tokens_used);
    println!("    model: {:?}", response.model);
    println!("    truncated: {}", response.truncated);
    println!("    messages: {}", response.messages.len());

    // AgentChunk - content
    let content_chunk = AgentChunk::content("Hello ");
    println!("\n  ✓ AgentChunk::content()");
    println!("    content: {:?}", content_chunk.content);
    println!("    done: {}", content_chunk.done);

    // AgentChunk - done
    let done_chunk = AgentChunk::done();
    println!("\n  ✓ AgentChunk::done()");
    println!("    done: {}", done_chunk.done);

    // AgentChunk - error
    let error_chunk = AgentChunk::error("Connection failed");
    println!("\n  ✓ AgentChunk::error()");
    println!("    error: {:?}", error_chunk.error);

    println!("\n  ✅ TEST 8 PASSED");
}

// ============================================================================
// TEST 9: Error Handling
// ============================================================================

fn test_error_handling() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 9: Error Handling");
    println!("{}", "=".repeat(70));

    // Create various errors
    let llm_error = AgentError::llm("API rate limit exceeded");
    let workflow_error = AgentError::workflow("Executor failed");
    let thread_error = AgentError::thread("Thread not found");
    let tool_error = AgentError::tool("calculator", "Division by zero");
    let timeout_error = AgentError::timeout("30s");
    let invalid_error = AgentError::invalid_input("Empty message");

    println!("\n  ✓ AgentError variants");
    println!("    LLM: {}", llm_error);
    println!("    Workflow: {}", workflow_error);
    println!("    Thread: {}", thread_error);
    println!("    Tool: {}", tool_error);
    println!("    Timeout: {}", timeout_error);
    println!("    Invalid: {}", invalid_error);

    // Test is_retriable
    println!("\n  ✓ is_retriable()");
    println!("    LLM error: {}", llm_error.is_retriable());
    println!("    Timeout error: {}", timeout_error.is_retriable());
    println!("    Invalid input: {}", invalid_error.is_retriable());

    assert!(llm_error.is_retriable());
    assert!(timeout_error.is_retriable());
    assert!(!invalid_error.is_retriable());

    println!("\n  ✅ TEST 9 PASSED");
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() {
    println!("\n{}", "═".repeat(70));
    println!("     AGENT DEMO - Phase 6 Agent Layer Verification");
    println!("{}", "═".repeat(70));

    let start = Instant::now();

    // Synchronous tests
    test_chat_messages();
    test_agent_thread();
    test_tools();
    test_response_types();
    test_error_handling();

    // Async tests
    test_chat_agent().await;
    test_agent_ext().await;
    test_streaming().await;
    test_agent_with_tools().await;

    println!("\n{}", "═".repeat(70));
    println!("     ALL TESTS PASSED! ({:?})", start.elapsed());
    println!("{}", "═".repeat(70));
    println!("\n  Phase 6 Agent Layer Summary:");
    println!("  - ✅ ChatMessage, ChatRole: Message types with serialization");
    println!("  - ✅ AgentThread: Conversation state management");
    println!("  - ✅ Tool, ToolParam: Function calling definitions");
    println!("  - ✅ ChatAgent: LLM wrapper with run() and run_stream()");
    println!("  - ✅ AgentExt: Convenience methods (invoke, run_once, new_thread)");
    println!("  - ✅ AgentChunk: Streaming response chunks");
    println!("  - ✅ AgentError: Error handling with is_retriable()");
    println!("\n");
}
