//! MCP Server - Expose validation/audit capabilities as MCP tools.
//!
//! This module provides MCP server functionality that exposes our
//! validation and analysis capabilities to external clients.
//!
//! ## Available Tools
//!
//! - `validate-code` - Compile and test code in sandbox
//! - `analyze-code` - Static analysis without execution
//! - `audit-security` - Security audit of code
//!
//! ## Architecture
//!
//! ```text
//! External Clients (Claude, Cursor, etc.)
//!           │
//!           ▼ MCP Protocol
//!    ┌──────────────────┐
//!    │   MCP Server     │
//!    │  (this module)   │
//!    └────────┬─────────┘
//!             │
//!    ┌────────┴─────────┐
//!    ▼                  ▼
//! SandboxValidator   CodeAnalyzer
//! ```

mod tools;

pub use tools::ValidatorMCPServer;
