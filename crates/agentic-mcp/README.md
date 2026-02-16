# agentic-mcp

MCP (Model Context Protocol) client for the Agentic Framework.

## Features

- **MCP Client**: Connect to MCP servers
- **Tool Execution**: Call MCP tools from agents
- **Code Mode**: Agents write code that calls MCP tools (98.7% token reduction)
- **Async-first**: Built on Tokio

## Installation

```toml
[dependencies]
agentic-mcp = "0.1"
```

## Quick Start

```rust
use agentic_mcp::{MCPClientPool, MCPConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = MCPConfig {
        servers: vec![
            MCPServerConfig {
                name: "sql".into(),
                url: "http://localhost:8080".into(),
                transport: TransportType::Http,
            }
        ],
    };

    let pool = MCPClientPool::new(config).await?;

    let result = pool.call("sql", "execute_sql", json!({
        "sql": "SELECT * FROM users"
    })).await?;

    println!("{:?}", result);

    Ok(())
}
```

## Code Mode

Instead of passing tool schemas to LLMs (expensive), agents write Python code:

```python
# Agent writes this code, executed in sandbox
result = mcp.call("execute_sql", {"sql": "SELECT * FROM users"})
print(result)
```

Benefits:
- 98.7% token reduction vs function calling
- More complex tool compositions
- Agents can use loops, conditions, error handling

## License

MIT
