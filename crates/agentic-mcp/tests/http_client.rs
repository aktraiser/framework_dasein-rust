//! The HTTP transport against a mock server.
use dasein_agentic_mcp::client::{MCPClientPool, MCPConfig, MCPServerConfig};
use dasein_agentic_mcp::MCPError;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn rpc(id: u64, result: serde_json::Value) -> serde_json::Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Mount the handshake: `initialize` (request 1) opens session `s-1`, and the
/// `initialized` notification must carry that session.
async fn mount_handshake(server: &MockServer) {
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "initialize" })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Mcp-Session-Id", "s-1")
                .set_body_json(rpc(1, json!({ "protocolVersion": "2025-03-26" }))),
        )
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({ "method": "notifications/initialized" }),
        ))
        .and(header("Mcp-Session-Id", "s-1"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(server)
        .await;
}

async fn connect(server: &MockServer) -> MCPClientPool {
    MCPClientPool::new(
        MCPConfig::new().add_server("mock", MCPServerConfig::http(server.uri()).timeout(2000)),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn handshake_session_and_sse_answers() {
    let server = MockServer::start().await;
    mount_handshake(&server).await;
    // Later requests carry the session and the negotiated protocol version.
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "tools/list" })))
        .and(header("Mcp-Session-Id", "s-1"))
        .and(header("MCP-Protocol-Version", "2025-03-26"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc(
            2,
            json!({ "tools": [{ "name": "echo", "inputSchema": { "type": "object" } }] }),
        )))
        .mount(&server)
        .await;
    // The tool answers as an event stream, after an unrelated notification.
    let stream = format!(
        ": ping\n\nevent: message\ndata: {}\n\nevent: message\ndata: {}\n\n",
        json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": {} }),
        rpc(
            3,
            json!({ "content": [{ "type": "text", "text": "héllo" }] })
        ),
    );
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "tools/call" })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(stream, "text/event-stream"))
        .mount(&server)
        .await;

    let pool = connect(&server).await;
    assert!(pool.connection_errors().await.is_empty());
    assert_eq!(pool.list_tools("mock").await.unwrap()[0].name, "echo");
    let result = pool.call_tool("mock", "echo", json!({})).await.unwrap();
    assert_eq!(result.text(), "héllo");
}

#[tokio::test]
async fn http_error_status_is_reported() {
    let server = MockServer::start().await;
    mount_handshake(&server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "tools/list" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc(2, json!({ "tools": [] }))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "tools/call" })))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
        .mount(&server)
        .await;

    let pool = connect(&server).await;
    let error = pool.call_tool("mock", "echo", json!({})).await.unwrap_err();
    assert!(matches!(error, MCPError::TransportError(_)), "{error}");
    assert!(error.to_string().contains("503"), "{error}");
    assert!(error.to_string().contains("upstream down"), "{error}");
}

#[tokio::test]
async fn failed_connection_is_recorded_without_leaking_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("no"))
        .mount(&server)
        .await;

    let config = MCPServerConfig::http(server.uri()).header("API_KEY", "secret-key-123");
    let pool = MCPClientPool::new(MCPConfig::new().add_server("mock", config))
        .await
        .unwrap();
    assert!(pool.list_servers().await.is_empty());
    let errors = pool.connection_errors().await;
    assert!(errors["mock"].contains("401"), "{errors:?}");
    assert!(!errors["mock"].contains("secret-key-123"), "{errors:?}");
}

#[tokio::test]
async fn endpoint_without_handshake_still_works() {
    let server = MockServer::start().await;
    // An older JSON-RPC endpoint: no `initialize`, tools only.
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "initialize" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0", "id": 1,
            "error": { "code": -32601, "message": "Method not found" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "tools/list" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc(2, json!({ "tools": [] }))))
        .mount(&server)
        .await;

    let pool = connect(&server).await;
    assert_eq!(pool.list_servers().await, ["mock"]);
}

#[tokio::test]
async fn generated_typescript_is_safe_for_hostile_names() {
    let server = MockServer::start().await;
    mount_handshake(&server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "method": "tools/list" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc(
            2,
            json!({ "tools": [
                { "name": "query-docs", "description": "close */ evil()", "inputSchema": {} },
                { "name": "query_docs", "inputSchema": {} },
                { "name": "../../escape", "inputSchema": {} },
                { "name": "delete", "inputSchema": {} },
            ] }),
        )))
        .mount(&server)
        .await;

    let pool = connect(&server).await;
    let out = tempfile::tempdir().unwrap();
    pool.generate_typescript(out.path()).await.unwrap();

    let dir = out.path().join("servers/mock");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    // Every file stayed inside the server's directory, under a valid name.
    assert_eq!(
        files,
        [
            "______escape.ts",
            "_delete.ts",
            "index.ts",
            "query_docs.ts",
            "query_docs_2.ts"
        ]
    );
    let tool = std::fs::read_to_string(dir.join("query_docs.ts")).unwrap();
    assert!(tool.contains("export async function query_docs("));
    assert!(tool.contains(r#"callMCPTool("mock", "query-docs", input)"#));
    assert!(!tool.contains("close */"), "comment can be closed: {tool}");
}
