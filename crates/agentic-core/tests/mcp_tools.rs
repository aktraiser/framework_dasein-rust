//! A ChatAgent using the tools of an MCP server, end to end: a real server
//! process (the MCP crate's test fixture) and a scripted model.
#![cfg(feature = "mcp")]

use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::agent::{Agent, AgentThread, ChatAgent, ChatMessage};
use dasein_agentic_llm::{
    FinishReason, LLMAdapter, LLMError, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage,
    Tool, ToolCall,
};
use dasein_agentic_mcp::client::{MCPClientPool, MCPConfig, MCPServerConfig};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Calls the tool it was told to, then repeats what the tool answered.
struct Scripted {
    tool: &'static str,
    arguments: serde_json::Value,
    offered: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl LLMAdapter for Scripted {
    fn provider(&self) -> &str {
        "scripted"
    }
    fn model(&self) -> &str {
        "scripted"
    }
    fn supports_tools(&self) -> bool {
        true
    }
    async fn generate(&self, _: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        Err(LLMError::ApiError("the agent offered no tools".into()))
    }
    async fn generate_with_tools(
        &self,
        messages: &[LLMMessage],
        tools: &[Tool],
    ) -> Result<LLMResponse, LLMError> {
        *self.offered.lock().unwrap() = tools.iter().map(|t| t.name.clone()).collect();
        let last = messages.last().unwrap();
        let (content, tool_calls) = if last.role == Role::Tool {
            (format!("The tool said: {}", last.content), vec![])
        } else {
            (
                String::new(),
                vec![ToolCall::new("call_1", self.tool, self.arguments.clone())],
            )
        };
        Ok(LLMResponse {
            content,
            tokens_used: TokenUsage::default(),
            finish_reason: FinishReason::Stop,
            model: "scripted".into(),
            tool_calls,
        })
    }
    fn generate_stream(
        &self,
        _: &[LLMMessage],
    ) -> Pin<Box<dyn futures::Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        Box::pin(futures::stream::empty())
    }
    async fn health_check(&self) -> Result<bool, LLMError> {
        Ok(true)
    }
}

async fn pool() -> Arc<MCPClientPool> {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../agentic-mcp/tests/fixtures/mcp_server.py"
    );
    let config = MCPConfig::new().add_server(
        "fixture",
        MCPServerConfig::stdio("python3", vec![script.into()]),
    );
    let pool = MCPClientPool::new(config).await.unwrap();
    let errors = pool.connection_errors().await;
    assert!(errors.is_empty(), "could not start the fixture: {errors:?}");
    Arc::new(pool)
}

async fn ask(tool: &'static str, arguments: serde_json::Value) -> (String, Vec<String>) {
    let offered = Arc::new(Mutex::new(Vec::new()));
    let model = Scripted {
        tool,
        arguments,
        offered: offered.clone(),
    };
    let agent = ChatAgent::new("assistant", model)
        .with_mcp_tools(pool().await)
        .await
        .unwrap();
    let response = agent
        .run(vec![ChatMessage::user("go")], &mut AgentThread::new())
        .await
        .unwrap();
    let offered = offered.lock().unwrap().clone();
    (response.content, offered)
}

#[tokio::test]
async fn agent_discovers_and_calls_mcp_tools() {
    let (answer, offered) = ask("fixture__echo", serde_json::json!({ "text": "bonjour" })).await;
    // Every tool of the server was offered to the model, under a safe name.
    for name in ["fixture__echo", "fixture__query-docs", "fixture__resource"] {
        assert!(
            offered.contains(&name.to_string()),
            "{name} not in {offered:?}"
        );
    }
    // The call reached the server and its answer came back to the model.
    assert!(answer.contains("bonjour"), "{answer}");
}

#[tokio::test]
async fn failing_mcp_tool_is_reported_to_the_model_as_an_error() {
    let (answer, _) = ask("fixture__fail", serde_json::json!({})).await;
    assert!(
        answer.contains("error") && answer.contains("it broke"),
        "{answer}"
    );
}

#[tokio::test]
async fn structured_results_and_non_text_items_are_described() {
    let (answer, _) = ask("fixture__resource", serde_json::json!({})).await;
    // Text of the embedded resource, the structured part, and a note that
    // two items (a link and an unknown type) were not passed as text.
    assert!(answer.contains("from a file"), "{answer}");
    assert!(
        answer.contains("\\\"count\\\":3") || answer.contains("\"count\":3"),
        "{answer}"
    );
    assert!(answer.contains("omitted_non_text_items"), "{answer}");
}
