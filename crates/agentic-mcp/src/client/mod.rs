//! MCP Client - Consume external MCP servers.
//!
//! Allows agents (Executor, Validator, Supervisor) to use tools from external
//! MCP servers like Context7, GitHub, Postgres, etc.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Agent (Executor/Validator)                    │
//! │                              │                                   │
//! │                              ▼                                   │
//! │              ┌──────────────────────────────┐                   │
//! │              │       MCPClientPool          │                   │
//! │              │   (shared, thread-safe)      │                   │
//! │              └──────────────┬───────────────┘                   │
//! │                             │                                    │
//! └─────────────────────────────┼────────────────────────────────────┘
//!                               │
//!          ┌────────────────────┼────────────────────┐
//!          ▼                    ▼                    ▼
//!   ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
//!   │  Context7    │   │   GitHub     │   │   Custom     │
//!   │  (HTTP/SSE)  │   │   (stdio)    │   │   Server     │
//!   └──────────────┘   └──────────────┘   └──────────────┘
//! ```
//!
//! # Configuration
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "context7": {
//!       "url": "https://mcp.context7.com/mcp",
//!       "headers": { "CONTEXT7_API_KEY": "..." }
//!     },
//!     "github": {
//!       "command": "npx",
//!       "args": ["-y", "@modelcontextprotocol/server-github"],
//!       "env": { "GITHUB_TOKEN": "..." }
//!     }
//!   }
//! }
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_mcp::client::{MCPClientPool, MCPConfig};
//!
//! // Load config
//! let config = MCPConfig::from_file("mcp_servers.json")?;
//!
//! // Create shared pool
//! let pool = MCPClientPool::new(config).await?;
//!
//! // Use from any agent
//! let tools = pool.list_tools("context7").await?;
//! let result = pool.call_tool("context7", "query-docs", json!({
//!     "libraryId": "/vercel/next.js",
//!     "query": "how to use app router"
//! })).await?;
//! ```

mod config;
mod pool;

pub use config::{MCPConfig, MCPServerConfig, TransportType};
pub use pool::{ContentItem, MCPClientPool, ToolDefinition, ToolResult};
