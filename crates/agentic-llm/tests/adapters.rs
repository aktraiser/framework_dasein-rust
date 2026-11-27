//! Anthropic, Gemini and Ollama adapters against mock servers.
#![cfg(all(feature = "anthropic", feature = "gemini", feature = "ollama"))]

use dasein_agentic_llm::{
    AnthropicAdapter, FinishReason, GeminiAdapter, LLMAdapter, LLMError, LLMMessage, OllamaAdapter,
    StreamChunk, Tool,
};
use futures::StreamExt;
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(events: &[serde_json::Value]) -> ResponseTemplate {
    let mut body = String::new();
    for event in events {
        body.push_str("event: message\ndata: ");
        body.push_str(&event.to_string());
        body.push_str("\n\n");
    }
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

async fn collect(adapter: &dyn LLMAdapter) -> Vec<Result<StreamChunk, LLMError>> {
    adapter
        .generate_stream(&[LLMMessage::user("hi")])
        .collect()
        .await
}

fn text(chunks: &[Result<StreamChunk, LLMError>]) -> String {
    chunks
        .iter()
        .map(|chunk| chunk.as_ref().unwrap().content.as_str())
        .collect()
}

// ---------------------------------------------------------------- Ollama

fn ollama(server: &MockServer) -> OllamaAdapter {
    OllamaAdapter::new("test-model").with_base_url(server.uri())
}

#[tokio::test]
async fn ollama_stream_yields_text_usage_and_one_final_chunk() {
    let server = MockServer::start().await;
    let lines = [
        json!({ "message": { "content": "Bonjour " }, "done": false }),
        json!({ "message": { "content": "à tous {}" }, "done": false }),
        json!({ "message": { "content": "" }, "done": true, "done_reason": "length",
                "prompt_eval_count": 7, "eval_count": 5 }),
    ];
    let body = lines.map(|line| line.to_string()).join("\n") + "\n";
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/x-ndjson"))
        .mount(&server)
        .await;

    let chunks = collect(&ollama(&server)).await;
    assert_eq!(text(&chunks), "Bonjour à tous {}");
    let last = chunks.last().unwrap().as_ref().unwrap();
    assert!(last.done);
    assert_eq!(last.finish_reason, Some(FinishReason::Length));
    let usage = last.tokens_used.as_ref().unwrap();
    assert_eq!((usage.prompt, usage.completion, usage.total), (7, 5, 12));
    assert_eq!(
        chunks.iter().filter(|c| c.as_ref().unwrap().done).count(),
        1
    );
}

#[tokio::test]
async fn ollama_stream_reports_failures_instead_of_ending_quietly() {
    // The server fails halfway.
    let server = MockServer::start().await;
    let body = format!(
        "{}\n{}\n",
        json!({ "message": { "content": "partial" }, "done": false }),
        json!({ "error": "out of memory" })
    );
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/x-ndjson"))
        .mount(&server)
        .await;
    let chunks = collect(&ollama(&server)).await;
    assert!(matches!(
        chunks.last().unwrap(),
        Err(LLMError::ApiError(message)) if message == "out of memory"
    ));

    // The connection closes before the model said it was done.
    let server = MockServer::start().await;
    let body = format!(
        "{}\n",
        json!({ "message": { "content": "cut" }, "done": false })
    );
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/x-ndjson"))
        .mount(&server)
        .await;
    let chunks = collect(&ollama(&server)).await;
    assert!(matches!(
        chunks.last().unwrap(),
        Err(LLMError::ConnectionError(_))
    ));

    // The model does not exist: the stream opens with an error status.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({ "error": "model 'x' not found" })),
        )
        .mount(&server)
        .await;
    let chunks = collect(&ollama(&server)).await;
    assert!(matches!(
        &chunks[0],
        Err(LLMError::ModelNotFound(message)) if message.contains("not found")
    ));
}

#[tokio::test]
async fn overloaded_server_is_retried_but_a_rejected_request_is_not() {
    let answer = json!({ "message": { "content": "ok" }, "done": true });

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&answer))
        .expect(1)
        .mount(&server)
        .await;
    let response = ollama(&server)
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap();
    assert_eq!(response.content, "ok");

    // A 400 would be a 400 again: one attempt only.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "bad request" })))
        .expect(1)
        .mount(&server)
        .await;
    let error = ollama(&server)
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap_err();
    assert!(matches!(error, LLMError::ApiError(message) if message.contains("bad request")));

    // Retries can be turned off.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    assert!(ollama(&server)
        .with_max_retries(0)
        .generate(&[LLMMessage::user("hi")])
        .await
        .is_err());
}

// -------------------------------------------------------------- Anthropic

fn anthropic(server: &MockServer) -> AnthropicAdapter {
    AnthropicAdapter::new("fake-test-key", "test-model")
        .with_base_url(server.uri())
        .with_max_retries(0)
}

#[tokio::test]
async fn anthropic_sends_every_system_message_and_reads_mixed_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "fake-test-key"))
        .and(body_string_contains("Be brief."))
        .and(body_string_contains("Answer in French."))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "test-model", "stop_reason": "end_turn",
            "content": [
                { "type": "thinking", "thinking": "hmm", "signature": "s" },
                { "type": "text", "text": "Bonjour" },
            ],
            "usage": { "input_tokens": 10, "output_tokens": 3, "cache_read_input_tokens": 4 },
        })))
        .expect(1)
        .mount(&server)
        .await;

    let response = anthropic(&server)
        .generate(&[
            LLMMessage::system("Be brief."),
            LLMMessage::user("hi"),
            LLMMessage::system("Answer in French."),
        ])
        .await
        .unwrap();
    assert_eq!(response.content, "Bonjour");
    assert_eq!(response.tokens_used.prompt, 14);
    assert_eq!(response.tokens_used.total, 17);
}

#[tokio::test]
async fn anthropic_tool_use_survives_unknown_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "test-model", "stop_reason": "tool_use",
            "content": [
                { "type": "server_tool_use", "id": "x", "name": "web_search", "input": {} },
                { "type": "text", "text": "Let me check." },
                { "type": "tool_use", "id": "call_1", "name": "lookup", "input": { "q": "rust" } },
            ],
            "usage": { "input_tokens": 1, "output_tokens": 1 },
        })))
        .mount(&server)
        .await;

    let tool = Tool::new("lookup", "Look something up");
    let response = anthropic(&server)
        .generate_with_tools(&[LLMMessage::user("hi")], &[tool])
        .await
        .unwrap();
    assert_eq!(response.content, "Let me check.");
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "lookup");
    assert_eq!(response.finish_reason, FinishReason::ToolCalls);
}

#[tokio::test]
async fn anthropic_stream_reports_usage_once_at_the_end() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"stream\":true"))
        .respond_with(sse(&[
            json!({ "type": "message_start", "message": { "usage": { "input_tokens": 10 } } }),
            json!({ "type": "content_block_delta", "delta": { "type": "thinking_delta", "thinking": "…" } }),
            json!({ "type": "content_block_delta", "delta": { "type": "text_delta", "text": "Bon" } }),
            json!({ "type": "content_block_delta", "delta": { "type": "text_delta", "text": "jour é" } }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "max_tokens" },
                    "usage": { "output_tokens": 5 } }),
            json!({ "type": "message_stop" }),
        ]))
        .mount(&server)
        .await;

    let chunks = collect(&anthropic(&server)).await;
    assert_eq!(text(&chunks), "Bonjour é");
    assert_eq!(chunks.len(), 3, "two text chunks and one final chunk");
    let last = chunks.last().unwrap().as_ref().unwrap();
    assert!(last.done);
    assert_eq!(last.finish_reason, Some(FinishReason::Length));
    let usage = last.tokens_used.as_ref().unwrap();
    assert_eq!((usage.prompt, usage.completion, usage.total), (10, 5, 15));
}

#[tokio::test]
async fn anthropic_errors_are_explicit_and_keep_the_key_out() {
    // An error event in the middle of a stream.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            json!({ "type": "content_block_delta", "delta": { "text": "par" } }),
            json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } }),
        ]))
        .mount(&server)
        .await;
    let chunks = collect(&anthropic(&server)).await;
    assert!(matches!(
        chunks.last().unwrap(),
        Err(LLMError::ApiError(message)) if message == "Overloaded"
    ));

    // A rejected key.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" }
        })))
        .mount(&server)
        .await;
    let error = anthropic(&server)
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap_err();
    assert!(matches!(&error, LLMError::AuthenticationError(m) if m == "invalid x-api-key"));

    // Nobody listening: the message names neither the key nor the URL.
    let unreachable = AnthropicAdapter::new("fake-test-key", "test-model")
        .with_base_url("http://127.0.0.1:1/secret-in-the-url")
        .with_max_retries(0);
    let error = unreachable
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap_err()
        .to_string();
    assert!(
        !error.contains("fake-test-key") && !error.contains("secret-in-the-url"),
        "{error}"
    );
}

// ----------------------------------------------------------------- Gemini

fn gemini(server: &MockServer) -> GeminiAdapter {
    GeminiAdapter::new("fake-test-key", "test-model")
        .with_base_url(server.uri())
        .with_max_retries(0)
}

#[tokio::test]
async fn gemini_stream_keeps_code_with_braces_intact() {
    let server = MockServer::start().await;
    let part = |text: &str| json!({ "candidates": [{ "content": { "role": "model", "parts": [{ "text": text }] } }] });
    Mock::given(method("POST"))
        .and(path("/v1beta/models/test-model:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        // The key is in a header, never in the URL.
        .and(header("x-goog-api-key", "fake-test-key"))
        .respond_with(sse(&[
            part("fn main() { "),
            part("println!(\"{}\", \"é}\"); }"),
            json!({
                "candidates": [{ "content": { "parts": [{ "text": "" }] }, "finishReason": "STOP" }],
                "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 9, "totalTokenCount": 13 },
            }),
        ]))
        .mount(&server)
        .await;

    let chunks = collect(&gemini(&server)).await;
    assert_eq!(text(&chunks), "fn main() { println!(\"{}\", \"é}\"); }");
    let last = chunks.last().unwrap().as_ref().unwrap();
    assert!(last.done);
    assert_eq!(last.finish_reason, Some(FinishReason::Stop));
    assert_eq!(last.tokens_used.as_ref().unwrap().total, 13);
}

#[tokio::test]
async fn gemini_explains_blocked_prompts_and_withheld_answers() {
    // The prompt itself was refused: no candidate at all.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/test-model:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "promptFeedback": { "blockReason": "SAFETY" }
        })))
        .mount(&server)
        .await;
    let error = gemini(&server)
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap_err();
    assert!(
        matches!(&error, LLMError::ApiError(m) if m.contains("SAFETY")),
        "{error}"
    );

    // The answer was withheld: a candidate without content.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{ "finishReason": "SAFETY" }],
            "usageMetadata": { "promptTokenCount": 3 },
        })))
        .mount(&server)
        .await;
    let response = gemini(&server)
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap();
    assert_eq!(response.content, "");
    assert_eq!(response.finish_reason, FinishReason::Error);
    assert_eq!(response.tokens_used.prompt, 3);
}

#[tokio::test]
async fn gemini_tool_calls_and_system_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("Be brief."))
        .and(body_string_contains("Answer in French."))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": { "role": "model", "parts": [
                    { "functionCall": { "name": "lookup", "args": { "q": "rust" } } }
                ] },
                "finishReason": "STOP",
            }],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let tool = Tool::new("lookup", "Look something up");
    let response = gemini(&server)
        .generate_with_tools(
            &[
                LLMMessage::system("Be brief."),
                LLMMessage::system("Answer in French."),
                LLMMessage::user("hi"),
            ],
            &[tool],
        )
        .await
        .unwrap();
    assert_eq!(response.tool_calls[0].name, "lookup");
    assert_eq!(response.finish_reason, FinishReason::ToolCalls);
}

#[tokio::test]
async fn gemini_stream_surfaces_errors_and_keeps_usage_from_earlier_chunks() {
    // Usage arrives with a text chunk; the final chunk has none.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            json!({
                "candidates": [{ "content": { "parts": [{ "text": "ok" }] } }],
                "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 2, "totalTokenCount": 6 },
            }),
            json!({ "candidates": [{ "finishReason": "STOP" }] }),
        ]))
        .mount(&server)
        .await;
    let chunks = collect(&gemini(&server)).await;
    let last = chunks.last().unwrap().as_ref().unwrap();
    assert_eq!(last.tokens_used.as_ref().unwrap().total, 6);

    // The API fails after the stream has started.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            json!({ "candidates": [{ "content": { "parts": [{ "text": "par" }] } }] }),
            json!({ "error": { "code": 429, "message": "Resource exhausted", "status": "RESOURCE_EXHAUSTED" } }),
        ]))
        .mount(&server)
        .await;
    let chunks = collect(&gemini(&server)).await;
    assert!(matches!(
        chunks.last().unwrap(),
        Err(LLMError::RateLimitError(message)) if message == "Resource exhausted"
    ));
}

#[tokio::test]
async fn server_error_after_processing_is_not_replayed() {
    // A 500 does not say the request was left untouched: one attempt only,
    // even with retries enabled.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream failed"))
        .expect(1)
        .mount(&server)
        .await;
    let error = ollama(&server)
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap_err();
    assert!(matches!(error, LLMError::ApiError(message) if message.contains("500")));
}

#[tokio::test]
async fn silence_limit_is_configurable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "message": { "content": "late" }, "done": true }))
                .set_delay(std::time::Duration::from_secs(3)),
        )
        .mount(&server)
        .await;
    let error = ollama(&server)
        .with_timeout(std::time::Duration::from_millis(200))
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap_err();
    assert!(matches!(error, LLMError::Timeout), "{error}");
}

#[tokio::test]
async fn streamed_call_after_tool_rounds_keeps_the_tool_exchange_readable() {
    use dasein_agentic_llm::ToolCall;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        // What the assistant asked for and what came back are both there,
        // tied together by the call ID, and no native tool block is sent
        // (the API rejects them without tool definitions).
        .and(body_string_contains("Called tool `weather` (call c1)"))
        .and(body_string_contains("[Result of tool call c1]"))
        .respond_with(sse(&[
            json!({ "type": "content_block_delta", "delta": { "text": "12°C" } }),
            json!({ "type": "message_stop" }),
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let history = [
        LLMMessage::user("Weather in Paris?"),
        LLMMessage::assistant_with_tool_calls(
            "",
            vec![ToolCall::new("c1", "weather", json!({ "city": "Paris" }))],
        ),
        LLMMessage::tool_result("c1", "12°C"),
    ];
    let chunks: Vec<_> = anthropic(&server).generate_stream(&history).collect().await;
    assert_eq!(text(&chunks), "12°C");

    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&requests[0].body);
    assert!(
        !body.contains("tool_use") && !body.contains("tool_result"),
        "{body}"
    );
}

#[tokio::test]
async fn gemini_sends_back_the_thought_signature_of_a_tool_call() {
    let server = MockServer::start().await;
    // Second round: the call comes back with its signature, the result after.
    Mock::given(method("POST"))
        .and(body_string_contains("functionResponse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": { "role": "model", "parts": [{ "text": "done" }] },
                "finishReason": "STOP",
            }],
        })))
        .expect(1)
        .mount(&server)
        .await;
    // First round: a thinking model signs the call it asks for.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": { "role": "model", "parts": [{
                    "functionCall": { "name": "lookup", "args": { "q": "rust" } },
                    "thoughtSignature": "c2lnbmF0dXJl",
                }] },
                "finishReason": "STOP",
            }],
        })))
        .mount(&server)
        .await;

    let adapter = gemini(&server);
    let tools = [Tool::new("lookup", "Look something up")];
    let question = LLMMessage::user("hi");
    let first = adapter
        .generate_with_tools(std::slice::from_ref(&question), &tools)
        .await
        .unwrap();
    let call = first.tool_calls[0].clone();
    let history = [
        question,
        LLMMessage::assistant_with_tool_calls("", first.tool_calls),
        LLMMessage::tool_result(call.id, "found"),
    ];
    let second = adapter.generate_with_tools(&history, &tools).await.unwrap();
    assert_eq!(second.content, "done");

    let requests = server.received_requests().await.unwrap();
    let replayed: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    let part = &replayed["contents"][1]["parts"][0];
    assert_eq!(part["functionCall"]["name"], "lookup");
    assert_eq!(part["thoughtSignature"], "c2lnbmF0dXJl", "{part}");
    // A call that had no signature is replayed without the field.
    let unsigned: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(!unsigned.to_string().contains("thoughtSignature"));
}

#[tokio::test]
async fn ollama_declares_tools_reads_calls_and_sends_results_back() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {"role": "assistant", "content": "", "tool_calls": [
                {"function": {"name": "get_weather", "arguments": {"city": "Paris"}}},
                // Some models send the arguments as a string.
                {"function": {"name": "get_time", "arguments": "{\"zone\":\"CET\"}"}},
            ]},
            "done": true, "done_reason": "stop", "prompt_eval_count": 9, "eval_count": 4,
        })))
        .mount(&server)
        .await;
    let tools = [
        Tool::new("get_weather", "Weather of a city").with_string_param("city", "Name"),
        Tool::new("get_time", "Time in a zone").with_string_param("zone", "Zone"),
    ];
    let adapter = ollama(&server);
    assert!(adapter.supports_tools());

    let question = LLMMessage::user("Weather and time in Paris?");
    let first = adapter
        .generate_with_tools(std::slice::from_ref(&question), &tools)
        .await
        .unwrap();
    assert_eq!(first.finish_reason, FinishReason::ToolCalls);
    assert_eq!(first.tokens_used.total, 13);
    let [weather, time] = &first.tool_calls[..] else {
        panic!("{:?}", first.tool_calls)
    };
    assert_eq!(
        (weather.name.as_str(), &weather.arguments),
        ("get_weather", &json!({"city": "Paris"}))
    );
    assert_eq!(
        (time.name.as_str(), &time.arguments),
        ("get_time", &json!({"zone": "CET"}))
    );
    // Ollama gives no IDs: the adapter makes distinct ones.
    assert!(!weather.id.is_empty() && weather.id != time.id);

    // The next round carries the calls and each result under its tool's name.
    adapter
        .generate_with_tools(
            &[
                question,
                LLMMessage::assistant_with_tool_calls("", first.tool_calls.clone()),
                LLMMessage::tool_result(time.id.clone(), "14:05"),
                LLMMessage::tool_result(weather.id.clone(), "12°C"),
            ],
            &tools,
        )
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let declared: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        declared["tools"][0],
        json!({"type": "function", "function": {
            "name": "get_weather", "description": "Weather of a city",
            "parameters": tools[0].parameters,
        }})
    );
    assert_eq!(declared["stream"], false);
    let sent: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(
        sent["messages"],
        json!([
            {"role": "user", "content": "Weather and time in Paris?"},
            {"role": "assistant", "content": "", "tool_calls": [
                {"type": "function", "function": {"name": "get_weather", "arguments": {"city": "Paris"}}},
                {"type": "function", "function": {"name": "get_time", "arguments": {"zone": "CET"}}},
            ]},
            {"role": "tool", "tool_name": "get_time", "content": "14:05"},
            {"role": "tool", "tool_name": "get_weather", "content": "12°C"},
        ])
    );
}

#[tokio::test]
async fn ollama_reports_a_model_without_tools_and_answers_plainly_without_any() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_string_contains("\"tools\""))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"error": "registry.ollama.ai/library/tiny:latest does not support tools"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {"content": "plain"}, "done": true,
        })))
        .mount(&server)
        .await;
    let adapter = ollama(&server).with_max_retries(0);
    let tool = Tool::new("get_weather", "Weather").with_string_param("city", "Name");
    let error = adapter
        .generate_with_tools(&[LLMMessage::user("hi")], &[tool])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not support tools"), "{error}");
    // Without tools to declare, the request is an ordinary one.
    let answer = adapter
        .generate_with_tools(&[LLMMessage::user("hi")], &[])
        .await
        .unwrap();
    assert_eq!(answer.content, "plain");
    assert!(answer.tool_calls.is_empty());
}
