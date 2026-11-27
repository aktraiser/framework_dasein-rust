//! Dashboard protections, per-key limits and usage accounting.
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use gateway_core::storage::Storage;
use gateway_server::{create_router, AppState};
use std::sync::Arc;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PASSWORD: &str = "fake-dashboard-test-password";

/// Environment the gateway reads, set once before any test touches it.
fn setup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::env::set_var("GATEWAY_ADMIN_PASSWORD", PASSWORD);
        std::env::remove_var("FIRECRACKER_REMOTE_URL");
        std::env::remove_var("FIRECRACKER_SSH_PASSWORD");
    });
}

/// A gateway whose `openai` provider, if any, points at `upstream`.
async fn gateway(upstream: Option<&str>) -> (Router, Arc<Storage>) {
    setup();
    let llm = upstream.map_or_else(
        || serde_json::json!({}),
        |base| {
            serde_json::json!({
                "providers": { "openai": { "enabled": true, "api_key": "FAKE_TEST_KEY", "api_base": base } },
                "models": { "test-model": { "provider": "openai" } },
            })
        },
    );
    let config =
        serde_json::from_value(serde_json::json!({ "server": {}, "sandbox": {}, "llm": llm }))
            .unwrap();
    let state = AppState::new(config).with_storage(Storage::in_memory().await.unwrap());
    let storage = state.storage.clone().unwrap();
    (create_router(state), storage)
}

fn login(password: &str, extra: &[(&str, &str)]) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/admin/login")
        .header(header::HOST, "gateway.test")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    for (name, value) in extra {
        request = request.header(*name, *value);
    }
    request
        .body(Body::from(format!("password={password}")))
        .unwrap()
}

fn chat(key: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"model":"test-model","messages":[{"role":"user","content":"hi"}]}"#,
        ))
        .unwrap()
}

#[tokio::test]
async fn login_sets_a_strict_cookie() {
    let (app, _) = gateway(None).await;
    let response = app.oneshot(login(PASSWORD, &[])).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/admin"] {
        assert!(cookie.contains(attribute), "missing {attribute}: {cookie}");
    }
}

#[tokio::test]
async fn cross_site_dashboard_posts_are_refused() {
    let (app, storage) = gateway(None).await;
    // A page on another site submitting the login or an admin form.
    for (name, value) in [
        ("origin", "https://evil.example"),
        ("sec-fetch-site", "cross-site"),
    ] {
        let response = app
            .clone()
            .oneshot(login(PASSWORD, &[(name, value)]))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{name}: {value}");
    }
    // Even with a valid session, a forged cross-site form does nothing.
    let session = storage.create_admin_session().await.unwrap();
    let forged = Request::builder()
        .method("POST")
        .uri("/admin/keys/generate")
        .header(header::HOST, "gateway.test")
        .header(header::ORIGIN, "https://evil.example")
        .header(
            header::COOKIE,
            format!("gateway_admin_session={}", session.id),
        )
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("name=stolen"))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(forged).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert!(storage.list_api_keys().await.unwrap().is_empty());
    // The same form from the dashboard itself goes through.
    let response = app
        .oneshot(login(PASSWORD, &[("origin", "http://gateway.test")]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn repeated_failed_logins_are_locked_out() {
    let (app, _) = gateway(None).await;
    for _ in 0..5 {
        let response = app.clone().oneshot(login("wrong", &[])).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "login page with an error"
        );
    }
    // Now even the right password is refused for this client.
    let response = app.oneshot(login(PASSWORD, &[])).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn requests_per_minute_limit_of_a_key_is_enforced() {
    let (app, storage) = gateway(None).await;
    let (_, limited) = storage
        .create_api_key("limited", Some(2), None)
        .await
        .unwrap();
    let (_, free) = storage.create_api_key("free", None, None).await.unwrap();

    for _ in 0..2 {
        let status = app.clone().oneshot(chat(&limited)).await.unwrap().status();
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    }
    let response = app.clone().oneshot(chat(&limited)).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = response.headers()[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry_after));

    // Another key, and calls that cost nothing, are unaffected.
    let status = app.clone().oneshot(chat(&free)).await.unwrap().status();
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    let models = Request::builder()
        .uri("/v1/models")
        .header(header::AUTHORIZATION, format!("Bearer {limited}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(models).await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn usage_is_logged_and_counts_against_the_token_limit() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "c1", "object": "chat.completion", "created": 0, "model": "test-model",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "hello" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 30, "total_tokens": 42 },
        })))
        .expect(1)
        .mount(&upstream)
        .await;
    let (app, storage) = gateway(Some(&upstream.uri())).await;
    let (record, key) = storage
        .create_api_key("metered", None, Some(40))
        .await
        .unwrap();

    let response = app.clone().oneshot(chat(&key)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let logs = storage.get_usage_logs(10).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].api_key_id.as_deref(), Some(record.id.as_str()));
    assert_eq!(logs[0].provider, "openai");
    assert_eq!(logs[0].model, "test-model");
    assert_eq!((logs[0].input_tokens, logs[0].output_tokens), (12, 30));

    // 42 tokens used against a limit of 40 per minute: the next call waits,
    // and never reaches the provider (the mock expects exactly one call).
    let response = app.oneshot(chat(&key)).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}
