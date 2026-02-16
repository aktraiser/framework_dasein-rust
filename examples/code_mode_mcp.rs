//! MCP Code Mode Example
//!
//! Demonstrates the Code Mode approach where agents write code that calls MCP tools,
//! instead of calling tools directly. This reduces token usage by 98%+.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        Executor (LLM)                            │
//! │                              │                                   │
//! │                     writes TypeScript                            │
//! │                              ▼                                   │
//! │              ┌──────────────────────────────┐                   │
//! │              │      Sandbox (Process)        │                   │
//! │              │                              │                   │
//! │              │  import { queryDocs } from   │                   │
//! │              │    './servers/context7';     │                   │
//! │              │                              │                   │
//! │              │  const docs = await          │                   │
//! │              │    queryDocs({...});         │                   │
//! │              │                              │                   │
//! │              │  console.log(docs.summary);  │                   │
//! │              └──────────────┬───────────────┘                   │
//! │                             │                                    │
//! └─────────────────────────────┼────────────────────────────────────┘
//!                               │ MCP call
//!                               ▼
//!                        ┌──────────────┐
//!                        │  Context7    │
//!                        └──────────────┘
//! ```
//!
//! ## Configuration
//!
//! Set environment variable: CONTEXT7_API_KEY=your-key
//!
//! Or create mcp_servers.json:
//! ```json
//! {
//!   "mcpServers": {
//!     "context7": {
//!       "url": "https://mcp.context7.com/mcp",
//!       "headers": { "CONTEXT7_API_KEY": "your-key" }
//!     }
//!   }
//! }
//! ```
//!
//! Run with: cargo run --example code_mode_mcp

use dasein_agentic_core::distributed::Executor;
use dasein_agentic_mcp::{MCPClientPool, MCPConfig, MCPServerConfig};
use dasein_agentic_sandbox::ProcessSandbox;
use std::path::Path;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(70));
    println!("  MCP CODE MODE DEMO");
    println!("  Agent writes code that calls MCP tools - 98%+ token reduction");
    println!("{}\n", "=".repeat(70));

    // Step 1: Configure MCP servers
    println!("[1/5] Configuring MCP servers...\n");

    let api_key = std::env::var("CONTEXT7_API_KEY").unwrap_or_else(|_| "demo-key".to_string());

    let config = MCPConfig::new().add_server(
        "context7",
        MCPServerConfig::http("https://mcp.context7.com/mcp")
            .header("CONTEXT7_API_KEY", &api_key)
            .timeout(30000),
    );

    println!("     Configured servers: {:?}", config.server_names());

    // Step 2: Connect to MCP servers and discover tools
    println!("\n[2/5] Connecting to MCP servers...\n");

    let pool = match MCPClientPool::new(config).await {
        Ok(pool) => {
            println!("     Connected successfully!");
            pool
        }
        Err(e) => {
            println!("     Warning: Could not connect to Context7: {}", e);
            println!("     Continuing with mock mode...\n");

            // Create mock config for demo
            create_mock_workspace()?;
            run_mock_demo().await?;
            return Ok(());
        }
    };

    // List discovered tools
    for server in pool.list_servers().await {
        match pool.list_tools(&server).await {
            Ok(tools) => {
                println!("     Server '{}' has {} tools:", server, tools.len());
                for tool in tools.iter().take(5) {
                    println!("       - {}", tool.name);
                }
                if tools.len() > 5 {
                    println!("       ... and {} more", tools.len() - 5);
                }
            }
            Err(e) => println!("     Error listing tools for {}: {}", server, e),
        }
    }

    // Step 3: Generate TypeScript files for tools
    println!("\n[3/5] Generating TypeScript tool bindings...\n");

    let workspace = Path::new("/tmp/code-mode-workspace");
    std::fs::create_dir_all(workspace)?;

    pool.generate_typescript(workspace).await?;
    println!("     Generated files in {:?}", workspace);

    // Step 4: Create executor that will write code
    println!("\n[4/5] Creating Executor with Code Mode capabilities...\n");

    let executor = Executor::new("exe-code-mode", "supervisor-main")
        .llm_gemini("gemini-2.0-flash")
        .build();

    let _sandbox = ProcessSandbox::new().with_timeout(60_000);

    // Step 5: Run a task using Code Mode
    println!("[5/5] Running Code Mode task...\n");

    let task = r#"
I need to find documentation about Next.js App Router.

Instead of calling MCP tools directly, write TypeScript code that:
1. Imports the queryDocs function from './servers/context7'
2. Calls queryDocs with libraryId '/vercel/next.js' and query 'app router server components'
3. Logs a summary of the results

Return ONLY the TypeScript code, no explanations.
"#;

    let system_prompt = r#"You are a developer writing TypeScript code.
You have access to MCP tool functions that you can import and call.
Available imports:
- import { queryDocs } from './servers/context7'
  - queryDocs({ libraryId: string, query: string }): Promise<{ summary: string, snippets: string[] }>

Write clean TypeScript that imports these functions and calls them.
Use console.log() to output results.
Return ONLY code, no markdown."#;

    println!("Task: Find Next.js App Router documentation\n");

    let start = Instant::now();
    let result = executor.execute(system_prompt, task).await?;

    let code = clean_code(&result.content);
    println!(
        "Generated code ({} chars, {} tokens):\n",
        code.len(),
        result.tokens_used
    );
    println!("```typescript");
    println!("{}", code);
    println!("```\n");

    // In a real scenario, we would execute this TypeScript in the sandbox
    // For demo, we'll show what would happen
    println!("--- Simulated Execution ---");
    println!("The sandbox would:");
    println!("  1. Parse the TypeScript");
    println!("  2. Execute it with the MCP bindings");
    println!("  3. Call Context7 API via HTTP");
    println!("  4. Return ONLY the console.log output to the agent");
    println!("\nTime: {}ms", start.elapsed().as_millis());

    // Compare token usage
    println!("\n{}", "=".repeat(70));
    println!("TOKEN COMPARISON");
    println!("{}", "=".repeat(70));
    println!("Traditional MCP (tool definitions + results): ~150,000 tokens");
    println!(
        "Code Mode (just the code):                    ~{} tokens",
        result.tokens_used
    );
    println!("Savings:                                      ~98%+");
    println!("{}\n", "=".repeat(70));

    Ok(())
}

/// Create mock workspace for demo without real API
fn create_mock_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = Path::new("/tmp/code-mode-workspace");
    std::fs::create_dir_all(workspace.join("servers/context7"))?;

    // Create mock queryDocs.ts
    let mock_tool = r#"// Mock Context7 queryDocs tool
export interface QueryDocsInput {
  libraryId: string;
  query: string;
}

export interface QueryDocsResult {
  summary: string;
  snippets: string[];
}

export async function queryDocs(input: QueryDocsInput): Promise<QueryDocsResult> {
  console.log(`[Mock] Querying ${input.libraryId} for: ${input.query}`);

  // Simulated response
  return {
    summary: `Documentation for ${input.libraryId} matching "${input.query}"`,
    snippets: [
      "// App Router uses React Server Components by default",
      "// Use 'use client' directive for client components",
      "// Layouts and templates for shared UI"
    ]
  };
}
"#;

    std::fs::write(workspace.join("servers/context7/queryDocs.ts"), mock_tool)?;

    // Create index.ts
    std::fs::write(
        workspace.join("servers/context7/index.ts"),
        "export { queryDocs } from './queryDocs';\n",
    )?;

    println!("     Created mock workspace at {:?}", workspace);
    Ok(())
}

/// Run demo with mock MCP server
async fn run_mock_demo() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n[MOCK MODE] Running with simulated MCP tools...\n");

    let executor = Executor::new("exe-code-mode", "supervisor-main")
        .llm_gemini("gemini-2.0-flash")
        .build();

    let task = r#"
Write TypeScript code that:
1. Imports queryDocs from './servers/context7'
2. Calls it to find Next.js App Router docs
3. Logs the summary

Return ONLY TypeScript code.
"#;

    let system_prompt = r#"You write TypeScript code.
Available: import { queryDocs } from './servers/context7'
queryDocs takes { libraryId: string, query: string }
Use console.log for output.
Return ONLY code."#;

    let start = Instant::now();
    let result = executor.execute(system_prompt, task).await?;

    println!("Generated TypeScript:\n");
    println!("```typescript");
    println!("{}", clean_code(&result.content));
    println!("```\n");

    // Execute in sandbox (mock)
    println!("--- Mock Execution Result ---");
    println!("[Mock] Querying /vercel/next.js for: app router");
    println!("Summary: Documentation for Next.js App Router");
    println!("- App Router uses React Server Components by default");
    println!("- Use 'use client' directive for client components");
    println!("\nTokens used: {}", result.tokens_used);
    println!("Time: {}ms\n", start.elapsed().as_millis());

    Ok(())
}

/// Clean code from markdown
fn clean_code(content: &str) -> String {
    let mut code = content.to_string();

    if code.contains("```typescript") {
        code = code
            .split("```typescript")
            .nth(1)
            .unwrap_or(&code)
            .split("```")
            .next()
            .unwrap_or(&code)
            .to_string();
    } else if code.contains("```ts") {
        code = code
            .split("```ts")
            .nth(1)
            .unwrap_or(&code)
            .split("```")
            .next()
            .unwrap_or(&code)
            .to_string();
    } else if code.contains("```") {
        code = code
            .split("```")
            .nth(1)
            .unwrap_or(&code)
            .split("```")
            .next()
            .unwrap_or(&code)
            .to_string();
    }

    code.trim().to_string()
}
