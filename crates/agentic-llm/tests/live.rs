//! Checks against the real provider APIs.
//!
//! These calls cost money and need credentials, so every test is ignored by
//! default and skips itself when its variables are not set. See
//! `docs/LIVE_TESTS.md` for the variables and the command to run them.
#![cfg(all(
    feature = "openai",
    feature = "anthropic",
    feature = "gemini",
    feature = "ollama"
))]

use dasein_agentic_llm::{
    AnthropicAdapter, FinishReason, GeminiAdapter, LLMAdapter, LLMMessage, OllamaAdapter,
    OpenAIAdapter, Tool,
};
use futures::StreamExt;

/// Read the variables a test needs, or say which one is missing and skip.
fn settings<const N: usize>(names: [&str; N]) -> Option<[String; N]> {
    let mut values = Vec::with_capacity(N);
    for name in names {
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => values.push(value),
            _ => {
                eprintln!("skipped: {name} is not set");
                return None;
            }
        }
    }
    values.try_into().ok()
}

/// The same three checks for every adapter: a plain answer with usage, a
/// streamed answer that ends properly, and (where supported) a tool call.
async fn exercise(adapter: &dyn LLMAdapter) {
    let name = adapter.provider().to_owned();
    let question = [
        LLMMessage::system("Answer with a single word."),
        LLMMessage::user("What is the capital of France?"),
    ];

    let answer = adapter.generate(&question).await.expect("generate");
    assert!(
        answer.content.to_lowercase().contains("paris"),
        "{name}: unexpected answer {:?}",
        answer.content
    );
    assert!(answer.tokens_used.total > 0, "{name}: no token usage");

    let chunks: Vec<_> = adapter.generate_stream(&question).collect().await;
    let mut streamed = String::new();
    for chunk in &chunks {
        streamed.push_str(&chunk.as_ref().expect("stream chunk").content);
    }
    assert!(
        streamed.to_lowercase().contains("paris"),
        "{name}: unexpected stream {streamed:?}"
    );
    let last = chunks.last().expect("empty stream").as_ref().unwrap();
    assert!(last.done, "{name}: stream did not end with a final chunk");
    assert!(
        last.tokens_used
            .as_ref()
            .is_some_and(|usage| usage.total > 0),
        "{name}: no token usage on the final stream chunk"
    );

    if adapter.supports_tools() {
        let tool = Tool::new("get_weather", "Get the current weather for a city")
            .with_string_param("city", "Name of the city");
        let response = adapter
            .generate_with_tools(
                &[LLMMessage::user(
                    "What is the weather in Paris? Use the tool.",
                )],
                &[tool],
            )
            .await
            .expect("generate_with_tools");
        assert_eq!(
            response.tool_calls.len(),
            1,
            "{name}: expected one tool call, got {:?}",
            response.content
        );
        assert_eq!(response.tool_calls[0].name, "get_weather");
        assert_eq!(response.finish_reason, FinishReason::ToolCalls, "{name}");
    }
}

#[tokio::test]
#[ignore = "calls the real Anthropic API; see docs/LIVE_TESTS.md"]
async fn anthropic_live() {
    let Some([key, model]) = settings(["ANTHROPIC_API_KEY", "LIVE_ANTHROPIC_MODEL"]) else {
        return;
    };
    exercise(&AnthropicAdapter::new(key, model).with_max_tokens(256)).await;
}

#[tokio::test]
#[ignore = "calls the real Gemini API; see docs/LIVE_TESTS.md"]
async fn gemini_live() {
    let Some([key, model]) = settings(["GEMINI_API_KEY", "LIVE_GEMINI_MODEL"]) else {
        return;
    };
    exercise(&GeminiAdapter::new(key, model)).await;
}

#[tokio::test]
#[ignore = "calls the real OpenAI API; see docs/LIVE_TESTS.md"]
async fn openai_live() {
    let Some([key, model]) = settings(["OPENAI_API_KEY", "LIVE_OPENAI_MODEL"]) else {
        return;
    };
    exercise(&OpenAIAdapter::new(key, model)).await;
}

#[tokio::test]
#[ignore = "needs a running Ollama server; see docs/LIVE_TESTS.md"]
async fn ollama_live() {
    let Some([model]) = settings(["LIVE_OLLAMA_MODEL"]) else {
        return;
    };
    let mut adapter = OllamaAdapter::new(model);
    if let Ok(url) = std::env::var("LIVE_OLLAMA_URL") {
        adapter = adapter.with_base_url(url);
    }
    exercise(&adapter).await;
}

#[tokio::test]
#[ignore = "calls the real Anthropic API; see docs/LIVE_TESTS.md"]
async fn rejected_key_is_an_authentication_error() {
    // No valid key is needed for this one, only the model name.
    let Some([model]) = settings(["LIVE_ANTHROPIC_MODEL"]) else {
        return;
    };
    let error = AnthropicAdapter::new("invalid-key-for-a-live-test", model)
        .with_max_retries(0)
        .generate(&[LLMMessage::user("hi")])
        .await
        .unwrap_err();
    assert!(
        matches!(error, dasein_agentic_llm::LLMError::AuthenticationError(_)),
        "{error}"
    );
}
