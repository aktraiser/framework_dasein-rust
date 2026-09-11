//! # agentic-tools
//!
//! Built-in tools for the Agentic Framework.
//!
//! ## Available Tools
//!
//! - **Web Search** (Tavily API) - Search the web for current information
//! - **Google Workspace** (via `gog` CLI):
//!   - Gmail: Search and send emails
//!   - Calendar: List upcoming events
//!   - Drive: Search files
//!
//! ## Example
//!
//! ```rust,no_run
//! use dasein_agentic_tools::{ToolRegistry, TavilySearch, GoogleWorkspace};
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut registry = ToolRegistry::new();
//!
//!     // Add Tavily search
//!     if let Ok(key) = std::env::var("TAVILY_API_KEY") {
//!         registry.register(TavilySearch::new(key));
//!     }
//!
//!     // Add Google Workspace tools
//!     if std::env::var("GOG_ACCOUNT").is_ok() {
//!         registry.register(GoogleWorkspace::gmail());
//!         registry.register(GoogleWorkspace::calendar());
//!         registry.register(GoogleWorkspace::drive());
//!     }
//!
//!     // Get tools for LLM
//!     let tools = registry.get_tools();
//!
//!     // Execute a tool call
//!     let result = registry.execute("web_search", &serde_json::json!({
//!         "query": "latest news"
//!     })).await;
//! }
//! ```

mod error;
mod registry;
mod traits;

#[cfg(feature = "search")]
pub mod search;

#[cfg(feature = "google")]
mod google;

pub use error::ToolError;
pub use registry::ToolRegistry;
pub use traits::{ToolExecutor, ToolResult};

#[cfg(feature = "search")]
pub use search::{SearchResult, TavilySearch};

#[cfg(feature = "google")]
pub use google::{CalendarTool, DriveTool, GmailTool, GoogleWorkspace};

// Re-export Tool and ToolCall from agentic-llm for convenience
pub use dasein_agentic_llm::{Tool, ToolCall};
