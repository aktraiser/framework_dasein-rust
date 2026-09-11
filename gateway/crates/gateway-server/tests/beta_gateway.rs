use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    routing::post,
};
use futures::StreamExt;
use gateway_core::storage::Storage;
use gateway_server::{create_router, AppState};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

fn config(base: &str) -> gateway_core::config::GatewayConfig {
    serde_json::from_value(json!({"server":{},"sandbox":{},"llm":{"providers":{"openai":{"enabled":true,"api_key":"fake-test-key","api_base":base}},"models":{"beta-test":{"provider":"openai"}}}})).unwrap()
}
fn request(method: &str, uri: &str, key: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
async fn body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn embeddings_uses_configured_provider_and_embedding_usage() {
    let upstream = axum::Router::new().route("/embeddings",post(|axum::Json(request):axum::Json<Value>| async move {
        assert_eq!(request["model"], "beta-test");
        axum::Json(json!({"object":"list","model":"beta-test","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"usage":{"prompt_tokens":2,"total_tokens":2}}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut state = AppState::new(config(&base));
    state.operator_api_key = Some("test".into());
    let response = create_router(state)
        .oneshot(request(
            "POST",
            "/v1/embeddings",
            "test",
            json!({"model":"beta-test","input":"hello"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value = body(response).await;
    assert_eq!(value["data"][0]["embedding"], json!([0.1, 0.2]));
    assert_eq!(value["usage"]["completion_tokens"], 0);
    task.abort();
}

#[tokio::test]
async fn streaming_delivers_utf8_before_upstream_finishes() {
    use gateway_llm::{provider::Provider, providers::OpenAIProvider};
    let release = Arc::new(tokio::sync::Notify::new());
    let signal = release.clone();
    let upstream = axum::Router::new().route(
        "/chat/completions",
        post(move || {
            let signal = signal.clone();
            async move {
                let first =
                    b"data: {\"choices\":[{\"delta\":{\"content\":\"caf\xc3\xa9\"}}]}\r\n\r\n";
                let chunks = futures::stream::iter(
                    first
                        .iter()
                        .map(|b| Ok::<_, std::io::Error>(vec![*b]))
                        .collect::<Vec<_>>(),
                );
                let last = futures::stream::once(async move {
                    signal.notified().await;
                    Ok::<_, std::io::Error>(b"data: [DONE]\n\n".to_vec())
                });
                (
                    [("content-type", "text/event-stream")],
                    Body::from_stream(chunks.chain(last)),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let provider = OpenAIProvider::new("fake").with_base_url(&base);
    let mut stream = provider
        .chat_completion_stream(
            serde_json::from_value(json!({"model":"beta-test","messages":[]})).unwrap(),
        )
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&frame).unwrap()["choices"][0]["delta"]["content"],
        "café"
    );
    release.notify_one();
    assert_eq!(stream.next().await.unwrap().unwrap(), "[DONE]");
    assert!(stream.next().await.is_none());
    server.abort();
}

#[tokio::test]
async fn gateway_stream_reports_truncation_without_fake_done() {
    let upstream = axum::Router::new().route(
        "/chat/completions",
        post(|| async {
            (
                [("content-type", "text/event-stream")],
                "data: {\"choices\":[]}\n\n",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut state = AppState::new(config(&base));
    state.operator_api_key = Some("test".into());
    let response = create_router(state)
        .oneshot(request(
            "POST",
            "/v1/chat/completions",
            "test",
            json!({"model":"beta-test","messages":[],"stream":true}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("event: error"));
    assert!(!text.contains("data: [DONE]"));
    server.abort();
}

#[tokio::test]
#[ignore = "Requires Docker and python:3.12-alpine"]
async fn boxes_enforce_ownership_recover_metadata_and_isolate_files() {
    let storage = Storage::in_memory().await.unwrap();
    let (_, alice) = storage.create_api_key("alice", None, None).await.unwrap();
    let (_, bob) = storage.create_api_key("bob", None, None).await.unwrap();
    let mut state = AppState::new(config("http://127.0.0.1:1")).with_storage(storage);
    state.operator_api_key = Some("operator-test".into());
    let app = create_router(state.clone());
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/sandboxes",
            &alice,
            json!({"runtime":"python"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value = body(response).await;
    let id = value["sandbox_id"].as_str().unwrap();
    let uri = format!("/v1/sandboxes/{id}");
    for (method, path) in [
        ("GET", uri.clone()),
        ("POST", format!("{uri}/execute")),
        ("DELETE", uri.clone()),
    ] {
        let response = app
            .clone()
            .oneshot(request(method, &path, &bob, json!({"code":"print(42)"})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    // Simulate a new process: a fresh manager loads persisted ownership/policy.
    state.boxes = Arc::new(gateway_server::boxes::Boxes::default());
    let app = create_router(state);
    for code in [
        "open('/workspace/secret','w').write('alice'); print(input())",
        "import os; assert not os.path.exists('/workspace/secret'); print('ok')",
    ] {
        let response = app
            .clone()
            .oneshot(request(
                "POST",
                &format!("{uri}/execute"),
                &alice,
                json!({"code":code,"stdin":"hello\n"}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value = body(response).await;
        assert_eq!(value["exit_code"], 0, "{value}");
    }
    assert_eq!(
        app.clone()
            .oneshot(request("DELETE", &uri, &alice, json!({})))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(request("GET", &uri, &alice, json!({})))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[ignore = "Requires Docker and python:3.12-alpine"]
async fn boxes_reject_excess_limits_and_delete_during_execution() {
    let storage = Storage::in_memory().await.unwrap();
    let (_, key) = storage.create_api_key("owner", None, None).await.unwrap();
    let state = AppState::new(config("http://127.0.0.1:1")).with_storage(storage);
    let app = create_router(state);
    for body in [
        json!({"runtime":"python","memory_mb":4096}),
        json!({"runtime":"python","timeout_seconds":0}),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request("POST", "/v1/sandboxes", &key, body))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/sandboxes",
            &key,
            json!({"runtime":"python"}),
        ))
        .await
        .unwrap();
    let value = body(response).await;
    let uri = format!("/v1/sandboxes/{}", value["sandbox_id"].as_str().unwrap());
    let execution = app.clone().oneshot(request(
        "POST",
        &format!("{uri}/execute"),
        &key,
        json!({"code":"import time; time.sleep(25)"}),
    ));
    let task = tokio::spawn(execution);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let response = app
            .clone()
            .oneshot(request("GET", &uri, &key, json!({})))
            .await
            .unwrap();
        if body(response).await["status"] == "running" {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        app.clone()
            .oneshot(request(
                "POST",
                &format!("{uri}/execute"),
                &key,
                json!({"code":"print(1)"})
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        app.oneshot(request("DELETE", &uri, &key, json!({})))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let response = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // Let the SDK cancellation guard finish before this test runtime exits.
    tokio::time::sleep(Duration::from_secs(3)).await;
}

#[tokio::test]
#[ignore = "Requires Docker and python:3.12-alpine"]
async fn rust_sdk_client_executes_and_cleans_up_gateway_box() {
    use dasein_agentic_sandbox::{GatewaySandbox, Sandbox};
    let storage = Storage::in_memory().await.unwrap();
    let (_, key) = storage.create_api_key("sdk", None, None).await.unwrap();
    let app = create_router(AppState::new(config("http://127.0.0.1:1")).with_storage(storage));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let sandbox = GatewaySandbox::builder(&base)
        .api_key(key)
        .runtime("python")
        .memory_mb(128)
        .timeout_ms(10000)
        .build()
        .await
        .unwrap();
    assert!(sandbox.session_id().await.is_some());
    let result = sandbox.execute("print(6*7)").await.unwrap();
    assert_eq!(result.stdout, "42\n");
    assert_eq!(result.exit_code, 0);
    sandbox.cleanup().await.unwrap();
    assert!(sandbox.session_id().await.is_none());
    server.abort();
}
