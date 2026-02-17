//! NATS Persistence Demo - Thread and Memory persistence with NATS KV.
//!
//! This example demonstrates how to persist:
//! 1. **AgentThread** - Conversation history via `NatsThreadStore`
//! 2. **Agent Memory** - Long-term memories via `NatsMemoryProvider`
//!
//! # Prerequisites
//!
//! Start NATS with JetStream:
//! ```bash
//! docker run -d --name nats -p 4222:4222 nats:latest -js
//! ```
//!
//! # Run
//!
//! ```bash
//! cargo run --example nats_persistence_demo
//! ```

use std::sync::Arc;

use dasein_agentic_core::distributed::bus::{NatsClient, NatsConfig};
use dasein_agentic_core::distributed::graph::agent::{
    AgentId, AgentThread, ChatMessage, Memory, MemoryCategory, MemoryContext, MemoryProvider,
    NatsMemoryProvider, NatsThreadStore, ThreadId, ThreadStore,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║         NATS PERSISTENCE DEMO - Thread & Memory                ║");
    println!("╚════════════════════════════════════════════════════════════════╝\n");

    // Connect to NATS
    println!("🔌 Connecting to NATS...");
    let nats = Arc::new(NatsClient::connect(NatsConfig::new("nats://localhost:4222")).await?);
    println!("   ✓ Connected to NATS with JetStream\n");

    // Run demos
    demo_thread_persistence(nats.clone()).await?;
    demo_memory_persistence(nats.clone()).await?;
    demo_persistence_across_reconnect().await?;

    println!("\n════════════════════════════════════════════════════════════════");
    println!("                      DEMO COMPLETE");
    println!("════════════════════════════════════════════════════════════════");

    Ok(())
}

// ============================================================================
// THREAD PERSISTENCE
// ============================================================================

async fn demo_thread_persistence(nats: Arc<NatsClient>) -> Result<(), Box<dyn std::error::Error>> {
    println!("╭────────────────────────────────────────────────────────────────╮");
    println!("│ 1. THREAD PERSISTENCE (NatsThreadStore)                        │");
    println!("╰────────────────────────────────────────────────────────────────╯\n");

    // Create the thread store
    let store = NatsThreadStore::new(nats).await?;
    println!("   ✓ NatsThreadStore initialized\n");

    // Create and populate a thread
    let agent_id = AgentId::new("demo-assistant");
    let mut thread = AgentThread::for_agent(agent_id.clone())
        .with_system_prompt("You are a helpful assistant.")
        .with_metadata("user_id", serde_json::json!("alice"))
        .with_metadata("session", serde_json::json!("session-001"));

    // Simulate a conversation
    thread.add_message(ChatMessage::user("Hello! What is Rust?"));
    thread.add_message(ChatMessage::assistant(
        "Rust is a systems programming language focused on safety, speed, and concurrency.",
    ));
    thread.add_message(ChatMessage::user("What makes it special?"));
    thread.add_message(ChatMessage::assistant(
        "Rust guarantees memory safety without a garbage collector through its ownership system.",
    ));

    let thread_id = thread.id.clone();
    println!("   Thread ID: {}", thread_id);
    println!("   Messages: {}", thread.message_count());
    println!("   Turns: {}", thread.turn_count);

    // Save to NATS
    println!("\n   💾 Saving thread to NATS KV...");
    store.save(&thread).await?;
    println!("   ✓ Thread saved");

    // Load it back
    println!("\n   📖 Loading thread from NATS KV...");
    let loaded = store.load(&thread_id).await?.expect("Thread should exist");
    println!("   ✓ Thread loaded");
    println!(
        "   ✓ Messages match: {}",
        loaded.message_count() == thread.message_count()
    );
    println!("   ✓ System prompt: {:?}", loaded.system_prompt);

    // List threads by agent
    println!("\n   📋 Listing threads for agent...");
    let summaries = store.list_by_agent(&agent_id).await?;
    println!(
        "   ✓ Found {} thread(s) for '{}'",
        summaries.len(),
        agent_id
    );
    for summary in &summaries {
        println!(
            "      - {} ({} messages, {} turns)",
            summary.id, summary.message_count, summary.turn_count
        );
    }

    // Cleanup
    println!("\n   🧹 Cleaning up...");
    store.delete(&thread_id).await?;
    println!("   ✓ Thread deleted\n");

    Ok(())
}

// ============================================================================
// MEMORY PERSISTENCE
// ============================================================================

async fn demo_memory_persistence(nats: Arc<NatsClient>) -> Result<(), Box<dyn std::error::Error>> {
    println!("╭────────────────────────────────────────────────────────────────╮");
    println!("│ 2. MEMORY PERSISTENCE (NatsMemoryProvider)                     │");
    println!("╰────────────────────────────────────────────────────────────────╯\n");

    // Create the memory provider
    let provider = NatsMemoryProvider::new(nats, "DEMO_MEMORIES").await?;
    println!("   ✓ NatsMemoryProvider initialized\n");

    let agent_id = AgentId::new("demo-assistant");
    let user_id = "alice";

    // Store various memories
    println!("   💾 Storing memories...");

    provider
        .store_memory(
            &agent_id,
            user_id,
            Memory::new("User prefers dark mode and minimal UI")
                .with_category(MemoryCategory::UserPreference)
                .with_importance(0.9),
        )
        .await?;

    provider
        .store_memory(
            &agent_id,
            user_id,
            Memory::new("User is a Rust developer working on distributed systems")
                .with_category(MemoryCategory::Fact)
                .with_importance(0.85),
        )
        .await?;

    provider
        .store_memory(
            &agent_id,
            user_id,
            Memory::new("User appreciates concise, technical responses")
                .with_category(MemoryCategory::UserPreference)
                .with_importance(0.8),
        )
        .await?;

    provider
        .store_memory(
            &agent_id,
            user_id,
            Memory::new("User's name is Alice")
                .with_category(MemoryCategory::Fact)
                .with_importance(0.95),
        )
        .await?;

    println!("   ✓ 4 memories stored\n");

    // Retrieve memories
    println!("   📖 Retrieving memories...");
    let memories = provider.get_memories(&agent_id, user_id).await?;
    println!("   ✓ Retrieved {} memories:", memories.len());
    for m in &memories {
        println!(
            "      - [{:?}] {} (importance: {:.2})",
            m.category, m.content, m.importance
        );
    }

    // Test before_invoke (simulates what happens before each LLM call)
    println!("\n   🔄 Simulating before_invoke()...");
    let thread = AgentThread::new().with_metadata("user_id", serde_json::json!("alice"));

    let mut ctx = MemoryContext::new();
    provider.before_invoke(&agent_id, &thread, &mut ctx).await?;

    println!(
        "   ✓ Injected {} memories into context",
        ctx.retrieved_memories.len()
    );
    if let Some(instructions) = &ctx.extra_instructions {
        println!("\n   📝 Extra instructions for LLM:");
        for line in instructions.lines().take(6) {
            println!("      {}", line);
        }
    }

    // Cleanup
    println!("\n   🧹 Cleaning up...");
    provider.clear_memories(&agent_id, user_id).await?;
    println!("   ✓ Memories cleared\n");

    Ok(())
}

// ============================================================================
// PERSISTENCE ACROSS RECONNECT
// ============================================================================

async fn demo_persistence_across_reconnect() -> Result<(), Box<dyn std::error::Error>> {
    println!("╭────────────────────────────────────────────────────────────────╮");
    println!("│ 3. PERSISTENCE ACROSS RECONNECTS                               │");
    println!("╰────────────────────────────────────────────────────────────────╯\n");

    let agent_id = AgentId::new("persist-demo");
    let user_id = "bob";
    let thread_id: ThreadId;

    // First connection: create and save data
    println!("   📤 First connection: Creating data...");
    {
        let nats = Arc::new(NatsClient::connect(NatsConfig::new("nats://localhost:4222")).await?);

        // Save a thread
        let thread_store = NatsThreadStore::new(nats.clone()).await?;
        let mut thread = AgentThread::for_agent(agent_id.clone());
        thread.add_message(ChatMessage::user("This should persist across reconnects!"));
        thread_id = thread.id.clone();
        thread_store.save(&thread).await?;
        println!("      ✓ Thread saved: {}", thread_id);

        // Save a memory
        let memory_provider = NatsMemoryProvider::new(nats, "PERSIST_DEMO_MEMORIES").await?;
        memory_provider
            .store_memory(
                &agent_id,
                user_id,
                Memory::new("Bob prefers verbose explanations"),
            )
            .await?;
        println!("      ✓ Memory saved");
    }
    println!("   📴 Connection closed\n");

    // Second connection: verify data persists
    println!("   📥 Second connection: Verifying data...");
    {
        let nats = Arc::new(NatsClient::connect(NatsConfig::new("nats://localhost:4222")).await?);

        // Check thread
        let thread_store = NatsThreadStore::new(nats.clone()).await?;
        let loaded = thread_store.load(&thread_id).await?;
        if let Some(thread) = loaded {
            println!("      ✓ Thread persisted: {}", thread.id);
            println!(
                "      ✓ Content: \"{}\"",
                thread.messages[0]
                    .content
                    .chars()
                    .take(50)
                    .collect::<String>()
            );
        }

        // Check memory
        let memory_provider = NatsMemoryProvider::new(nats, "PERSIST_DEMO_MEMORIES").await?;
        let memories = memory_provider.get_memories(&agent_id, user_id).await?;
        if !memories.is_empty() {
            println!("      ✓ Memory persisted: \"{}\"", memories[0].content);
        }

        // Cleanup
        thread_store.delete(&thread_id).await?;
        memory_provider.clear_memories(&agent_id, user_id).await?;
    }

    println!("\n   🎉 Data persists across reconnections!\n");

    Ok(())
}
