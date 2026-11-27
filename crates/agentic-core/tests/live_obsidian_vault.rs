//! A real model answering from an Obsidian vault, through an MCP server
//! written by someone else: the reference filesystem server of the Model
//! Context Protocol project, started with `npx`.
//!
//! Calls the real Gemini API and downloads the server from npm on its first
//! run: ignored unless asked for, and skipped unless `GEMINI_API_KEY` and
//! `LIVE_GEMINI_MODEL` are set and `npx` is installed.
//!
//! ```bash
//! LIVE_GEMINI_MODEL=gemini-flash-latest cargo test -p dasein-agentic-core \
//!   --features mcp --test live_obsidian_vault -- --include-ignored --nocapture
//! ```
#![cfg(feature = "mcp")]

use dasein_agentic_core::distributed::graph::agent::{Agent, AgentThread, ChatAgent, ChatMessage};
use dasein_agentic_llm::GeminiAdapter;
use dasein_agentic_mcp::client::{MCPClientPool, MCPConfig, MCPServerConfig};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The version the test was written against.
const SERVER: &str = "@modelcontextprotocol/server-filesystem@2.4.1";

/// Every file under `directory`, with its content.
fn snapshot(directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(next).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    files
}

#[tokio::test]
#[ignore = "calls the real Gemini API and downloads an MCP server from npm"]
async fn real_model_answers_from_an_obsidian_vault() {
    let (Ok(key), Ok(model)) = (
        std::env::var("GEMINI_API_KEY"),
        std::env::var("LIVE_GEMINI_MODEL"),
    ) else {
        eprintln!("skipped: GEMINI_API_KEY or LIVE_GEMINI_MODEL is not set");
        return;
    };
    let has_npx = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join("npx").is_file())
    });
    if !has_npx {
        eprintln!("skipped: npx is not installed");
        return;
    }
    let vault = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/vault")
        .canonicalize()
        .unwrap();
    let before = snapshot(&vault);

    // A stdio server does not inherit this process's environment: `npx`
    // is given what it needs to find Node and its cache, and nothing else.
    let mut server = MCPServerConfig::stdio(
        "npx",
        vec![
            "-y".into(),
            SERVER.into(),
            vault.to_string_lossy().into_owned(),
        ],
    )
    .timeout(120_000);
    for variable in ["PATH", "HOME"] {
        if let Ok(value) = std::env::var(variable) {
            server = server.env_var(variable, value);
        }
    }
    let pool = MCPClientPool::new(MCPConfig::new().add_server("vault", server))
        .await
        .unwrap();
    let errors = pool.connection_errors().await;
    assert!(errors.is_empty(), "could not start the server: {errors:?}");

    // The server can also write, move and edit files. The agent is only
    // offered what reads.
    let reads = |name: &str, _round: usize| {
        ["read", "list", "search", "directory_tree", "get_file_info"]
            .iter()
            .any(|allowed| name.contains(allowed))
            && !name.contains("write")
    };
    let agent = ChatAgent::new("vault", GeminiAdapter::new(key, model))
        .with_system_prompt(
            "You answer questions about the user's notes. The notes are Markdown files \
             in an Obsidian vault; [[Name]] links to the note called Name.md, which may \
             be in another folder. Look things up with your tools before answering.",
        )
        .with_mcp_tools(Arc::new(pool))
        .await
        .unwrap()
        .with_tool_filter(reads);
    let response = agent
        .run(
            vec![ChatMessage::user(
                "Qui est responsable du projet Atelier Kestrel, quel est son numéro de poste, \
                 et quel est le code de la porte de l'atelier ?",
            )],
            &mut AgentThread::new(),
        )
        .await
        .unwrap();
    println!("answer: {}", response.content);

    // Three facts from three notes in three folders, reached through links.
    for fact in ["Tanguy", "4417", "HIBOU-7302"] {
        assert!(
            response.content.contains(fact),
            "{fact} is missing from: {}",
            response.content
        );
    }
    // And the vault is as it was.
    assert_eq!(snapshot(&vault), before, "the vault was modified");
}
