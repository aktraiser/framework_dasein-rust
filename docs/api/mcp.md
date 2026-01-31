# agentic-mcp

Model Context Protocol (MCP) client for external tool integration.

## Installation

```toml
[dependencies]
agentic-mcp = "0.1"
```

## Status

> **Work in Progress**: This crate is awaiting the stabilization of the `rmcp` crate.

## Planned Features

### Tool Discovery

```rust
use agentic_mcp::McpClient;

// Connect to MCP server
let client = McpClient::connect_stdio("npx", &["-y", "@anthropic/mcp-server-filesystem"]).await?;

// List available tools
let tools = client.list_tools().await?;
for tool in tools {
    println!("Tool: {} - {}", tool.name, tool.description);
}
```

### Tool Invocation

```rust
// Call a tool
let result = client.call_tool("read_file", json!({
    "path": "/path/to/file.txt"
})).await?;

println!("Content: {}", result);
```

### Integration with Agents

```rust
use agentic_core::Agent;
use agentic_mcp::McpToolProvider;

// Create MCP tool provider
let mcp = McpToolProvider::new()
    .add_server("filesystem", "npx", &["-y", "@anthropic/mcp-server-filesystem"])
    .add_server("github", "npx", &["-y", "@anthropic/mcp-server-github"])
    .await?;

// Agent can now use MCP tools
let agent = Agent::new(config, llm, Some(sandbox))
    .with_tools(mcp);
```

## Supported Transports

| Transport | Status |
|-----------|--------|
| Stdio | Planned |
| HTTP/SSE | Planned |
| WebSocket | Planned |

## MCP Servers

Popular MCP servers that will be compatible:

- `@anthropic/mcp-server-filesystem` - File operations
- `@anthropic/mcp-server-github` - GitHub API
- `@anthropic/mcp-server-postgres` - PostgreSQL queries
- `@anthropic/mcp-server-slack` - Slack integration
