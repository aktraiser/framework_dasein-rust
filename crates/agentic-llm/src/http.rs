//! HTTP plumbing shared by the provider adapters: a client with timeouts,
//! errors that carry the provider's message but never a URL or key, retries
//! for calls that can safely be repeated, and stream framing that survives
//! any split of the bytes across network chunks.

use std::time::Duration;

use crate::error::LLMError;

/// Retries after the first attempt, unless an adapter is configured otherwise.
pub(crate) const DEFAULT_MAX_RETRIES: u32 = 2;

/// Longest wait honoured between two attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Largest pending line or event accepted from a stream.
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Silence tolerated from a provider unless an adapter is configured
/// otherwise. Long on purpose: a non-streamed completion sends nothing until
/// it is complete.
pub(crate) const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// Client for provider calls. It gives up when the provider cannot be reached
/// within 10 s or stays silent for `idle_timeout`.
pub(crate) fn client(idle_timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(idle_timeout)
        .build()
        .unwrap_or_else(|error| {
            tracing::error!(%error, "HTTP client setup failed; using defaults without timeouts");
            reqwest::Client::new()
        })
}

/// Describe a transport failure without its URL, which may carry credentials.
pub(crate) fn transport_error(error: reqwest::Error) -> LLMError {
    if error.is_timeout() {
        return LLMError::Timeout;
    }
    LLMError::ConnectionError(error.without_url().to_string())
}

/// Turn an HTTP error status and its body into the matching error, keeping
/// the provider's own explanation when the body has one.
pub(crate) fn status_error(status: reqwest::StatusCode, body: &str) -> LLMError {
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            // `{"error": {"message": ...}}` or `{"error": "..."}`.
            let error = value.get("error")?;
            error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .or_else(|| error.as_str())
        })
        .map_or_else(
            || body.trim().chars().take(300).collect::<String>(),
            str::to_owned,
        );
    let message = if message.is_empty() {
        format!("HTTP {status}")
    } else {
        message
    };
    match status.as_u16() {
        401 | 403 => LLMError::AuthenticationError(message),
        // Gemini answers 400 to a bad key.
        400 if body.contains("API_KEY_INVALID") => LLMError::AuthenticationError(message),
        // A 404 can also be a wrong URL: only call it a missing model when
        // the provider says so.
        404 if message.to_ascii_lowercase().contains("model") => LLMError::ModelNotFound(message),
        429 => LLMError::RateLimitError(message),
        _ => LLMError::ApiError(format!("HTTP {status}: {message}")),
    }
}

/// Send a request, repeating it when that is both useful and safe: the
/// provider could not be reached, limited the rate (429) or was overloaded
/// (503, 529). In those cases it did not process the request. A request that
/// timed out, broke after it was sent, or got another server error may
/// already have been processed and billed, so it is not replayed.
///
/// Returns the response once its status is a success; any other status
/// becomes an error built from the response body.
pub(crate) async fn send(
    build: impl Fn() -> reqwest::RequestBuilder,
    max_retries: u32,
) -> Result<reqwest::Response, LLMError> {
    let mut attempt = 0;
    loop {
        let (error, retry_after) = match build().send().await {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .map(Duration::from_secs);
                let body = response.text().await.unwrap_or_default();
                let error = status_error(status, &body);
                if !matches!(status.as_u16(), 429 | 503 | 529) {
                    return Err(error);
                }
                (error, retry_after)
            }
            Err(error) if error.is_connect() => (transport_error(error), None),
            Err(error) => return Err(transport_error(error)),
        };
        if attempt >= max_retries {
            return Err(error);
        }
        // 0.5 s, 1 s, 2 s..., or what the provider asked for if longer.
        let backoff = Duration::from_millis(500u64.saturating_mul(1 << attempt.min(6)));
        tokio::time::sleep(
            retry_after
                .map_or(backoff, |wait| wait.max(backoff))
                .min(MAX_BACKOFF),
        )
        .await;
        attempt += 1;
    }
}

/// Splits a byte stream into lines. Bytes are decoded only once a line is
/// complete, so a character or a line cut between two chunks is never lost.
#[derive(Default)]
pub(crate) struct LineBuffer {
    pending: Vec<u8>,
    /// Bytes of `pending` already known to hold no newline.
    searched: usize,
}

impl LineBuffer {
    /// Add a chunk and return the lines it completes, without terminators.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, LLMError> {
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        let mut start = 0;
        // Resume where the last search stopped: each byte is looked at once.
        let mut from = self.searched;
        while let Some(offset) = self.pending[from..].iter().position(|byte| *byte == b'\n') {
            let end = from + offset;
            let line = self.pending[start..end]
                .strip_suffix(b"\r")
                .unwrap_or_else(|| &self.pending[start..end]);
            lines.push(String::from_utf8_lossy(line).into_owned());
            start = end + 1;
            from = start;
        }
        self.pending.drain(..start);
        self.searched = self.pending.len();
        if self.pending.len() > MAX_FRAME_BYTES {
            return Err(LLMError::InvalidResponse(format!(
                "stream line longer than {MAX_FRAME_BYTES} bytes"
            )));
        }
        Ok(lines)
    }

    /// The unterminated last line, once the stream has ended.
    pub(crate) fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.pending);
        self.searched = 0;
        (!rest.is_empty()).then(|| String::from_utf8_lossy(&rest).into_owned())
    }
}

/// Extracts the `data` payload of each server-sent event from a byte stream.
#[derive(Default)]
pub(crate) struct SseBuffer {
    lines: LineBuffer,
    data: Vec<String>,
}

impl SseBuffer {
    /// Add a chunk and return the payloads of the events it completes.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, LLMError> {
        let mut events = Vec::new();
        for line in self.lines.push(chunk)? {
            self.line(&line, &mut events);
        }
        Ok(events)
    }

    /// The payload of a last event that was not followed by a blank line.
    pub(crate) fn finish(&mut self) -> Option<String> {
        let mut events = Vec::new();
        if let Some(line) = self.lines.finish() {
            self.line(&line, &mut events);
        }
        self.line("", &mut events);
        events.pop()
    }

    fn line(&mut self, line: &str, events: &mut Vec<String>) {
        if line.is_empty() {
            // A blank line ends the event.
            if !self.data.is_empty() {
                events.push(std::mem::take(&mut self.data).join("\n"));
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            self.data
                .push(value.strip_prefix(' ').unwrap_or(value).to_owned());
        }
        // `event:`, `id:`, `retry:` and comment lines carry nothing we use.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `bytes` cut into pieces of every size from 1 to its length.
    fn every_chunking(bytes: &[u8], mut check: impl FnMut(&mut dyn Iterator<Item = &[u8]>)) {
        for size in 1..=bytes.len() {
            check(&mut bytes.chunks(size));
        }
    }

    #[test]
    fn lines_survive_any_chunking() {
        let input = "héllo wörld\r\n{\"text\":\"日本語\"}\n\nlast".as_bytes();
        every_chunking(input, |chunks| {
            let mut buffer = LineBuffer::default();
            let mut lines = Vec::new();
            for chunk in chunks {
                lines.extend(buffer.push(chunk).unwrap());
            }
            lines.extend(buffer.finish());
            assert_eq!(lines, ["héllo wörld", "{\"text\":\"日本語\"}", "", "last"]);
        });
    }

    #[test]
    fn sse_events_survive_any_chunking() {
        let input = concat!(
            ": comment\n",
            "event: message\ndata: {\"a\":\"é{}\"}\n\n",
            "data: first\r\ndata: second\r\n\r\n",
            "data:unterminated",
        )
        .as_bytes();
        every_chunking(input, |chunks| {
            let mut buffer = SseBuffer::default();
            let mut events = Vec::new();
            for chunk in chunks {
                events.extend(buffer.push(chunk).unwrap());
            }
            events.extend(buffer.finish());
            assert_eq!(events, ["{\"a\":\"é{}\"}", "first\nsecond", "unterminated"]);
        });
    }

    #[test]
    fn status_errors_keep_the_provider_message() {
        use reqwest::StatusCode;
        let nested = r#"{"error":{"message":"Invalid API key","type":"auth"}}"#;
        assert!(matches!(
            status_error(StatusCode::UNAUTHORIZED, nested),
            LLMError::AuthenticationError(message) if message == "Invalid API key"
        ));
        assert!(matches!(
            status_error(StatusCode::NOT_FOUND, r#"{"error":"model 'x' not found"}"#),
            LLMError::ModelNotFound(message) if message == "model 'x' not found"
        ));
        // A wrong URL is not a missing model.
        assert!(matches!(
            status_error(StatusCode::NOT_FOUND, "Not Found"),
            LLMError::ApiError(message) if message.contains("404")
        ));
        // Gemini reports a bad key as a 400.
        let bad_key = r#"{"error":{"message":"API key not valid.","details":[{"reason":"API_KEY_INVALID"}]}}"#;
        assert!(matches!(
            status_error(StatusCode::BAD_REQUEST, bad_key),
            LLMError::AuthenticationError(_)
        ));
        assert!(matches!(
            status_error(StatusCode::TOO_MANY_REQUESTS, "slow down"),
            LLMError::RateLimitError(message) if message == "slow down"
        ));
        assert!(matches!(
            status_error(StatusCode::BAD_GATEWAY, ""),
            LLMError::ApiError(message) if message.contains("502")
        ));
    }
}
