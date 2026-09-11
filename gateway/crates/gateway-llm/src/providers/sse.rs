use crate::provider::{CompletionStream, ProviderError, ProviderResult};
use futures::StreamExt;

pub async fn openai_stream(response: reqwest::Response) -> ProviderResult<CompletionStream> {
    let status = response.status();
    if !status.is_success() {
        return Err(ProviderError::Api {
            status: status.as_u16(),
            message: response.text().await.unwrap_or_default(),
        });
    }
    if !response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| s.starts_with("text/event-stream"))
    {
        return Err(ProviderError::Unavailable(
            "Upstream did not return SSE".into(),
        ));
    }
    Ok(Box::pin(async_stream::try_stream! {
        let mut bytes = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut data = Vec::new();
        let mut event_bytes = 0usize;
        let mut done = false;
        'stream: while let Some(chunk) = bytes.next().await {
            let chunk = chunk?;
            for byte in chunk {
                if byte != b'\n' {
                    buffer.push(byte);
                    if buffer.len() + event_bytes > 1024 * 1024 { Err(ProviderError::Unavailable("Upstream SSE event exceeds 1 MiB".into()))?; }
                    continue;
                }
                if buffer.last() == Some(&b'\r') { buffer.pop(); }
                let line = String::from_utf8(std::mem::take(&mut buffer)).map_err(|_| ProviderError::Unavailable("Invalid SSE UTF-8".into()))?;
                if line.is_empty() {
                    if !data.is_empty() {
                        let payload = data.join("\n"); data.clear(); event_bytes = 0;
                        if payload == "[DONE]" { yield payload; done = true; break 'stream; }
                        let value: serde_json::Value = serde_json::from_str(&payload).map_err(|_| ProviderError::Unavailable("Invalid SSE JSON".into()))?;
                        if value.get("error").is_some() { Err(ProviderError::Unavailable(payload.clone()))?; }
                        yield payload;
                    }
                } else if let Some(value) = line.strip_prefix("data:") {
                    let value = value.strip_prefix(' ').unwrap_or(value).to_string();
                    event_bytes += value.len() + 1; data.push(value);
                }
            }
        }
        if !done { Err(ProviderError::Unavailable("Upstream stream ended without [DONE]".into()))?; }
    }))
}
