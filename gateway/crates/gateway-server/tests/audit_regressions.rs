use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use gateway_server::{create_router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn audit_anonymous_execution_is_rejected() {
    // In-process routing: no socket, no provider, no remote machine.
    std::env::remove_var("FIRECRACKER_REMOTE_URL");
    std::env::remove_var("FIRECRACKER_SSH_PASSWORD");
    let config =
        serde_json::from_value(serde_json::json!({"server":{}, "llm":{}, "sandbox":{}})).unwrap();
    let app = create_router(AppState::new(config));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"code":"print(1)","runtime":"python"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "unauthenticated execution reached handler: {}",
        response.status()
    );
}

#[tokio::test]
async fn audit_configured_model_is_registered() {
    std::env::remove_var("FIRECRACKER_REMOTE_URL");
    std::env::remove_var("FIRECRACKER_SSH_PASSWORD");
    let config = serde_json::from_value(serde_json::json!({"server":{},"sandbox":{},"llm":{"providers":{"openai":{"enabled":true,"api_key":"FAKE_AUDIT_KEY","api_base":"http://127.0.0.1:1"}},"models":{"audit-model":{"provider":"openai"}}}})).unwrap();
    let state = AppState::new(config);
    let request = serde_json::from_value(
        serde_json::json!({"model":"audit-model","messages":[{"role":"user","content":"test"}]}),
    )
    .unwrap();
    let result = state.llm_router.chat_completion(request, &[]).await;
    // A wired provider would attempt the local URL and report connection failure.
    assert!(
        !matches!(
            result,
            Err(gateway_llm::provider::ProviderError::ModelNotSupported(_))
        ),
        "configured model absent from runtime registry"
    );
}

#[tokio::test]
async fn api_key_permissions_are_enforced() {
    let storage = gateway_core::storage::Storage::in_memory().await.unwrap();
    let (_, key) = storage
        .create_api_key("llm-only", None, None)
        .await
        .unwrap();
    let config =
        serde_json::from_value(serde_json::json!({"server":{},"llm":{},"sandbox":{}})).unwrap();
    let mut state = AppState::new(config).with_storage(storage);
    state.operator_api_key = Some("fake-operator-test-key".into());
    let app = create_router(state);
    for (route, token, expected) in [
        ("/v1/models", key.as_str(), StatusCode::OK),
        (
            "/v1/files?path=/tmp/test",
            key.as_str(),
            StatusCode::FORBIDDEN,
        ),
        (
            "/v1/sandboxes/unknown",
            "fake-operator-test-key",
            StatusCode::NOT_FOUND,
        ),
        ("/v1/models", "wrong", StatusCode::UNAUTHORIZED),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(route)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{route}");
    }
    assert_eq!(
        app.oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::OK
    );
}
