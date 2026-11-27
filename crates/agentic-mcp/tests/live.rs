//! Checks against real MCP servers.
//!
//! Ignored by default; each test skips itself when its variables are not
//! set. See `docs/LIVE_TESTS.md`.
use dasein_agentic_mcp::client::{MCPClientPool, MCPConfig, MCPServerConfig};

fn setting(name: &str) -> Option<String> {
    let value = std::env::var(name).ok().filter(|value| !value.is_empty());
    if value.is_none() {
        eprintln!("skipped: {name} is not set");
    }
    value
}

/// Connect, list the tools, and call one if `LIVE_MCP_TOOL` names it.
async fn exercise(config: MCPServerConfig) {
    let pool = MCPClientPool::new(MCPConfig::new().add_server("live", config))
        .await
        .unwrap();
    let errors = pool.connection_errors().await;
    assert!(errors.is_empty(), "could not connect: {errors:?}");

    let tools = pool.list_tools("live").await.unwrap();
    assert!(!tools.is_empty(), "the server lists no tools");
    eprintln!(
        "tools: {}",
        tools
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    if let Ok(tool) = std::env::var("LIVE_MCP_TOOL") {
        let arguments = std::env::var("LIVE_MCP_ARGUMENTS").ok().map_or_else(
            || serde_json::json!({}),
            |raw| serde_json::from_str(&raw).expect("LIVE_MCP_ARGUMENTS must be JSON"),
        );
        let result = pool.call_tool("live", &tool, arguments).await.unwrap();
        assert!(
            !result.is_error,
            "tool reported an error: {}",
            result.text()
        );
        eprintln!(
            "result: {}",
            result.text().chars().take(300).collect::<String>()
        );
    }

    // Generated TypeScript must be writable for whatever names the server uses.
    let out = tempfile::tempdir().unwrap();
    pool.generate_typescript(out.path()).await.unwrap();
}

#[tokio::test]
#[ignore = "starts a real MCP server process; see docs/LIVE_TESTS.md"]
async fn stdio_server_live() {
    // For example: LIVE_MCP_COMMAND="npx -y @modelcontextprotocol/server-everything"
    let Some(command) = setting("LIVE_MCP_COMMAND") else {
        return;
    };
    let mut parts = command.split_whitespace().map(str::to_owned);
    let program = parts.next().expect("LIVE_MCP_COMMAND is empty");
    exercise(MCPServerConfig::stdio(program, parts.collect())).await;
}

#[tokio::test]
#[ignore = "calls a real MCP server over HTTP; see docs/LIVE_TESTS.md"]
async fn http_server_live() {
    let Some(url) = setting("LIVE_MCP_URL") else {
        return;
    };
    let mut config = MCPServerConfig::http(url);
    if let Ok(key) = std::env::var("LIVE_MCP_API_KEY") {
        config = config.header("API_KEY", key);
    }
    exercise(config).await;
}
