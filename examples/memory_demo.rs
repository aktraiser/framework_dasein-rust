//! Memory Demo - Demonstrates Agent Memory and Chat Reducers.
//!
//! This example shows how to use:
//! 1. **MemoryProvider** - Long-term memory for agents
//! 2. **ChatReducer** - Context window management
//!
//! # Run
//!
//! ```bash
//! cargo run --example memory_demo
//! ```

use agentic_core::distributed::graph::agent::{
    // Memory types
    InMemoryProvider, Memory, MemoryCategory, MemoryContext, MemoryProvider,
    NoOpMemoryProvider,
    // Reducer types
    ChatReducer, MessageCountingReducer, NoOpReducer, SlidingWindowReducer,
    TokenCountingReducer,
    // Core types
    AgentId, AgentThread, ChatMessage,
};

fn main() {
    println!("=== Agent Memory & Reducer Demo ===\n");

    demo_memory_context();
    demo_memory_types();
    demo_memory_provider();
    demo_reducers();

    println!("\n=== Demo Complete ===");
}

// ============================================================================
// MEMORY CONTEXT
// ============================================================================

fn demo_memory_context() {
    println!("--- Memory Context ---\n");

    // Create a memory context
    let mut ctx = MemoryContext::new();
    println!("Empty context: {}", ctx.is_empty());

    // Add instructions
    ctx.extra_instructions = Some("Remember that the user prefers concise answers.".into());

    // Add retrieved memories
    ctx.add_memory(Memory::new("User name: Alice").with_importance(0.9));
    ctx.add_memory(Memory::new("Likes Rust programming").with_importance(0.7));

    println!("After adding memories: {}", !ctx.is_empty());
    println!("Retrieved memories: {}", ctx.retrieved_memories.len());

    // Generate context string
    if let Some(context_str) = ctx.to_context_string() {
        println!("\nContext string:\n{}\n", context_str);
    }
}

// ============================================================================
// MEMORY TYPES
// ============================================================================

fn demo_memory_types() {
    println!("--- Memory Types ---\n");

    // Create different types of memories
    let preference = Memory::new("User prefers dark mode")
        .with_category(MemoryCategory::UserPreference)
        .with_importance(0.8);

    let fact = Memory::new("User's birthday is March 15")
        .with_category(MemoryCategory::Fact)
        .with_importance(0.6);

    let context = Memory::new("Working on a Rust project")
        .with_category(MemoryCategory::Context)
        .with_importance(0.5)
        .with_metadata("project", serde_json::json!("agentic-rs"));

    println!("Created memories:");
    println!(
        "  - {:?}: {} (importance: {})",
        preference.category, preference.content, preference.importance
    );
    println!(
        "  - {:?}: {} (importance: {})",
        fact.category, fact.content, fact.importance
    );
    println!(
        "  - {:?}: {} (importance: {}, metadata: {:?})",
        context.category, context.content, context.importance, context.metadata
    );
    println!();
}

// ============================================================================
// MEMORY PROVIDER
// ============================================================================

fn demo_memory_provider() {
    println!("--- Memory Provider ---\n");

    // Create an in-memory provider
    let runtime = tokio::runtime::Runtime::new().unwrap();

    runtime.block_on(async {
        let provider = InMemoryProvider::new().with_max_memories(10);
        let agent_id = AgentId::new("assistant");
        let user_id = "alice";

        // Store some memories
        println!("Storing memories...");
        provider
            .store_memory(
                &agent_id,
                user_id,
                Memory::new("Alice likes concise responses")
                    .with_category(MemoryCategory::UserPreference)
                    .with_importance(0.9),
            )
            .await
            .unwrap();

        provider
            .store_memory(
                &agent_id,
                user_id,
                Memory::new("Alice is a software engineer")
                    .with_category(MemoryCategory::Fact)
                    .with_importance(0.7),
            )
            .await
            .unwrap();

        provider
            .store_memory(
                &agent_id,
                user_id,
                Memory::new("Alice prefers Rust")
                    .with_category(MemoryCategory::UserPreference)
                    .with_importance(0.85),
            )
            .await
            .unwrap();

        // Retrieve memories
        let memories = provider.get_memories(&agent_id, user_id).await.unwrap();
        println!("Retrieved {} memories:", memories.len());
        for m in &memories {
            println!("  - [{}] {} ({})", m.category_str(), m.content, m.importance);
        }

        // Test before_invoke
        let thread = AgentThread::new().with_metadata("user_id", serde_json::json!("alice"));

        let mut ctx = MemoryContext::new();
        provider
            .before_invoke(&agent_id, &thread, &mut ctx)
            .await
            .unwrap();

        println!("\nBefore invoke injected {} memories", ctx.retrieved_memories.len());
        if let Some(instructions) = &ctx.extra_instructions {
            println!("Extra instructions:\n{}", instructions);
        }

        // Clear memories
        provider.clear_memories(&agent_id, user_id).await.unwrap();
        let empty = provider.get_memories(&agent_id, user_id).await.unwrap();
        println!("\nAfter clear: {} memories", empty.len());
    });

    // No-op provider
    println!("\nNoOpMemoryProvider: always returns empty");
    let _noop = NoOpMemoryProvider;

    println!();
}

// ============================================================================
// REDUCERS
// ============================================================================

fn demo_reducers() {
    println!("--- Chat Reducers ---\n");

    // Create a conversation with many messages
    let mut messages = vec![ChatMessage::system("You are a helpful assistant.")];

    for i in 0..20 {
        messages.push(ChatMessage::user(format!("Question {}", i)));
        messages.push(ChatMessage::assistant(format!(
            "Answer to question {} - here is some content",
            i
        )));
    }

    println!("Original messages: {} (including 1 system)", messages.len());

    // MessageCountingReducer
    println!("\n1. MessageCountingReducer (max 10 messages):");
    let reducer = MessageCountingReducer::new(10);
    let reduced = reducer.reduce(&messages);
    println!(
        "   Needs reduction: {} | After: {} messages",
        reducer.needs_reduction(&messages),
        reduced.len()
    );
    println!(
        "   First: {} | Last: {}",
        if reduced[0].is_system() {
            "system"
        } else {
            &reduced[0].content
        },
        &reduced.last().unwrap().content
    );

    // TokenCountingReducer
    println!("\n2. TokenCountingReducer (max 500 tokens):");
    let reducer = TokenCountingReducer::new(500);
    let reduced = reducer.reduce(&messages);
    println!(
        "   Needs reduction: {} | After: {} messages",
        reducer.needs_reduction(&messages),
        reduced.len()
    );

    // SlidingWindowReducer
    println!("\n3. SlidingWindowReducer (window=6):");

    // Mark one message as important
    let mut messages_with_important = messages.clone();
    messages_with_important[5] = messages[5]
        .clone()
        .with_metadata(serde_json::json!({"important": true}));

    let reducer = SlidingWindowReducer::new(6);
    let reduced = reducer.reduce(&messages_with_important);
    println!(
        "   Needs reduction: {} | After: {} messages",
        reducer.needs_reduction(&messages_with_important),
        reduced.len()
    );

    // Check important message preserved
    let has_important = reduced.iter().any(|m| {
        m.metadata
            .as_ref()
            .and_then(|meta| meta.get("important"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    });
    println!("   Important message preserved: {}", has_important);

    // NoOpReducer
    println!("\n4. NoOpReducer:");
    let reducer = NoOpReducer;
    let reduced = reducer.reduce(&messages);
    println!(
        "   Needs reduction: {} | After: {} messages (unchanged)",
        reducer.needs_reduction(&messages),
        reduced.len()
    );

    println!();
}
