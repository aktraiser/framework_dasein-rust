//! Google Workspace tools (via `gog` CLI).
//!
//! These tools require the `gog` CLI to be installed and configured:
//! ```bash
//! brew install steipete/tap/gogcli
//! gog auth credentials /path/to/client_secret.json
//! gog auth add you@gmail.com --services gmail,calendar,drive
//! export GOG_ACCOUNT=you@gmail.com
//! ```

use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;
use tracing::debug;

use crate::error::ToolError;
use crate::traits::{ToolExecutor, ToolResult};
use dasein_agentic_llm::Tool;

/// Factory for creating Google Workspace tools.
pub struct GoogleWorkspace;

impl GoogleWorkspace {
    /// Create a Gmail search tool.
    pub fn gmail() -> GmailTool {
        GmailTool::new()
    }

    /// Create a Gmail send tool.
    pub fn gmail_send() -> GmailSendTool {
        GmailSendTool::new()
    }

    /// Create a Calendar events tool.
    pub fn calendar() -> CalendarTool {
        CalendarTool::new()
    }

    /// Create a Drive search tool.
    pub fn drive() -> DriveTool {
        DriveTool::new()
    }

    /// Check if gog CLI is available and configured.
    pub async fn is_available() -> bool {
        std::env::var("GOG_ACCOUNT").is_ok()
            && Command::new("gog")
                .arg("--version")
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false)
    }
}

// ============================================================================
// GMAIL SEARCH
// ============================================================================

/// Gmail search tool.
pub struct GmailTool {
    max_results: usize,
}

impl GmailTool {
    /// Create a new Gmail search tool.
    pub fn new() -> Self {
        Self { max_results: 10 }
    }

    /// Set the maximum number of results.
    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }
}

impl Default for GmailTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for GmailTool {
    fn name(&self) -> &str {
        "gmail_search"
    }

    fn definition(&self) -> Tool {
        Tool::new("gmail_search", "Search and list emails in Gmail. Use empty query or 'newer_than:1d' for recent emails.")
            .with_string_param(
                "query",
                "Gmail search query. IMPORTANT: Do NOT use accented characters (use 'remi' not 'rémi'). Examples: '' (empty for recent), 'newer_than:1d', 'is:unread', 'from:user@domain.com', 'from:domain.com', 'subject:meeting'. Prefer email domains over names (e.g., 'from:yunik.ai').",
            )
            .with_optional_param("max", "integer", "Maximum number of results (default 10)")
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult, ToolError> {
        let query_raw = arguments["query"].as_str().unwrap_or("");
        // Use a sensible default if query is empty or invalid
        let query = if query_raw.is_empty() || query_raw == "is:latest" {
            "newer_than:1d"
        } else {
            query_raw
        };
        let max = arguments["max"].as_u64().unwrap_or(self.max_results as u64) as usize;

        // Get account from env
        let account = std::env::var("GOG_ACCOUNT")
            .map_err(|_| ToolError::ConfigError("GOG_ACCOUNT not set".to_string()))?;

        debug!(query = %query, max = %max, account = %account, "Searching Gmail");

        let output = Command::new("gog")
            .args([
                "gmail",
                "search",
                query,
                "--account",
                &account,
                "--max",
                &max.to_string(),
                "--json",
            ])
            .output()
            .await?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            debug!(stderr = %stderr, "gog stderr");
        }

        if !output.status.success() {
            return Err(ToolError::CommandFailed(format!(
                "gog failed (exit {}): {}",
                output.status, stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        debug!(stdout_len = %stdout.len(), "gog stdout");

        // Parse JSON and format for LLM
        // gog returns { "threads": [...], "nextPageToken": ... }
        if let Ok(data) = serde_json::from_str::<Value>(&stdout) {
            let mut text = String::new();

            // gog uses "threads" array
            let threads = data["threads"].as_array();
            if let Some(arr) = threads {
                for email in arr.iter().take(max) {
                    let from = email["from"].as_str().unwrap_or("?");
                    let subject = email["subject"].as_str().unwrap_or("(no subject)");
                    let date = email["date"].as_str().unwrap_or("");
                    let labels = email["labels"]
                        .as_array()
                        .map(|l| {
                            l.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();

                    text.push_str(&format!(
                        "- From: {}\n  Subject: {}\n  Date: {}\n  Labels: {}\n\n",
                        from, subject, date, labels
                    ));
                }
            }

            if text.is_empty() {
                text = "No emails found.".to_string();
            }

            Ok(ToolResult::ok_with_data(text, data))
        } else {
            Ok(ToolResult::ok(stdout.to_string()))
        }
    }
}

// ============================================================================
// GMAIL SEND
// ============================================================================

/// Gmail send tool.
pub struct GmailSendTool;

impl GmailSendTool {
    /// Create a new Gmail send tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for GmailSendTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for GmailSendTool {
    fn name(&self) -> &str {
        "gmail_send"
    }

    fn definition(&self) -> Tool {
        Tool::new("gmail_send", "Send an email via Gmail")
            .with_string_param("to", "Recipient email address")
            .with_string_param("subject", "Email subject")
            .with_string_param("body", "Email body text")
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult, ToolError> {
        let to = arguments["to"]
            .as_str()
            .ok_or_else(|| ToolError::MissingArgument("to".to_string()))?;
        let subject = arguments["subject"]
            .as_str()
            .ok_or_else(|| ToolError::MissingArgument("subject".to_string()))?;
        let body = arguments["body"]
            .as_str()
            .ok_or_else(|| ToolError::MissingArgument("body".to_string()))?;

        // Get account from env
        let account = std::env::var("GOG_ACCOUNT")
            .map_err(|_| ToolError::ConfigError("GOG_ACCOUNT not set".to_string()))?;

        debug!(to = %to, subject = %subject, account = %account, "Sending Gmail");

        let output = Command::new("gog")
            .args([
                "gmail",
                "send",
                "--account",
                &account,
                "--to",
                to,
                "--subject",
                subject,
                "--body",
                body,
            ])
            .output()
            .await?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            debug!(stderr = %stderr, "gog stderr");
        }

        if !output.status.success() {
            return Err(ToolError::CommandFailed(format!(
                "gog failed (exit {}): {}",
                output.status, stderr
            )));
        }

        Ok(ToolResult::ok(format!(
            "Email sent to {} with subject: {}",
            to, subject
        )))
    }
}

// ============================================================================
// CALENDAR
// ============================================================================

/// Google Calendar events tool.
pub struct CalendarTool {
    calendar_id: String,
}

impl CalendarTool {
    /// Create a new Calendar tool for the primary calendar.
    pub fn new() -> Self {
        Self {
            calendar_id: "primary".to_string(),
        }
    }

    /// Use a specific calendar ID.
    pub fn with_calendar(mut self, calendar_id: impl Into<String>) -> Self {
        self.calendar_id = calendar_id.into();
        self
    }
}

impl Default for CalendarTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for CalendarTool {
    fn name(&self) -> &str {
        "calendar_events"
    }

    fn definition(&self) -> Tool {
        Tool::new("calendar_events", "Get upcoming calendar events").with_optional_param(
            "days",
            "integer",
            "Number of days to look ahead (default 7)",
        )
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult, ToolError> {
        let days = arguments["days"].as_i64().unwrap_or(7);

        let now = chrono::Utc::now();
        let from = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let to = (now + chrono::Duration::days(days))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        // Get account from env
        let account = std::env::var("GOG_ACCOUNT")
            .map_err(|_| ToolError::ConfigError("GOG_ACCOUNT not set".to_string()))?;

        debug!(calendar = %self.calendar_id, days = %days, account = %account, "Fetching calendar events");

        let output = Command::new("gog")
            .args([
                "calendar",
                "events",
                &self.calendar_id,
                "--account",
                &account,
                "--from",
                &from,
                "--to",
                &to,
                "--json",
            ])
            .output()
            .await?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            debug!(stderr = %stderr, "gog stderr");
        }

        if !output.status.success() {
            return Err(ToolError::CommandFailed(format!(
                "gog failed (exit {}): {}",
                output.status, stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        debug!(stdout_len = %stdout.len(), "gog stdout");

        if let Ok(events) = serde_json::from_str::<Value>(&stdout) {
            let mut text = String::new();

            if let Some(arr) = events.as_array() {
                for event in arr {
                    let summary = event["summary"].as_str().unwrap_or("(no title)");
                    let start = event["start"]["dateTime"]
                        .as_str()
                        .or_else(|| event["start"]["date"].as_str())
                        .unwrap_or("?");
                    let location = event["location"].as_str().unwrap_or("");

                    text.push_str(&format!("- {}\n  When: {}\n", summary, start));
                    if !location.is_empty() {
                        text.push_str(&format!("  Location: {}\n", location));
                    }
                    text.push('\n');
                }
            }

            if text.is_empty() {
                text = "No events found.".to_string();
            }

            Ok(ToolResult::ok_with_data(text, events))
        } else {
            Ok(ToolResult::ok(stdout.to_string()))
        }
    }
}

// ============================================================================
// DRIVE
// ============================================================================

/// Google Drive search tool.
pub struct DriveTool {
    max_results: usize,
}

impl DriveTool {
    /// Create a new Drive search tool.
    pub fn new() -> Self {
        Self { max_results: 10 }
    }

    /// Set the maximum number of results.
    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }
}

impl Default for DriveTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for DriveTool {
    fn name(&self) -> &str {
        "drive_search"
    }

    fn definition(&self) -> Tool {
        Tool::new("drive_search", "Search files in Google Drive")
            .with_string_param("query", "Search query for file names or content")
            .with_optional_param("max", "integer", "Maximum number of results (default 10)")
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult, ToolError> {
        let query = arguments["query"]
            .as_str()
            .ok_or_else(|| ToolError::MissingArgument("query".to_string()))?;
        let max = arguments["max"].as_u64().unwrap_or(self.max_results as u64) as usize;

        // Get account from env
        let account = std::env::var("GOG_ACCOUNT")
            .map_err(|_| ToolError::ConfigError("GOG_ACCOUNT not set".to_string()))?;

        debug!(query = %query, max = %max, account = %account, "Searching Drive");

        let output = Command::new("gog")
            .args([
                "drive",
                "search",
                query,
                "--account",
                &account,
                "--max",
                &max.to_string(),
                "--json",
            ])
            .output()
            .await?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            debug!(stderr = %stderr, "gog stderr");
        }

        if !output.status.success() {
            return Err(ToolError::CommandFailed(format!(
                "gog failed (exit {}): {}",
                output.status, stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        debug!(stdout_len = %stdout.len(), "gog stdout");

        if let Ok(files) = serde_json::from_str::<Value>(&stdout) {
            let mut text = String::new();

            if let Some(arr) = files.as_array() {
                for file in arr.iter().take(max) {
                    let name = file["name"].as_str().unwrap_or("?");
                    let mime = file["mimeType"].as_str().unwrap_or("");
                    let modified = file["modifiedTime"].as_str().unwrap_or("");
                    let link = file["webViewLink"].as_str().unwrap_or("");

                    text.push_str(&format!(
                        "- {} ({})\n  Modified: {}\n  Link: {}\n\n",
                        name, mime, modified, link
                    ));
                }
            }

            if text.is_empty() {
                text = "No files found.".to_string();
            }

            Ok(ToolResult::ok_with_data(text, files))
        } else {
            Ok(ToolResult::ok(stdout.to_string()))
        }
    }
}
