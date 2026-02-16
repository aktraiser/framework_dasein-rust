//! Patterns LLM Test - Real workflow execution with ACTUAL LLM calls.
//!
//! This example tests the orchestration patterns with real LLM executors,
//! not mock transformers.
//!
//! # Environment Variables Required
//!
//! - `GEMINI_API_KEY` or `OPENAI_API_KEY`: For LLM calls
//! - `MODEL`: Optional, defaults to "gemini-2.0-flash"
//!
//! # Run
//!
//! ```bash
//! GEMINI_API_KEY=xxx cargo run --example patterns_llm_test
//! ```

use dasein_agentic_core::distributed::graph::{
    Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind, ExecutorRegistry, Workflow,
    WorkflowConfig,
};
use dasein_agentic_core::distributed::Executor as LLMExecutor;
use dasein_agentic_core::patterns::{ConcurrentBuilder, SequentialBuilder};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

// ============================================================================
// MESSAGE TYPE
// ============================================================================

/// Message flowing through the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContentMessage {
    /// The content being processed
    content: String,
    /// Role: "draft", "reviewed", "polished"
    stage: String,
    /// Token usage tracking
    tokens_used: u32,
}

impl ContentMessage {
    fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            stage: "initial".into(),
            tokens_used: 0,
        }
    }

    fn with_stage(mut self, stage: impl Into<String>) -> Self {
        self.stage = stage.into();
        self
    }

    fn add_tokens(mut self, tokens: u32) -> Self {
        self.tokens_used += tokens;
        self
    }
}

// ============================================================================
// LLM EXECUTOR WRAPPERS
// ============================================================================

/// Writer executor - generates initial content using LLM.
struct WriterExecutor {
    id: ExecutorId,
    llm: Arc<Mutex<LLMExecutor>>,
}

impl WriterExecutor {
    fn new(llm: Arc<Mutex<LLMExecutor>>) -> Self {
        Self {
            id: ExecutorId::new("writer"),
            llm,
        }
    }
}

#[async_trait]
impl Executor for WriterExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        let msg: ContentMessage = serde_json::from_value(input)
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        println!("\n[Writer] Generating content for: '{}'", msg.content);

        let system = "You are a creative writer. Write a short paragraph (2-3 sentences) about the given topic. Be concise and engaging.";

        let llm = self.llm.lock().await;
        let result = llm
            .execute(system, &msg.content)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        let output = ContentMessage::new(&result.content)
            .with_stage("draft")
            .add_tokens(result.tokens_used);

        println!("[Writer] Generated {} chars, {} tokens", result.content.len(), output.tokens_used);

        ctx.send_message(serde_json::to_value(&output).unwrap())
            .await?;
        ctx.yield_output(format!("Writer: {} tokens", output.tokens_used))
            .await?;

        Ok(())
    }
}

/// Reviewer executor - reviews and provides feedback using LLM.
struct ReviewerExecutor {
    id: ExecutorId,
    llm: Arc<Mutex<LLMExecutor>>,
}

impl ReviewerExecutor {
    fn new(llm: Arc<Mutex<LLMExecutor>>) -> Self {
        Self {
            id: ExecutorId::new("reviewer"),
            llm,
        }
    }
}

#[async_trait]
impl Executor for ReviewerExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        let msg: ContentMessage = serde_json::from_value(input)
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        println!("\n[Reviewer] Reviewing content ({} chars)...", msg.content.len());

        let system = "You are an editor. Review and improve the following text. Keep it concise (2-3 sentences). Focus on clarity and flow.";

        let llm = self.llm.lock().await;
        let result = llm
            .execute(system, &msg.content)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        let output = ContentMessage::new(&result.content)
            .with_stage("reviewed")
            .add_tokens(msg.tokens_used + result.tokens_used);

        println!("[Reviewer] Improved content, total {} tokens", output.tokens_used);

        ctx.send_message(serde_json::to_value(&output).unwrap())
            .await?;
        ctx.yield_output(format!("Reviewer: {} tokens", result.tokens_used))
            .await?;

        Ok(())
    }
}

/// Polisher executor - final polish using LLM.
struct PolisherExecutor {
    id: ExecutorId,
    llm: Arc<Mutex<LLMExecutor>>,
}

impl PolisherExecutor {
    fn new(llm: Arc<Mutex<LLMExecutor>>) -> Self {
        Self {
            id: ExecutorId::new("polisher"),
            llm,
        }
    }
}

#[async_trait]
impl Executor for PolisherExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        let msg: ContentMessage = serde_json::from_value(input)
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        println!("\n[Polisher] Final polish ({} chars)...", msg.content.len());

        let system = "You are a copy editor. Make final polish to the text: fix any grammar, improve word choice. Keep length similar. Output only the polished text.";

        let llm = self.llm.lock().await;
        let result = llm
            .execute(system, &msg.content)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        let output = ContentMessage::new(&result.content)
            .with_stage("polished")
            .add_tokens(msg.tokens_used + result.tokens_used);

        println!("[Polisher] Final content ready, total {} tokens", output.tokens_used);

        ctx.send_message(serde_json::to_value(&output).unwrap())
            .await?;
        ctx.yield_output(format!(
            "FINAL OUTPUT:\n{}\n\nTotal tokens: {}",
            result.content, output.tokens_used
        ))
        .await?;

        Ok(())
    }
}

/// Researcher executor - researches a topic using LLM.
struct ResearcherExecutor {
    id: ExecutorId,
    focus: String,
    llm: Arc<Mutex<LLMExecutor>>,
}

impl ResearcherExecutor {
    fn new(id: impl Into<String>, focus: impl Into<String>, llm: Arc<Mutex<LLMExecutor>>) -> Self {
        Self {
            id: ExecutorId::new(id),
            focus: focus.into(),
            llm,
        }
    }
}

#[async_trait]
impl Executor for ResearcherExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        let msg: ContentMessage = serde_json::from_value(input)
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        println!("\n[{}] Researching {} aspect...", self.id, self.focus);

        let system = format!(
            "You are a researcher focusing on {}. Provide 1-2 key insights about the topic. Be concise.",
            self.focus
        );

        let llm = self.llm.lock().await;
        let result = llm
            .execute(&system, &msg.content)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        let output = ContentMessage::new(&result.content)
            .with_stage(format!("research-{}", self.focus))
            .add_tokens(result.tokens_used);

        println!("[{}] Found insights, {} tokens", self.id, output.tokens_used);

        ctx.send_message(serde_json::to_value(&output).unwrap())
            .await?;
        ctx.yield_output(format!("{}: {}", self.id, result.content.lines().next().unwrap_or("")))
            .await?;

        Ok(())
    }
}

/// Splitter executor for fan-out.
struct SplitterExecutor {
    id: ExecutorId,
}

impl SplitterExecutor {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: ExecutorId::new(id),
        }
    }
}

#[async_trait]
impl Executor for SplitterExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        println!("\n[Splitter] Broadcasting to researchers...");
        ctx.send_message(input).await?;
        ctx.yield_output("Split to researchers".into()).await?;
        Ok(())
    }
}

/// Aggregator executor for fan-in.
struct AggregatorExecutor {
    id: ExecutorId,
    #[allow(dead_code)] // Reserved for future LLM-based aggregation
    llm: Arc<Mutex<LLMExecutor>>,
}

impl AggregatorExecutor {
    fn new(id: impl Into<String>, llm: Arc<Mutex<LLMExecutor>>) -> Self {
        Self {
            id: ExecutorId::new(id),
            llm,
        }
    }
}

#[async_trait]
impl Executor for AggregatorExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        let msg: ContentMessage = serde_json::from_value(input)
            .map_err(|e| ExecutorError::new(self.id.clone(), e.to_string()))?;

        println!("\n[Aggregator] Received research: {} chars from {}", msg.content.len(), msg.stage);

        // For simplicity, just yield each research result
        ctx.yield_output(format!("Research ({}): {}", msg.stage, msg.content))
            .await?;

        Ok(())
    }
}

// ============================================================================
// TESTS
// ============================================================================

/// Test 1: Sequential Content Pipeline with Real LLM
async fn test_sequential_llm() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!(" TEST 1: SEQUENTIAL LLM PIPELINE");
    println!(" Writer → Reviewer → Polisher (3 LLM calls)");
    println!("{}", "=".repeat(70));

    // Setup LLM
    let model = std::env::var("MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let llm = LLMExecutor::new("pattern-llm", "supervisor")
        .llm_gemini(&model)
        .build();
    let llm = Arc::new(Mutex::new(llm));

    println!("Using model: {}\n", model);

    // Build workflow using SequentialBuilder
    let definition = SequentialBuilder::<serde_json::Value>::new("content-pipeline")
        .name("Content Pipeline with LLM")
        .add_participant("writer")
        .add_participant("reviewer")
        .add_participant("polisher")
        .build()?;

    println!(
        "Workflow: {} executors, {} edges",
        definition.executors.len(),
        definition.edges.all().len()
    );

    // Create LLM executors
    let writer = WriterExecutor::new(llm.clone());
    let reviewer = ReviewerExecutor::new(llm.clone());
    let polisher = PolisherExecutor::new(llm.clone());

    // Register
    let mut registry: ExecutorRegistry<serde_json::Value, String> = ExecutorRegistry::new();
    registry.register(writer);
    registry.register(reviewer);
    registry.register(polisher);

    // Run
    let config = WorkflowConfig::new().with_max_supersteps(10);
    let workflow = Workflow::with_config(definition, registry, config);

    let input = ContentMessage::new("The future of artificial intelligence");
    let input_json = serde_json::to_value(&input)?;

    println!("\nTopic: '{}'\n", input.content);
    println!("{}", "-".repeat(70));

    let start = Instant::now();
    let result = workflow.run(input_json).await?;
    let elapsed = start.elapsed();

    println!("{}", "-".repeat(70));
    println!("\nResult:");
    println!("  Success: {}", result.success);
    println!("  Supersteps: {}", result.superstep_count);
    println!("  Duration: {:?}", elapsed);
    println!("  Outputs: {} items", result.outputs.len());

    // Show final output
    if let Some(last) = result.outputs.last() {
        println!("\n{}", last);
    }

    assert!(result.success, "Workflow should succeed");
    assert!(result.superstep_count >= 3, "Should have at least 3 supersteps");

    println!("\n✅ Sequential LLM test PASSED\n");
    Ok(())
}

/// Test 2: Concurrent Research with Real LLM
async fn test_concurrent_llm() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!(" TEST 2: CONCURRENT LLM RESEARCH");
    println!(" Splitter → [Technical, Business, Social] → Aggregator");
    println!("{}", "=".repeat(70));

    // Setup LLM
    let model = std::env::var("MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let llm = LLMExecutor::new("pattern-llm", "supervisor")
        .llm_gemini(&model)
        .build();
    let llm = Arc::new(Mutex::new(llm));

    println!("Using model: {}\n", model);

    // Build workflow using ConcurrentBuilder
    let definition = ConcurrentBuilder::<serde_json::Value>::new("research-pipeline")
        .name("Concurrent Research Pipeline")
        .add_participant("researcher-technical")
        .add_participant("researcher-business")
        .add_participant("researcher-social")
        .build()?;

    println!(
        "Workflow: {} executors, {} edges",
        definition.executors.len(),
        definition.edges.all().len()
    );

    // Create executors
    let splitter = SplitterExecutor::new("research-pipeline-splitter");
    let tech = ResearcherExecutor::new("researcher-technical", "technical aspects", llm.clone());
    let biz = ResearcherExecutor::new("researcher-business", "business impact", llm.clone());
    let social = ResearcherExecutor::new("researcher-social", "social implications", llm.clone());
    let aggregator = AggregatorExecutor::new("research-pipeline-aggregator", llm.clone());

    // Register
    let mut registry: ExecutorRegistry<serde_json::Value, String> = ExecutorRegistry::new();
    registry.register(splitter);
    registry.register(tech);
    registry.register(biz);
    registry.register(social);
    registry.register(aggregator);

    // Run
    let config = WorkflowConfig::new().with_max_supersteps(10);
    let workflow = Workflow::with_config(definition, registry, config);

    let input = ContentMessage::new("quantum computing");
    let input_json = serde_json::to_value(&input)?;

    println!("\nTopic: '{}'\n", input.content);
    println!("{}", "-".repeat(70));

    let start = Instant::now();
    let result = workflow.run(input_json).await?;
    let elapsed = start.elapsed();

    println!("{}", "-".repeat(70));
    println!("\nResult:");
    println!("  Success: {}", result.success);
    println!("  Supersteps: {}", result.superstep_count);
    println!("  Duration: {:?}", elapsed);
    println!("  Outputs: {} items", result.outputs.len());

    // Show research outputs
    println!("\nResearch Results:");
    for (i, output) in result.outputs.iter().enumerate() {
        if output.len() > 100 {
            println!("  [{}] {}...", i + 1, &output[..100]);
        } else {
            println!("  [{}] {}", i + 1, output);
        }
    }

    assert!(result.success, "Workflow should succeed");

    println!("\n✅ Concurrent LLM test PASSED\n");
    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║      PATTERNS LLM TEST - Real LLM Workflow Execution           ║");
    println!("╚════════════════════════════════════════════════════════════════╝");

    // Check for API key
    if std::env::var("GEMINI_API_KEY").is_err() && std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("\n❌ Error: No API key found!");
        eprintln!("   Set GEMINI_API_KEY or OPENAI_API_KEY environment variable.");
        std::process::exit(1);
    }

    let mut passed = 0;
    let mut failed = 0;

    // Test 1: Sequential LLM
    match test_sequential_llm().await {
        Ok(_) => passed += 1,
        Err(e) => {
            println!("❌ Sequential LLM test FAILED: {}", e);
            failed += 1;
        }
    }

    // Test 2: Concurrent LLM
    match test_concurrent_llm().await {
        Ok(_) => passed += 1,
        Err(e) => {
            println!("❌ Concurrent LLM test FAILED: {}", e);
            failed += 1;
        }
    }

    // Summary
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║                         TEST SUMMARY                           ║");
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!(
        "║  Passed: {}                                                     ║",
        passed
    );
    println!(
        "║  Failed: {}                                                     ║",
        failed
    );
    println!("╚════════════════════════════════════════════════════════════════╝");

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}
