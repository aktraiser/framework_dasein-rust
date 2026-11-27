//! JSON-RPC transports for MCP servers: streamable HTTP and stdio.
//!
//! Both transports perform the `initialize` handshake, give every request its
//! own ID, enforce the server's configured timeout and refuse messages larger
//! than [`MAX_MESSAGE_BYTES`].

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, warn};

use super::config::MCPServerConfig;
use crate::MCPError;

/// Protocol revision this client speaks; the server may answer with its own.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Largest single message accepted from a server.
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// JSON-RPC "method not found".
const METHOD_NOT_FOUND: i64 = -32601;

/// A live connection to one MCP server.
pub(super) enum Connection {
    Http(HttpConnection),
    Stdio(StdioConnection),
}

impl Connection {
    /// Open the transport and run the `initialize` handshake.
    pub(super) async fn connect(config: &MCPServerConfig) -> Result<Self, MCPError> {
        let connection = match (&config.url, &config.command) {
            (Some(url), _) => Self::Http(HttpConnection::new(url, config)?),
            (None, Some(command)) => Self::Stdio(StdioConnection::spawn(command, config)?),
            (None, None) => {
                return Err(MCPError::ConnectionFailed(
                    "Server config has neither `url` nor `command`".into(),
                ))
            }
        };
        connection.initialize().await?;
        Ok(connection)
    }

    /// Whether the connection can no longer carry requests and must be
    /// replaced (a stdio server that exited or whose pipe is broken).
    pub(super) fn is_closed(&self) -> bool {
        match self {
            Self::Http(_) => false,
            Self::Stdio(stdio) => stdio.closed.load(Ordering::SeqCst),
        }
    }

    /// Send a request and return the `result` of its response.
    pub(super) async fn request(&self, method: &str, params: Value) -> Result<Value, MCPError> {
        match self.request_once(method, params.clone()).await {
            // The server no longer knows our session (it restarted or expired
            // it). It did not process the request, so a new handshake and one
            // retry are safe.
            Err(MCPError::Http { status: 404, .. }) if self.forget_http_session() => {
                self.initialize().await?;
                self.request_once(method, params).await
            }
            other => other,
        }
    }

    /// Drop the HTTP session, if there is one. Returns whether there was.
    fn forget_http_session(&self) -> bool {
        match self {
            Self::Http(http) => std::mem::take(&mut *http.session()).id.is_some(),
            Self::Stdio(_) => false,
        }
    }

    async fn request_once(&self, method: &str, params: Value) -> Result<Value, MCPError> {
        let response = match self {
            Self::Http(http) => http.request(method, params).await?,
            Self::Stdio(stdio) => stdio.request(method, params).await?,
        };
        if let Some(error) = response.get("error") {
            return Err(MCPError::Rpc {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| error.to_string(), str::to_owned),
                data: error.get("data").filter(|data| !data.is_null()).cloned(),
            });
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| MCPError::TransportError("Response has neither result nor error".into()))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), MCPError> {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match self {
            Self::Http(http) => http.notify(&message).await,
            Self::Stdio(stdio) => stdio.write(&message).await,
        }
    }

    async fn initialize(&self) -> Result<(), MCPError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        match self.request_once("initialize", params).await {
            Ok(result) => {
                if let (Self::Http(http), Some(version)) =
                    (self, result.get("protocolVersion").and_then(Value::as_str))
                {
                    http.set_protocol_version(version);
                }
            }
            // Plain JSON-RPC endpoints that predate the handshake still work:
            // they reject `initialize` as an unknown method, in JSON-RPC or
            // with a bare HTTP status.
            Err(MCPError::Rpc { code, .. })
                if code == METHOD_NOT_FOUND && matches!(self, Self::Http(_)) =>
            {
                return Ok(());
            }
            Err(MCPError::Http {
                status: 400 | 404 | 405,
                ..
            }) => return Ok(()),
            Err(e) => return Err(e),
        }
        self.notify("notifications/initialized", json!({})).await
    }
}

// ============================================================================
// HTTP
// ============================================================================

/// Streamable HTTP transport: one POST per message, answered with JSON or SSE.
pub(super) struct HttpConnection {
    client: reqwest::Client,
    url: String,
    next_id: AtomicU64,
    session: std::sync::Mutex<HttpSession>,
}

#[derive(Default)]
struct HttpSession {
    id: Option<String>,
    protocol_version: Option<String>,
}

impl HttpConnection {
    fn new(url: &str, config: &MCPServerConfig) -> Result<Self, MCPError> {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION};

        let invalid = |what: &str, key: &str| {
            MCPError::ConnectionFailed(format!("Invalid {what} for header `{key}`"))
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        for (key, value) in &config.headers {
            // `API_KEY`-style entries are shorthand for a bearer token.
            if key == "CONTEXT7_API_KEY" || key == "API_KEY" {
                let mut bearer = HeaderValue::from_str(&format!("Bearer {value}"))
                    .map_err(|_| invalid("value", key))?;
                bearer.set_sensitive(true);
                headers.insert(AUTHORIZATION, bearer);
            } else {
                headers.insert(
                    HeaderName::from_bytes(key.as_bytes()).map_err(|_| invalid("name", key))?,
                    HeaderValue::from_str(value).map_err(|_| invalid("value", key))?,
                );
            }
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| MCPError::ConnectionFailed(format!("Failed to create client: {e}")))?;
        Ok(Self {
            client,
            url: url.to_owned(),
            next_id: AtomicU64::new(1),
            session: std::sync::Mutex::default(),
        })
    }

    fn set_protocol_version(&self, version: &str) {
        self.session().protocol_version = Some(version.to_owned());
    }

    fn session(&self) -> std::sync::MutexGuard<'_, HttpSession> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn post(&self, message: &Value) -> Result<reqwest::Response, MCPError> {
        let mut request = self.client.post(&self.url).json(message);
        {
            let session = self.session();
            if let Some(id) = &session.id {
                request = request.header("Mcp-Session-Id", id);
            }
            if let Some(version) = &session.protocol_version {
                request = request.header("MCP-Protocol-Version", version);
            }
        }
        let response = request.send().await.map_err(http_error)?;
        if let Some(id) = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
        {
            self.session().id = Some(id.to_owned());
        }
        Ok(response)
    }

    async fn notify(&self, message: &Value) -> Result<(), MCPError> {
        // Servers answer notifications with 202 and no body; nothing to read.
        self.post(message).await.map(drop)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, MCPError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut response = self.post(&message).await?;

        let status = response.status();
        let is_sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));

        let mut body = Vec::new();
        let mut scan = SseScan::default();
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            if body.len() + chunk.len() > MAX_MESSAGE_BYTES {
                return Err(MCPError::TransportError(format!(
                    "Response larger than {MAX_MESSAGE_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
            // An SSE stream may stay open after our answer: return as soon as
            // the response to this request has arrived.
            if is_sse && status.is_success() {
                if let Some(found) = take_sse_response(&body, &mut scan, id) {
                    return Ok(found);
                }
            }
        }
        if !status.is_success() {
            // A JSON-RPC error body explains more than the bare status.
            if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                if value.get("error").is_some() {
                    return Ok(value);
                }
            }
            return Err(MCPError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).chars().take(200).collect(),
            });
        }
        if is_sse {
            // Stream ended; the last event may lack its trailing blank line.
            body.extend_from_slice(b"\n\n");
            return take_sse_response(&body, &mut scan, id).ok_or_else(|| {
                MCPError::TransportError("Event stream ended without a response".into())
            });
        }
        serde_json::from_slice(&body)
            .map_err(|e| MCPError::SerializationError(format!("Invalid response: {e}")))
    }
}

fn http_error(error: reqwest::Error) -> MCPError {
    if error.is_timeout() {
        return MCPError::Timeout;
    }
    // The URL can carry credentials; keep it out of error messages.
    MCPError::TransportError(format!("HTTP request failed: {}", error.without_url()))
}

/// Progress of the SSE scan over a growing response body.
#[derive(Default)]
struct SseScan {
    /// Start of the first event not yet examined.
    consumed: usize,
    /// How far the search for that event's end has already looked.
    searched: usize,
}

/// Scan the complete SSE events of `body` not examined yet for the JSON-RPC
/// response with the given ID. Each byte is searched once however the body is
/// split across network chunks.
fn take_sse_response(body: &[u8], scan: &mut SseScan, id: u64) -> Option<Value> {
    loop {
        // A separator may straddle the point where the last search stopped.
        let from = scan.searched.saturating_sub(3).max(scan.consumed);
        let Some((at, separator)) = find_blank_line(&body[from..]) else {
            scan.searched = body.len();
            return None;
        };
        let end = from + at;
        let event = String::from_utf8_lossy(&body[scan.consumed..end]);
        scan.consumed = end + separator;
        scan.searched = scan.consumed;

        let data = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|value| value.strip_prefix(' ').unwrap_or(value))
            .collect::<Vec<_>>()
            .join("\n");
        if let Ok(message) = serde_json::from_str::<Value>(&data) {
            let is_response = message.get("result").is_some() || message.get("error").is_some();
            if is_response && message.get("id").and_then(Value::as_u64) == Some(id) {
                return Some(message);
            }
        }
    }
}

/// Find the blank line ending an SSE event (`\n\n` or `\r\n\r\n`): where
/// the event's text stops and how many bytes the terminator takes.
fn find_blank_line(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut search = 0;
    while let Some(offset) = bytes[search..].iter().position(|byte| *byte == b'\n') {
        let newline = search + offset;
        match &bytes[newline + 1..] {
            [b'\n', ..] => return Some((newline, 2)),
            [b'\r', b'\n', ..] => {
                let start = newline - usize::from(newline > 0 && bytes[newline - 1] == b'\r');
                return Some((start, newline + 3 - start));
            }
            _ => search = newline + 1,
        }
    }
    None
}

// ============================================================================
// STDIO
// ============================================================================

type Pending = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// Bound on best-effort writes (cancellations, replies to the server).
const COURTESY_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Stdio transport: the server is a child process exchanging one JSON message
/// per line on stdin/stdout.
pub(super) struct StdioConnection {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    closed: Arc<AtomicBool>,
    next_id: AtomicU64,
    timeout: Duration,
    reader: tokio::task::JoinHandle<()>,
    // Killed when the connection is dropped (`kill_on_drop`).
    _child: Child,
}

impl Drop for StdioConnection {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl StdioConnection {
    fn spawn(program: &str, config: &MCPServerConfig) -> Result<Self, MCPError> {
        let mut command = Command::new(program);
        command
            .args(&config.args)
            // The server gets a minimal environment plus what the config
            // lists, not this process's secrets.
            .env_clear()
            .envs(inherited_environment())
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| MCPError::ConnectionFailed(format!("Failed to start `{program}`: {e}")))?;
        let missing = |stream: &str| MCPError::ConnectionFailed(format!("Server has no {stream}"));
        let stdin = Arc::new(Mutex::new(
            child.stdin.take().ok_or_else(|| missing("stdin"))?,
        ));
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| missing("stdout"))?);
        let stderr = BufReader::new(child.stderr.take().ok_or_else(|| missing("stderr"))?);

        // Stderr is the server's log channel. Drain it so the server never
        // blocks on a full pipe; the task ends when the process exits.
        let name = program.to_owned();
        tokio::spawn(async move {
            let mut stderr = stderr;
            while let Ok(Some(line)) = read_line_limited(&mut stderr, MAX_MESSAGE_BYTES).await {
                debug!(server = %name, "{}", String::from_utf8_lossy(&line));
            }
        });

        let pending = Pending::default();
        let closed = Arc::new(AtomicBool::new(false));
        let reader = tokio::spawn(read_messages(
            stdout,
            stdin.clone(),
            pending.clone(),
            closed.clone(),
        ));
        Ok(Self {
            stdin,
            pending,
            closed,
            next_id: AtomicU64::new(1),
            timeout: Duration::from_millis(config.timeout_ms),
            reader,
            _child: child,
        })
    }

    async fn write(&self, message: &Value) -> Result<(), MCPError> {
        write_message(&self.stdin, &self.closed, message, self.timeout).await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, MCPError> {
        let started = std::time::Instant::now();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        lock(&self.pending).insert(id, sender);

        let closed = || MCPError::TransportError("MCP server closed the connection".into());
        // Checked after registering, so a reader that stops now still fails us.
        if self.closed.load(Ordering::SeqCst) {
            lock(&self.pending).remove(&id);
            return Err(closed());
        }
        // Writing counts against the timeout too: a server that stopped
        // reading its input must not hang the caller.
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.write(&message).await {
            lock(&self.pending).remove(&id);
            return Err(e);
        }
        let remaining = self.timeout.saturating_sub(started.elapsed());
        match tokio::time::timeout(remaining, receiver).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(closed()),
            Err(_) => {
                lock(&self.pending).remove(&id);
                // Best effort, and in the background: let the server stop
                // working on it without making the caller wait for the pipe.
                if !self.closed.load(Ordering::SeqCst) {
                    let cancel = json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/cancelled",
                        "params": { "requestId": id, "reason": "timeout" },
                    });
                    let (stdin, closed) = (self.stdin.clone(), self.closed.clone());
                    tokio::spawn(async move {
                        let _ =
                            write_message(&stdin, &closed, &cancel, COURTESY_WRITE_TIMEOUT).await;
                    });
                }
                Err(MCPError::Timeout)
            }
        }
    }
}

fn lock(pending: &Pending) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Value>>> {
    pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Write one message to the server within `limit`.
///
/// A write that fails or runs out of time may have left half a line in the
/// pipe, after which nothing the server reads makes sense: the connection is
/// marked closed so it gets replaced instead of reused.
async fn write_message(
    stdin: &Mutex<ChildStdin>,
    closed: &AtomicBool,
    message: &Value,
    limit: Duration,
) -> Result<(), MCPError> {
    // Compact JSON never contains a raw newline, so one line is one message.
    let mut line =
        serde_json::to_vec(message).map_err(|e| MCPError::SerializationError(e.to_string()))?;
    line.push(b'\n');
    let sent = async {
        let mut stdin = stdin.lock().await;
        stdin.write_all(&line).await?;
        stdin.flush().await
    };
    let result = match tokio::time::timeout(limit, sent).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(e)) => MCPError::TransportError(format!("Failed to write to MCP server: {e}")),
        Err(_) => MCPError::Timeout,
    };
    closed.store(true, Ordering::SeqCst);
    Err(result)
}

/// Route every message the server prints until its stdout closes.
async fn read_messages<R: AsyncBufRead + Unpin>(
    mut stdout: R,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    closed: Arc<AtomicBool>,
) {
    loop {
        let line = match read_line_limited(&mut stdout, MAX_MESSAGE_BYTES).await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                warn!("Stopped reading MCP server output: {e}");
                break;
            }
        };
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            // Some servers print banners on stdout; they are not protocol.
            debug!("Ignoring non-JSON output from MCP server");
            continue;
        };
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            // A request from the server expects an answer; notifications do not.
            if let Some(id) = message.get("id") {
                let reply = if method == "ping" {
                    json!({ "jsonrpc": "2.0", "id": id, "result": {} })
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": METHOD_NOT_FOUND, "message": "Method not supported" },
                    })
                };
                // Answered from its own task: if reading waited on writing,
                // a server blocked on its output could never drain its input.
                let (stdin, closed) = (stdin.clone(), closed.clone());
                tokio::spawn(async move {
                    let _ = write_message(&stdin, &closed, &reply, COURTESY_WRITE_TIMEOUT).await;
                });
            }
        } else if let Some(id) = message.get("id").and_then(Value::as_u64) {
            if let Some(waiting) = lock(&pending).remove(&id) {
                let _ = waiting.send(message);
            }
        }
    }
    // Dropping the senders fails every request still waiting.
    closed.store(true, Ordering::SeqCst);
    lock(&pending).clear();
}

/// Read one line without its terminator, refusing lines longer than `max`.
/// Returns `None` at end of input.
async fn read_line_limited<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let taken = newline.unwrap_or(available.len());
        if line.len() + taken > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("line longer than {max} bytes"),
            ));
        }
        line.extend_from_slice(&available[..taken]);
        reader.consume(taken + usize::from(newline.is_some()));
        if newline.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(line));
        }
    }
}

/// Variables a child process needs to start and to reach the network:
/// location, locale, proxy and certificate settings. Anything else the server
/// requires, credentials above all, must be listed in its `env` configuration.
fn inherited_environment() -> impl Iterator<Item = (String, String)> {
    const COMMON: &[&str] = &[
        "PATH",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
    ];
    const PLATFORM: &[&str] = if cfg!(windows) {
        &[
            "APPDATA",
            "COMSPEC",
            "HOMEDRIVE",
            "HOMEPATH",
            "LOCALAPPDATA",
            "PATHEXT",
            "PROCESSOR_ARCHITECTURE",
            "SYSTEMDRIVE",
            "SYSTEMROOT",
            "TEMP",
            "USERNAME",
            "USERPROFILE",
        ]
    } else {
        &[
            "HOME",
            "LANG",
            "LC_ALL",
            "LOGNAME",
            "SHELL",
            "TERM",
            "TMPDIR",
            "USER",
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
        ]
    };
    COMMON.iter().chain(PLATFORM).filter_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| ((*name).to_owned(), value))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_response_is_found_among_other_events() {
        let body = concat!(
            ": keep-alive\n\n",
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n",
            "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\r\n\r\n",
        )
        .as_bytes();
        let mut scan = SseScan::default();
        let found = take_sse_response(body, &mut scan, 7).unwrap();
        assert_eq!(found["result"]["ok"], true);
        assert_eq!(scan.consumed, body.len());
    }

    #[test]
    fn sse_response_is_found_whatever_the_chunking() {
        let body = concat!(
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"x\"}\r\n\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"h\u{e9}llo\"}\n\n",
        )
        .as_bytes();
        // Feed the body one more byte at a time: the answer must appear
        // exactly when its event is complete, never before, never missed.
        let mut scan = SseScan::default();
        for received in 0..body.len() {
            assert!(take_sse_response(&body[..received], &mut scan, 1).is_none());
        }
        let found = take_sse_response(body, &mut scan, 1).unwrap();
        assert_eq!(found["result"], "h\u{e9}llo");
    }

    #[tokio::test]
    async fn line_reader_enforces_the_limit() {
        let mut input = &b"short\r\nexactly-10\n"[..];
        assert_eq!(
            read_line_limited(&mut input, 10).await.unwrap().as_deref(),
            Some(&b"short"[..])
        );
        assert_eq!(
            read_line_limited(&mut input, 10).await.unwrap().as_deref(),
            Some(&b"exactly-10"[..])
        );
        assert!(read_line_limited(&mut input, 10).await.unwrap().is_none());

        let mut too_long = &b"this line is too long\n"[..];
        assert!(read_line_limited(&mut too_long, 10).await.is_err());
    }
}
