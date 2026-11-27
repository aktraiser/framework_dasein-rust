//! Chat completion API

use crate::middleware::auth::Principal;
use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use gateway_core::llm::ChatCompletionRequest;
use serde::{Deserialize, Serialize};

/// Price of a request in USD, from the per-1K-token prices configured for
/// its model. Zero when the model has no configured price.
fn cost_usd(state: &AppState, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    state.config.llm.models.get(model).map_or(0.0, |prices| {
        f64::from(input_tokens) / 1000.0 * prices.input_cost_per_1k
            + f64::from(output_tokens) / 1000.0 * prices.output_cost_per_1k
    })
}

/// Record what a request consumed: in the usage log shown on the dashboard,
/// and against the key's tokens-per-minute limit. Never fails the request.
pub(crate) async fn record_usage(
    state: &AppState,
    principal: Option<&Principal>,
    provider: &str,
    model: &str,
    usage: Option<&gateway_core::llm::Usage>,
    started: std::time::Instant,
) {
    let Some(storage) = &state.storage else {
        return;
    };
    let api_key_id = principal.and_then(Principal::api_key_id);
    let (input, output) = usage.map_or((0, 0), |u| (u.prompt_tokens, u.completion_tokens));
    if let (Some(id), Some(usage)) = (api_key_id, usage) {
        if let Err(error) = storage.record_tokens(id, usage.total_tokens).await {
            tracing::warn!(%error, "Failed to count tokens against the key's limit");
        }
    }
    let clamp = |value: u128| i32::try_from(value).unwrap_or(i32::MAX);
    if let Err(error) = storage
        .log_usage(
            api_key_id,
            provider,
            model,
            clamp(u128::from(input)),
            clamp(u128::from(output)),
            cost_usd(state, model, input, output),
            clamp(started.elapsed().as_millis()),
        )
        .await
    {
        tracing::warn!(%error, "Failed to record usage");
    }
}

/// Rough token count of a text when the provider gave none: about four
/// characters per token, rounded up.
fn estimate_tokens(chars: usize) -> u32 {
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
}

/// Usage of a streamed answer, recorded when the stream is dropped: after its
/// last chunk, or earlier if the client goes away. Either way the request is
/// logged and counted. If the provider's own figures did not arrive (the
/// client left first, or the provider reports none), they are estimated from
/// the text that went through, so closing a stream early is not free.
struct StreamUsage {
    state: AppState,
    principal: Option<Principal>,
    provider: String,
    model: String,
    started: std::time::Instant,
    usage: Option<gateway_core::llm::Usage>,
    prompt_chars: usize,
    answer_chars: usize,
}

impl StreamUsage {
    /// Take what a chunk tells about usage: the provider's figures if it
    /// carries them, and the length of its text for the fallback estimate.
    fn observe(&mut self, chunk: &serde_json::Value) {
        if let Some(usage) = chunk
            .get("usage")
            .and_then(|usage| serde_json::from_value(usage.clone()).ok())
        {
            self.usage = Some(usage);
        }
        let choices = chunk.get("choices").and_then(serde_json::Value::as_array);
        for choice in choices.into_iter().flatten() {
            if let Some(text) = choice.pointer("/delta/content").and_then(|c| c.as_str()) {
                self.answer_chars += text.chars().count();
            }
        }
    }
}

impl Drop for StreamUsage {
    fn drop(&mut self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let usage = self.usage.take().unwrap_or_else(|| {
            let prompt_tokens = estimate_tokens(self.prompt_chars);
            let completion_tokens = estimate_tokens(self.answer_chars);
            gateway_core::llm::Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens.saturating_add(completion_tokens),
            }
        });
        let (state, principal) = (self.state.clone(), self.principal.take());
        let (provider, model) = (
            std::mem::take(&mut self.provider),
            std::mem::take(&mut self.model),
        );
        let started = self.started;
        runtime.spawn(async move {
            record_usage(
                &state,
                principal.as_ref(),
                &provider,
                &model,
                Some(&usage),
                started,
            )
            .await;
        });
    }
}

/// POST /v1/chat/completions
pub async fn chat_completions(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<Response, ApiError> {
    let started = std::time::Instant::now();
    let principal = principal.map(|Extension(principal)| principal);
    let model = request.model.clone();
    let router = state
        .runtime_router()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if request.stream.unwrap_or(false) {
        use axum::response::sse::{Event, KeepAlive, Sse};
        use futures::StreamExt;
        use gateway_llm::provider::ProviderError;
        // The provider is asked for the usage of the stream, so it can be
        // counted. The chunk carrying it is passed on only to clients that
        // asked for it themselves.
        let client_options = request.stream_options.take();
        let client_wants_usage = client_options
            .as_ref()
            .and_then(|options| options.include_usage)
            .unwrap_or(false);
        let prompt_chars = request
            .messages
            .iter()
            .map(|message| message.content.chars().count())
            .sum();
        let provider = router.provider_name(&model).unwrap_or_default();
        let mut metered = request.clone();
        metered.stream_options = Some(gateway_core::llm::StreamOptions {
            include_usage: Some(true),
        });
        let stream = match router.chat_completion_stream(metered).await {
            // A provider that does not know `stream_options` rejects the
            // request: send it as the client wrote it, and estimate usage.
            Err(ProviderError::Api {
                status: 400 | 422, ..
            }) => {
                request.stream_options = client_options;
                router.chat_completion_stream(request).await
            }
            other => other,
        }
        .map_err(|e| ApiError::Provider(e.to_string()))?;
        let mut recorder = StreamUsage {
            state: state.clone(),
            principal,
            provider,
            model,
            started,
            usage: None,
            prompt_chars,
            answer_chars: 0,
        };
        let events = stream.filter_map(move |item| {
            let event = match item {
                Ok(data) => {
                    let chunk = serde_json::from_str::<serde_json::Value>(&data).ok();
                    if let Some(chunk) = &chunk {
                        recorder.observe(chunk);
                    }
                    // A chunk with usage and no choices exists only because
                    // the gateway asked for it, whatever shape the usage has.
                    let usage_only = chunk.as_ref().is_some_and(|chunk| {
                        chunk.get("usage").is_some_and(|usage| !usage.is_null())
                            && chunk
                                .get("choices")
                                .and_then(serde_json::Value::as_array)
                                .map_or(true, Vec::is_empty)
                    });
                    (client_wants_usage || !usage_only).then(|| Event::default().data(data))
                }
                Err(error) => Some(Event::default().event("error").data(
                    serde_json::json!({"error":{"message":error.to_string(),"type":"upstream_error"}})
                        .to_string(),
                )),
            };
            futures::future::ready(event.map(Ok::<_, std::convert::Infallible>))
        });
        return Ok(Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response());
    }
    // Options that only make sense for a stream are not sent with a plain
    // request, which providers would reject.
    request.stream_options = None;
    // Get fallback providers for this model
    let fallbacks = state
        .config
        .llm
        .models
        .get(&model)
        .map(|m| m.fallbacks.clone())
        .unwrap_or_default();

    // Route the request
    let (response, provider) = router
        .chat_completion_with_provider(request, &fallbacks)
        .await
        .map_err(|e| ApiError::Provider(e.to_string()))?;
    record_usage(
        &state,
        principal.as_ref(),
        &provider,
        &model,
        response.usage.as_ref(),
        started,
    )
    .await;

    Ok(Json(response).into_response())
}

/// GET /v1/models
pub async fn list_models(State(state): State<AppState>) -> Json<ModelsResponse> {
    let models: Vec<ModelInfo> = state
        .config
        .llm
        .models
        .keys()
        .map(|name| ModelInfo {
            id: name.clone(),
            object: "model".to_string(),
            owned_by: "agentic-gateway".to_string(),
        })
        .collect();

    Json(ModelsResponse {
        object: "list".to_string(),
        data: models,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: String,
    pub owned_by: String,
}

/// API error type
#[derive(Debug)]
pub enum ApiError {
    Provider(String),
    BadRequest(String),
    NotFound(String),
    Internal(String),
    Unavailable(String),
    ServiceUnavailable(String),
    Busy(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Busy(msg) => (StatusCode::TOO_MANY_REQUESTS, msg),
            ApiError::ServiceUnavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg),
            ApiError::Provider(msg) => (StatusCode::BAD_GATEWAY, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::Unavailable(msg) => (StatusCode::NOT_IMPLEMENTED, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        let body = serde_json::json!({
            "error": {
                "message": message,
                "type": "api_error",
            }
        });

        (status, Json(body)).into_response()
    }
}
