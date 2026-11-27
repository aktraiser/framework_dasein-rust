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

/// A gateway built from the given `llm` and `server` configuration.
async fn gateway_with(llm: serde_json::Value, server: serde_json::Value) -> (Router, Arc<Storage>) {
    setup();
    let config =
        serde_json::from_value(serde_json::json!({ "server": server, "sandbox": {}, "llm": llm }))
            .unwrap();
    let state = AppState::new(config).with_storage(Storage::in_memory().await.unwrap());
    let storage = state.storage.clone().unwrap();
    (create_router(state), storage)
}

/// `test-model` served by an `openai` provider at `upstream`, priced at
/// $1 per 1K input tokens and $2 per 1K output tokens.
fn priced_model(upstream: &str) -> serde_json::Value {
    serde_json::json!({
        "providers": { "openai": { "enabled": true, "api_key": "FAKE_TEST_KEY", "api_base": upstream } },
        "models": { "test-model": {
            "provider": "openai", "input_cost_per_1k": 1.0, "output_cost_per_1k": 2.0
        } },
    })
}

/// A gateway whose `openai` provider, if any, points at `upstream`.
async fn gateway(upstream: Option<&str>) -> (Router, Arc<Storage>) {
    let llm = upstream.map_or_else(|| serde_json::json!({}), priced_model);
    gateway_with(llm, serde_json::json!({})).await
}

/// Rate limits are counted per clock minute: do not start a test that
/// expects a refusal just before the minute turns.
async fn avoid_minute_boundary() {
    let second = chrono::Utc::now().timestamp().rem_euclid(60);
    if second >= 55 {
        tokio::time::sleep(std::time::Duration::from_secs(61 - second.unsigned_abs())).await;
    }
}

/// Mark a request as coming from `peer`, as the server's listener would.
fn from_peer(mut request: Request<Body>, peer: &str) -> Request<Body> {
    let address: std::net::SocketAddr = format!("{peer}:40000").parse().unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(address));
    request
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
    avoid_minute_boundary().await;
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
    avoid_minute_boundary().await;
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
    // 12 input tokens at $1/1K and 30 output tokens at $2/1K.
    assert!(
        (logs[0].cost_usd - 0.072).abs() < 1e-9,
        "{}",
        logs[0].cost_usd
    );

    // 42 tokens used against a limit of 40 per minute: the next call waits,
    // and never reaches the provider (the mock expects exactly one call).
    let response = app.oneshot(chat(&key)).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn streamed_answers_are_metered_too() {
    let upstream = MockServer::start().await;
    let chunk = |content: &str| {
        serde_json::json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 0, "model": "test-model",
            "choices": [{ "index": 0, "delta": { "content": content }, "finish_reason": null }],
            "usage": null,
        })
    };
    let usage = serde_json::json!({
        "id": "c1", "object": "chat.completion.chunk", "created": 0, "model": "test-model",
        "choices": [],
        "usage": { "prompt_tokens": 7, "completion_tokens": 5, "total_tokens": 12 },
    });
    let body = format!(
        "data: {}\n\ndata: {}\n\ndata: {usage}\n\ndata: [DONE]\n\n",
        chunk("hel"),
        chunk("lo")
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        // The gateway asks the provider for the usage of the stream.
        .and(wiremock::matchers::body_partial_json(
            serde_json::json!({ "stream": true, "stream_options": { "include_usage": true } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .expect(1)
        .mount(&upstream)
        .await;
    let (app, storage) = gateway(Some(&upstream.uri())).await;
    let (record, key) = storage
        .create_api_key("streamer", None, None)
        .await
        .unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"model":"test-model","stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let received = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let received = String::from_utf8(received.to_vec()).unwrap();
    assert!(
        received.contains("hel") && received.contains("[DONE]"),
        "{received}"
    );
    // This client did not ask for the usage chunk, so it does not get it.
    assert!(!received.contains("prompt_tokens"), "{received}");

    // Usage is recorded once the stream is finished.
    let mut logs = Vec::new();
    for _ in 0..100 {
        logs = storage.get_usage_logs(10).await.unwrap();
        if !logs.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(logs.len(), 1, "streamed request was not logged");
    assert_eq!(logs[0].api_key_id.as_deref(), Some(record.id.as_str()));
    assert_eq!((logs[0].input_tokens, logs[0].output_tokens), (7, 5));
    assert!(
        (logs[0].cost_usd - 0.017).abs() < 1e-9,
        "{}",
        logs[0].cost_usd
    );
}

#[tokio::test]
async fn login_lockout_follows_the_client_behind_a_trusted_proxy() {
    let server = serde_json::json!({ "trusted_proxies": ["10.0.0.0/24"] });
    let (app, _) = gateway_with(serde_json::json!({}), server).await;
    let attempt = |password: &'static str, peer: &'static str, forwarded: &'static str| {
        let app = app.clone();
        async move {
            let request = login(password, &[("x-forwarded-for", forwarded)]);
            app.oneshot(from_peer(request, peer))
                .await
                .unwrap()
                .status()
        }
    };
    // One client behind the proxy uses up its attempts...
    for _ in 0..5 {
        assert_eq!(
            attempt("wrong", "10.0.0.1", "198.51.100.4").await,
            StatusCode::OK
        );
    }
    assert_eq!(
        attempt(PASSWORD, "10.0.0.1", "198.51.100.4").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // ...without locking out another client behind the same proxy.
    assert_eq!(
        attempt(PASSWORD, "10.0.0.1", "198.51.100.5").await,
        StatusCode::SEE_OTHER
    );
    // A direct client cannot dodge its own lockout by inventing the header.
    for fake in ["1.1.1.1", "2.2.2.2", "3.3.3.3", "4.4.4.4", "5.5.5.5"] {
        assert_eq!(attempt("wrong", "203.0.113.9", fake).await, StatusCode::OK);
    }
    assert_eq!(
        attempt(PASSWORD, "203.0.113.9", "9.9.9.9").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn session_cookie_is_secure_when_the_dashboard_is_on_https() {
    let server = serde_json::json!({ "trusted_proxies": ["10.0.0.1"] });
    let (app, _) = gateway_with(serde_json::json!({}), server).await;
    let cookie = |request: Request<Body>| {
        let app = app.clone();
        async move {
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::SEE_OTHER);
            response.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .to_owned()
        }
    };
    // Plain HTTP on localhost: no Secure, or the browser would drop it.
    let plain = cookie(login(PASSWORD, &[("origin", "http://gateway.test")])).await;
    assert!(!plain.contains("Secure"), "{plain}");
    // The browser is on an HTTPS page.
    let https = cookie(login(PASSWORD, &[("origin", "https://gateway.test")])).await;
    assert!(https.contains("Secure"), "{https}");
    // A trusted proxy says the client used HTTPS.
    let proxied = from_peer(
        login(PASSWORD, &[("x-forwarded-proto", "https")]),
        "10.0.0.1",
    );
    assert!(cookie(proxied).await.contains("Secure"));
    // The same header from anyone else is ignored.
    let spoofed = from_peer(
        login(PASSWORD, &[("x-forwarded-proto", "https")]),
        "203.0.113.9",
    );
    assert!(!cookie(spoofed).await.contains("Secure"));
}
