//! A real model discovering and calling the tools of an MCP server, through
//! a `ChatAgent`. Calls the real Gemini API: ignored unless asked for, and
//! skipped unless `GEMINI_API_KEY` and `LIVE_GEMINI_MODEL` are set.
//!
//! Models that sign their tool calls (Gemini 3 and later, the `-latest`
//! aliases) need the adapter to send the signature back: until that fix is
//! in, this test fails on the round after the tool call with such a model.
//!
//! ```bash
//! LIVE_GEMINI_MODEL=gemini-flash-latest cargo test -p dasein-agentic-core \
//!   --features mcp --test live_mcp_agent -- --include-ignored --nocapture
//! ```
#![cfg(feature = "mcp")]

use dasein_agentic_core::distributed::graph::agent::{Agent, AgentThread, ChatAgent, ChatMessage};
use dasein_agentic_llm::GeminiAdapter;
use dasein_agentic_mcp::client::{MCPClientPool, MCPConfig, MCPServerConfig};
use std::sync::Arc;

#[tokio::test]
#[ignore = "calls the real Gemini API"]
async fn real_model_discovers_and_calls_an_mcp_tool() {
    let (Ok(key), Ok(model)) = (
        std::env::var("GEMINI_API_KEY"),
        std::env::var("LIVE_GEMINI_MODEL"),
    ) else {
        eprintln!("skipped: GEMINI_API_KEY or LIVE_GEMINI_MODEL is not set");
        return;
    };

    // The server records the calls it receives in this file.
    let calls = std::env::temp_dir().join(format!("orders-mcp-{}.jsonl", uuid::Uuid::new_v4()));
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/orders_mcp_server.py"
    );
    let config = MCPConfig::new().add_server(
        "orders",
        MCPServerConfig::stdio(
            "python3",
            vec![script.into(), calls.to_string_lossy().into_owned()],
        ),
    );
    let pool = MCPClientPool::new(config).await.unwrap();
    let errors = pool.connection_errors().await;
    assert!(errors.is_empty(), "could not start the server: {errors:?}");

    // Nothing about the tools is told to the agent: it gets them from the
    // server.
    let agent = ChatAgent::new("support", GeminiAdapter::new(key, model))
        .with_mcp_tools(Arc::new(pool))
        .await
        .unwrap();
    let response = agent
        .run(
            vec![ChatMessage::user(
                "Where is order 4172 right now? Give the city and the reference.",
            )],
            &mut AgentThread::new(),
        )
        .await
        .unwrap();
    println!("answer: {}", response.content);

    // The server really was called, with the order from the question.
    let received = std::fs::read_to_string(&calls).unwrap_or_default();
    let _ = std::fs::remove_file(&calls);
    println!("calls received by the server:\n{received}");
    let received: Vec<serde_json::Value> = received
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        received
            .iter()
            .any(|call| call["name"] == "get_order_status"
                && call["arguments"]["order_id"]
                    .as_str()
                    .is_some_and(|id| id.contains("4172"))),
        "the server was not asked about order 4172: {received:?}"
    );
    // And what it answered, which the model could not know, is in the reply.
    assert!(
        response.content.contains("Reykjavik"),
        "{}",
        response.content
    );
    assert!(
        response.content.contains("BLUE-PALLET-88"),
        "{}",
        response.content
    );
}
