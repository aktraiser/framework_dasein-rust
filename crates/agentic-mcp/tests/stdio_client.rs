//! The stdio transport against a real server process (`fixtures/mcp_server.py`).
use dasein_agentic_mcp::client::{ContentItem, MCPClientPool, MCPConfig, MCPServerConfig};
use dasein_agentic_mcp::MCPError;
use serde_json::json;

fn server() -> MCPServerConfig {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mcp_server.py");
    MCPServerConfig::stdio("python3", vec![script.into()])
}

async fn pool(config: MCPServerConfig) -> MCPClientPool {
    let pool = MCPClientPool::new(MCPConfig::new().add_server("fixture", config))
        .await
        .unwrap();
    let errors = pool.connection_errors().await;
    assert!(errors.is_empty(), "could not start the fixture: {errors:?}");
    pool
}

#[tokio::test]
async fn discovers_tools_across_pages() {
    let pool = pool(server()).await;
    let names: Vec<String> = pool
        .list_tools("fixture")
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert_eq!(
        names,
        [
            "echo",
            "query-docs",
            "fail",
            "resource",
            "env",
            "slow",
            "exit"
        ]
    );
}

#[tokio::test]
async fn calls_a_tool_despite_interleaved_server_traffic() {
    let pool = pool(server()).await;
    // The fixture sends a notification and its own request before answering.
    for round in 0..3 {
        let text = format!("héllo {round}\nsecond line");
        let result = pool
            .call_tool("fixture", "echo", json!({ "text": text }))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.text(), text);
    }
}

#[tokio::test]
async fn concurrent_calls_get_their_own_answers() {
    let pool = std::sync::Arc::new(pool(server()).await);
    let calls = (0..20).map(|n| {
        let pool = pool.clone();
        tokio::spawn(async move {
            let result = pool
                .call_tool("fixture", "echo", json!({ "text": n.to_string() }))
                .await
                .unwrap();
            (n, result.text())
        })
    });
    for call in calls {
        let (n, text) = call.await.unwrap();
        assert_eq!(text, n.to_string());
    }
}

#[tokio::test]
async fn tool_failure_is_reported_not_hidden() {
    let pool = pool(server()).await;
    let result = pool.call_tool("fixture", "fail", json!({})).await.unwrap();
    assert!(result.is_error);
    assert_eq!(result.text(), "it broke");
}

#[tokio::test]
async fn resources_and_unknown_content_are_kept() {
    let pool = pool(server()).await;
    let result = pool
        .call_tool("fixture", "resource", json!({}))
        .await
        .unwrap();
    assert!(matches!(
        &result.content[0],
        ContentItem::Resource { resource } if resource.text.as_deref() == Some("from a file")
    ));
    assert!(matches!(
        &result.content[1],
        ContentItem::ResourceLink { uri, .. } if uri == "file:///b.txt"
    ));
    assert!(matches!(result.content[2], ContentItem::Unknown));
    assert_eq!(result.structured_content, Some(json!({ "count": 3 })));
}

#[tokio::test]
async fn unknown_tool_is_a_call_error() {
    let pool = pool(server()).await;
    let error = pool
        .call_tool("fixture", "nope", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(error, MCPError::CallFailed(_)), "{error}");
    assert!(error.to_string().contains("Unknown tool"), "{error}");
}

#[tokio::test]
async fn server_gets_configured_env_but_not_ours() {
    // Present in this process, and it must not reach the server.
    std::env::set_var("DASEIN_MCP_TEST_SECRET", "leaked");
    let pool = pool(server().env_var("GREETING", "bonjour")).await;
    let read = |name: &'static str| {
        let pool = &pool;
        async move {
            pool.call_tool("fixture", "env", json!({ "name": name }))
                .await
                .unwrap()
                .text()
        }
    };
    assert_eq!(read("GREETING").await, "bonjour");
    assert_eq!(read("DASEIN_MCP_TEST_SECRET").await, "<unset>");
}

#[tokio::test]
async fn slow_tool_times_out() {
    let pool = pool(server().timeout(500)).await;
    let error = pool
        .call_tool("fixture", "slow", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(error, MCPError::Timeout), "{error}");
}

#[tokio::test]
async fn server_exit_fails_the_call_instead_of_hanging() {
    let pool = pool(server()).await;
    let error = pool
        .call_tool("fixture", "exit", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(error, MCPError::TransportError(_)), "{error}");
    // Later calls fail fast too.
    let error = pool
        .call_tool("fixture", "echo", json!({ "text": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, MCPError::TransportError(_)), "{error}");
}

#[tokio::test]
async fn unreachable_server_is_reported() {
    let pool = MCPClientPool::new(MCPConfig::new().add_server(
        "missing",
        MCPServerConfig::stdio("dasein-no-such-mcp-server", vec![]),
    ))
    .await
    .unwrap();
    assert!(pool.list_servers().await.is_empty());
    let errors = pool.connection_errors().await;
    assert!(errors["missing"].contains("Failed to start"), "{errors:?}");
}
