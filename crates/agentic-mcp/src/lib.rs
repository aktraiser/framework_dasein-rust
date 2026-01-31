//! # agentic-mcp
//!
//! MCP Code Mode implementation for agentic-rs.
//!
//! Instead of calling MCP tools directly (which consumes lots of tokens),
//! agents write code that calls MCP tools. The code runs in our sandbox,
//! and only final results return to the agent context.
//!
//! # The Problem (Traditional MCP)
//!
//! ```text
//! Agent → call tool1 → 10,000 tokens in context
//!       → call tool2 → 10,000 tokens in context
//!       → call tool3 → 10,000 tokens in context
//!
//! Total: 150,000+ tokens per workflow
//! ```
//!
//! # The Solution (Code Mode)
//!
//! ```text
//! Agent writes TypeScript:
//!
//!   import * as gdrive from './servers/google-drive';
//!   import * as salesforce from './servers/salesforce';
//!
//!   const doc = await gdrive.getDocument({ id: 'abc' });
//!   await salesforce.update({ data: doc.content });
//!   console.log('Done');
//!
//! Code executes in sandbox → Only "Done" returns to context
//!
//! Total: 2,000 tokens (98.7% reduction!)
//! ```
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        Agent (Executor)                          │
//! │                              │                                   │
//! │                      writes TypeScript                           │
//! │                              ▼                                   │
//! │              ┌──────────────────────────────┐                   │
//! │              │      Sandbox (Firecracker)    │                   │
//! │              │                              │                   │
//! │              │  ./servers/                  │                   │
//! │              │    ├── context7/             │                   │
//! │              │    │   └── queryDocs.ts      │                   │
//! │              │    ├── github/               │                   │
//! │              │    │   └── createPR.ts       │                   │
//! │              │    └── ...                   │                   │
//! │              │                              │                   │
//! │              │  Code imports and calls      │                   │
//! │              │  these generated modules     │                   │
//! │              └──────────────┬───────────────┘                   │
//! │                             │                                    │
//! └─────────────────────────────┼────────────────────────────────────┘
//!                               │ MCP calls (from inside sandbox)
//!          ┌────────────────────┼────────────────────┐
//!          ▼                    ▼                    ▼
//!   ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
//!   │  Context7    │   │   GitHub     │   │   Postgres   │
//!   └──────────────┘   └──────────────┘   └──────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use agentic_mcp::{MCPConfig, CodeModeRuntime};
//! use agentic_sandbox::FirecrackerSandbox;
//!
//! // 1. Load MCP server configurations
//! let config = MCPConfig::from_json(r#"{
//!     "mcpServers": {
//!         "context7": {
//!             "url": "https://mcp.context7.com/mcp",
//!             "headers": { "CONTEXT7_API_KEY": "..." }
//!         }
//!     }
//! }"#)?;
//!
//! // 2. Create runtime with sandbox
//! let sandbox = FirecrackerSandbox::builder().build();
//! let runtime = CodeModeRuntime::new(sandbox, config).await?;
//!
//! // 3. Agent writes code that uses MCP tools
//! let code = r#"
//!     import { queryDocs } from './servers/context7';
//!
//!     const docs = await queryDocs({
//!         libraryId: '/vercel/next.js',
//!         query: 'app router'
//!     });
//!     console.log(docs.summary);
//! "#;
//!
//! // 4. Execute - only console output returns to agent
//! let result = runtime.execute(code).await?;
//! println!("Agent sees: {}", result.stdout);  // Just the summary
//! ```

pub mod client;
mod error;
pub mod server;

pub use client::{MCPClientPool, MCPConfig, MCPServerConfig, TransportType};
pub use error::MCPError;
pub use server::ValidatorMCPServer;
