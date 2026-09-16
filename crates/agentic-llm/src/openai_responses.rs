//! Stateless Responses transport. Provider continuation items travel with the
//! first tool call, so serialized SDK threads retain encrypted reasoning state.
use crate::{
    FinishReason, LLMAdapter, LLMError, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage,
    Tool, ToolCall,
};
use async_trait::async_trait;
use futures::Stream;
use serde_json::{json, Value};
use std::{pin::Pin, time::Duration};

pub struct OpenAIResponsesAdapter {
    client: reqwest::Client,
    api_key: String,
    model: String,
    api_base: String,
    max_tokens: Option<u32>,
    cache_key: Option<String>,
}
impl OpenAIResponsesAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .expect("HTTP client configuration"),
            api_key: api_key.into(),
            model: model.into(),
            api_base: "https://api.openai.com/v1".into(),
            max_tokens: None,
            cache_key: None,
        }
    }
    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into().trim_end_matches('/').into();
        self
    }
    pub fn with_prompt_cache_key(mut self, key: impl Into<String>) -> Self {
        self.cache_key = Some(key.into());
        self
    }
    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }
    pub(crate) fn required(model: &str) -> bool {
        let model = model.strip_prefix("ft:").unwrap_or(model);
        let base = model.split(':').next().unwrap_or(model);
        base.strip_prefix("gpt-")
            .or_else(|| base.strip_prefix("chatgpt-"))
            .and_then(|v| v.split(['.', '-']).next())
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|major| major >= 5)
            || ["o1", "o3", "o4"]
                .iter()
                .any(|p| base == *p || base.starts_with(&format!("{p}-")))
    }
    fn request(&self, messages: &[LLMMessage], tools: &[Tool]) -> Value {
        let mut input = Vec::new();
        for m in messages {
            if m.role == Role::Assistant && !m.tool_calls.is_empty() {
                if let Some(items) = m.tool_calls[0]
                    .provider_metadata
                    .as_ref()
                    .and_then(|v| v["openai_responses_output"].as_array())
                {
                    input.extend(items.iter().cloned());
                } else {
                    if !m.content.is_empty() {
                        input.push(json!({"role":"assistant","content":m.content}));
                    }
                    for call in &m.tool_calls {
                        input.push(json!({"type":"function_call","call_id":call.id,"name":call.name,"arguments":call.arguments.to_string()}));
                    }
                }
            } else if m.role == Role::Tool {
                input.push(json!({"type":"function_call_output","call_id":m.tool_call_id,"output":m.content}));
            } else {
                input.push(json!({"role":match m.role {Role::System=>"developer",Role::User=>"user",_=>"assistant"},"content":m.content}));
            }
        }
        let mut request = json!({"model":self.model,"input":input,"store":false,"include":["reasoning.encrypted_content"],"tools":tools.iter().map(|t|json!({"type":"function","name":t.name,"description":t.description,"parameters":t.parameters,"strict":false})).collect::<Vec<_>>()});
        if let Some(n) = self.max_tokens {
            request["max_output_tokens"] = json!(n);
        }
        if let Some(key) = &self.cache_key {
            request["prompt_cache_key"] = json!(key);
        }
        // Temperature is intentionally absent. Reasoning uses the model default.
        request
    }
    fn parse(body: Value) -> Result<LLMResponse, LLMError> {
        let incomplete = body["status"] == "incomplete";
        if !incomplete && body["status"] != "completed" {
            return Err(LLMError::InvalidResponse(
                "Responses did not complete".into(),
            ));
        }
        let output = body["output"]
            .as_array()
            .ok_or_else(|| LLMError::InvalidResponse("Missing Responses output".into()))?;
        let content = output
            .iter()
            .filter(|i| i["type"] == "message")
            .flat_map(|i| i["content"].as_array().into_iter().flatten())
            .filter_map(|c| c["text"].as_str().or_else(|| c["refusal"].as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        let mut calls = Vec::new();
        if !incomplete {
            for item in output.iter().filter(|i| i["type"] == "function_call") {
                let id = item["call_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| LLMError::InvalidResponse("Missing function call_id".into()))?;
                let name = item["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| LLMError::InvalidResponse("Missing function name".into()))?;
                let args = item["arguments"].as_str().ok_or_else(|| {
                    LLMError::InvalidResponse("Missing function arguments".into())
                })?;
                let args: Value = serde_json::from_str(args).map_err(|_| {
                    LLMError::InvalidResponse("Invalid function arguments JSON".into())
                })?;
                if !args.is_object() {
                    return Err(LLMError::InvalidResponse(
                        "Function arguments must be an object".into(),
                    ));
                }
                calls.push(ToolCall::new(id, name, args));
            }
            if let Some(first) = calls.first_mut() {
                first.provider_metadata = Some(json!({"openai_responses_output":output}));
            }
        }
        if !incomplete && calls.is_empty() && content.is_empty() {
            return Err(LLMError::EmptyResponse);
        }
        let n = |key: &str| {
            body["usage"][key]
                .as_u64()
                .unwrap_or(0)
                .min(u64::from(u32::MAX)) as u32
        };
        Ok(LLMResponse {
            content,
            model: body["model"].as_str().unwrap_or("").into(),
            tokens_used: TokenUsage {
                details: Some(body["usage"].clone()),
                prompt: n("input_tokens"),
                completion: n("output_tokens"),
                total: n("total_tokens"),
            },
            finish_reason: if incomplete {
                FinishReason::Length
            } else if calls.is_empty() {
                FinishReason::Stop
            } else {
                FinishReason::ToolCalls
            },
            tool_calls: calls,
        })
    }
}
// Do not expose request URLs, headers, proxy credentials or raw source errors.
fn transport_error(error: reqwest::Error) -> LLMError {
    if error.is_timeout() {
        return LLMError::Timeout;
    }
    let mut reason = if error.is_connect() {
        "DNS/TCP/TLS ou proxy"
    } else {
        "transport interrompu pendant la requête ou la lecture"
    };
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        let value = cause.to_string().to_ascii_lowercase();
        if value.contains("dns") || value.contains("resolve") {
            reason = "résolution DNS";
        } else if value.contains("certificate") {
            reason = "certificat TLS";
        } else if value.contains("connection refused") {
            reason = "connexion refusée";
        } else if value.contains("network is unreachable") || value.contains("no route to host") {
            reason = "réseau inaccessible";
        }
        source = cause.source();
    }
    LLMError::ConnectionError(format!("Échec de connexion : {reason}"))
}

#[async_trait]
impl LLMAdapter for OpenAIResponsesAdapter {
    fn provider(&self) -> &str {
        "openai"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn supports_tools(&self) -> bool {
        true
    }
    async fn generate(&self, m: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        self.generate_with_tools(m, &[]).await
    }
    async fn generate_with_tools(
        &self,
        m: &[LLMMessage],
        t: &[Tool],
    ) -> Result<LLMResponse, LLMError> {
        let mut response = self
            .client
            .post(format!("{}/responses", self.api_base))
            .bearer_auth(&self.api_key)
            .timeout(Duration::from_secs(120))
            .json(&self.request(m, t))
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err(LLMError::InvalidResponse("Response exceeds 4 MiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&bytes).map_err(|_| {
            LLMError::InvalidResponse(format!("HTTP {} returned invalid JSON", status.as_u16()))
        })?;
        if !status.is_success() {
            let msg = body["error"]["message"]
                .as_str()
                .unwrap_or("OpenAI request rejected")
                .replace(&self.api_key, "[redacted]")
                .chars()
                .take(2000)
                .collect::<String>();
            return Err(match status.as_u16() {
                401 | 403 => LLMError::AuthenticationError(msg),
                429 => LLMError::RateLimitError(msg),
                _ => LLMError::ApiError(format!("HTTP {}: {msg}", status.as_u16())),
            });
        }
        Self::parse(body)
    }
    fn generate_stream(
        &self,
        m: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        let m = m.to_vec();
        Box::pin(futures::stream::once(async move {
            let r = self.generate(&m).await?;
            Ok(StreamChunk {
                content: r.content,
                done: true,
                tokens_used: Some(r.tokens_used),
                finish_reason: Some(r.finish_reason),
            })
        }))
    }
    async fn health_check(&self) -> Result<bool, LLMError> {
        let r = self
            .client
            .get(format!("{}/models/{}", self.api_base, self.model))
            .bearer_auth(&self.api_key)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|_| LLMError::ConnectionError("Model catalog unavailable".into()))?;
        Ok(r.status().is_success())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenAIAdapter;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };
    #[test]
    fn routing_and_request_compatibility() {
        for m in [
            "gpt-5.5",
            "gpt-5.6-luna",
            "gpt-5.5-2026-04-23",
            "gpt-6-astra",
            "o3",
            "o4-mini",
            "ft:gpt-5:org:custom",
        ] {
            assert!(OpenAIResponsesAdapter::required(m), "{m}");
        }
        for m in ["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo", "mock-model"] {
            assert!(!OpenAIResponsesAdapter::required(m));
        }
        let req = OpenAIResponsesAdapter::new("test", "gpt-5.5")
            .with_max_tokens(1024)
            .request(
                &[LLMMessage::system("rules"), LLMMessage::user("go")],
                &[Tool::new("lookup", "lookup").with_optional_param("query", "string", "Query")],
            );
        assert!(req.get("temperature").is_none());
        assert_eq!(req["store"], false);
        assert_eq!(req["max_output_tokens"], 1024);
        assert_eq!(req["input"][0]["role"], "developer");
        assert_eq!(req["tools"][0]["name"], "lookup");
        assert_eq!(req["tools"][0]["strict"], false);
    }
    #[tokio::test]
    async fn responses_tool_round_trip_preserves_encrypted_items_and_call_ids() {
        let server = MockServer::start().await;
        let output = json!([
            {"id":"rs_1","type":"reasoning","summary":[],"encrypted_content":"opaque-test-state"},
            {"id":"msg_1","type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Checking.","annotations":[]}]},
            {"id":"fc_1","type":"function_call","call_id":"call_1","name":"lookup","arguments":"{}","status":"completed"},
            {"id":"fc_2","type":"function_call","call_id":"call_2","name":"lookup","arguments":"{}","status":"completed"}
        ]);
        Mock::given(method("POST")).and(path("/v1/responses")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"completed","model":"gpt-5.6-luna","output":output,"usage":{"input_tokens":10,"output_tokens":30,"total_tokens":40}}))).expect(1).mount(&server).await;
        let adapter = OpenAIAdapter::new("fake-test-key", "gpt-5.6-luna")
            .with_api_base(format!("{}/v1", server.uri()))
            .with_max_tokens(1024)
            .with_temperature(0.7);
        let first = adapter
            .generate_with_tools(
                &[LLMMessage::user("lookup")],
                &[Tool::new("lookup", "lookup")],
            )
            .await
            .unwrap();
        assert_eq!(first.tool_calls[0].id, "call_1");
        assert_eq!(first.tokens_used.total, 40);
        let restored: Vec<ToolCall> =
            serde_json::from_str(&serde_json::to_string(&first.tool_calls).unwrap()).unwrap();
        let messages = vec![
            LLMMessage::user("lookup"),
            LLMMessage::assistant_with_tool_calls(first.content, restored),
            LLMMessage::tool_result("call_1", "42"),
            LLMMessage::tool_result("call_2", "43"),
            LLMMessage::user("Forum: new question"),
        ];
        server.verify().await;
        server.reset().await;
        Mock::given(method("POST")).and(path("/v1/responses")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"completed","model":"gpt-5.6-luna","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}],"usage":{"input_tokens":42,"output_tokens":3,"total_tokens":45}}))).expect(1).mount(&server).await;
        let final_response = adapter
            .generate_with_tools(&messages, &[Tool::new("lookup", "lookup")])
            .await
            .unwrap();
        assert_eq!(final_response.content, "Done");
        assert_eq!(final_response.finish_reason, FinishReason::Stop);
        let requests = server.received_requests().await.unwrap();
        let req: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(req.get("temperature").is_none());
        for i in 0..4 {
            assert_eq!(req["input"][i + 1], output[i]);
        }
        assert_eq!(
            req["input"][5],
            json!({"type":"function_call_output","call_id":"call_1","output":"42"})
        );
        assert_eq!(req["input"][7]["content"], "Forum: new question");
        server.verify().await;
    }
    #[test]
    fn incomplete_batch_is_discarded_without_losing_token_usage() {
        let r=OpenAIResponsesAdapter::parse(json!({"status":"incomplete","model":"gpt-5.5","output":[{"type":"function_call","call_id":"x","name":"mutate","arguments":"{broken"}],"usage":{"input_tokens":5,"output_tokens":1024,"total_tokens":1029}})).unwrap();
        assert_eq!(r.finish_reason, FinishReason::Length);
        assert!(r.tool_calls.is_empty());
        assert_eq!(r.tokens_used.total, 1029);
        assert!(OpenAIResponsesAdapter::parse(json!({"status":"completed","output":[{"type":"function_call","call_id":"x","name":"test","arguments":"oops"}]})).is_err());
    }
    #[tokio::test]
    async fn provider_rejection_is_not_retried_or_reported_as_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(json!({"error":{"message":"bad fake-test-key"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let err = OpenAIResponsesAdapter::new("fake-test-key", "gpt-5.5")
            .with_api_base(format!("{}/v1", server.uri()))
            .generate(&[LLMMessage::user("test")])
            .await
            .unwrap_err();
        assert!(matches!(err, LLMError::AuthenticationError(_)));
        assert!(!err.to_string().contains("fake-test-key"));
        server.verify().await;
    }
    #[test]
    fn provider_usage_subdivisions_are_preserved() {
        let r=OpenAIResponsesAdapter::parse(serde_json::json!({"status":"completed","model":"gpt-5.5","output":[{"type":"message","content":[{"type":"output_text","text":"Done"}]}],"usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":800},"output_tokens":100,"output_tokens_details":{"reasoning_tokens":50},"total_tokens":1100}})).unwrap();
        assert_eq!(r.tokens_used.total, 1100);
        assert_eq!(
            r.tokens_used.details.as_ref().unwrap()["input_tokens_details"]["cached_tokens"],
            800
        );
        assert_eq!(
            r.tokens_used.details.unwrap()["output_tokens_details"]["reasoning_tokens"],
            50
        );
    }
}
