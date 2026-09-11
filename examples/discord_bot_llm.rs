//! Discord bot with LLM + Tool Calling + Skills (Semantic Routing) + Embeds
//!
//! Demonstrates the hybrid skill routing system:
//! - If Redis + Qdrant available: semantic routing via embeddings
//! - Fallback: LLM classification
//!
//! Run with:
//! ```bash
//! export DISCORD_TOKEN="your_token"
//! export GEMINI_API_KEY="your_key"
//! export OPENAI_API_KEY="your_key"  # For embeddings
//! export TAVILY_API_KEY="your_key"  # https://tavily.com (free)
//! export GOG_ACCOUNT="you@gmail.com"  # For Google tools (optional)
//!
//! # Optional: Start Redis + Qdrant for semantic routing
//! docker run -d -p 6379:6379 redis
//! docker run -d -p 6333:6333 qdrant/qdrant
//!
//! cargo run --example discord_bot_llm --features discord
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use dasein_agentic_chat_platforms::{ChatPlatform, DiscordBot, Embed, OutgoingMessage};
use dasein_agentic_llm::{Embedder, LLMAdapter, LLMMessage, OpenAIEmbedder};
use dasein_agentic_skills::{builtin, RouterConfig, Skill, SkillRegistry, SkillRouter};
use dasein_agentic_storage::{QdrantStore, RedisStore};
use dasein_agentic_tools::{search::SearchResult, GoogleWorkspace, TavilySearch, ToolRegistry};

/// Skill router mode
enum SkillMode {
    /// Semantic routing via Redis + Qdrant + Embeddings
    Semantic(Arc<SkillRouter<SkillRegistry<RedisStore, QdrantStore, OpenAIEmbedder>>>),
    /// Fallback: LLM classification
    LLMClassifier(Vec<Skill>),
}

/// Route query to skill using semantic search or LLM fallback
async fn route_skill(mode: &SkillMode, llm: &dyn LLMAdapter, query: &str) -> Option<Skill> {
    match mode {
        SkillMode::Semantic(router) => match router.route(query).await {
            Ok(result) => {
                if let Some(m) = result.best_match {
                    println!(
                        "  -> Semantic match: {} (score: {:.2})",
                        m.skill.name, m.score
                    );
                    Some(m.skill)
                } else {
                    None
                }
            }
            Err(e) => {
                println!("  -> Routing error: {}", e);
                None
            }
        },
        SkillMode::LLMClassifier(skills) => classify_skill_llm(llm, query, skills).await,
    }
}

/// LLM-based skill classification (fallback)
async fn classify_skill_llm(llm: &dyn LLMAdapter, query: &str, skills: &[Skill]) -> Option<Skill> {
    let skill_list: Vec<_> = skills
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.description))
        .collect();

    let prompt = format!(
        "Which skill should be used for this query? Reply ONLY with the skill name or 'none'.\n\n\
         Skills:\n{}\n\nQuery: {}",
        skill_list.join("\n"),
        query
    );

    let messages = vec![
        LLMMessage::system("You are a classifier. Reply only with the skill name or 'none'."),
        LLMMessage::user(&prompt),
    ];

    match llm.generate(&messages).await {
        Ok(response) => {
            let choice = response.content.trim().to_lowercase();
            skills
                .iter()
                .find(|s| choice.contains(&s.name.to_lowercase()))
                .cloned()
        }
        Err(_) => None,
    }
}

/// Initialize semantic skill routing (Redis + Qdrant + Embeddings)
async fn init_semantic_routing(openai_key: &str) -> Option<SkillMode> {
    // Try to connect to Redis
    let redis = match RedisStore::connect("redis://127.0.0.1:6379").await {
        Ok(r) => {
            println!("Redis: connected");
            Arc::new(r)
        }
        Err(e) => {
            println!("Redis: not available ({})", e);
            return None;
        }
    };

    // Try to connect to Qdrant
    let qdrant = match QdrantStore::new("http://127.0.0.1:6333") {
        Ok(q) => {
            println!("Qdrant: connected");
            Arc::new(q)
        }
        Err(e) => {
            println!("Qdrant: not available ({})", e);
            return None;
        }
    };

    // Create embedder
    let embedder = Arc::new(OpenAIEmbedder::new(openai_key));
    println!("Embedder: OpenAI ({})", embedder.model());

    // Create skill registry
    let registry = match SkillRegistry::new(redis, qdrant, embedder).await {
        Ok(r) => Arc::new(r),
        Err(e) => {
            println!("SkillRegistry: failed to create ({})", e);
            return None;
        }
    };

    // Register built-in skills
    for skill in builtin::all_skills() {
        if let Err(e) = registry.register(skill.clone()).await {
            // Ignore "already exists" errors
            if !e.to_string().contains("already exists") {
                println!("  -> Failed to register skill {}: {}", skill.name, e);
            }
        }
    }

    // Create router
    let router = SkillRouter::new(registry, RouterConfig::default());
    println!("SkillRouter: ready (semantic routing enabled)");

    Some(SkillMode::Semantic(Arc::new(router)))
}

type History = Arc<RwLock<HashMap<String, Vec<LLMMessage>>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let discord_token = std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN required");

    println!("==========================================");
    println!("   Discord Bot + LLM + Tools + Skills");
    println!("   (Hybrid Semantic Routing)");
    println!("==========================================\n");

    // Load built-in skills
    let skills = builtin::all_skills();
    println!(
        "Skills loaded: {:?}",
        skills.iter().map(|s| &s.name).collect::<Vec<_>>()
    );

    // Try to initialize semantic routing (Redis + Qdrant + Embeddings)
    let skill_mode = if let Ok(openai_key) = std::env::var("OPENAI_API_KEY") {
        match init_semantic_routing(&openai_key).await {
            Some(mode) => mode,
            None => {
                println!("\nFalling back to LLM classification...");
                SkillMode::LLMClassifier(skills.clone())
            }
        }
    } else {
        println!("OPENAI_API_KEY not set, using LLM classification for skills");
        SkillMode::LLMClassifier(skills.clone())
    };

    // Build tool registry
    let mut registry = ToolRegistry::new();

    // Add Tavily web search
    if let Ok(key) = std::env::var("TAVILY_API_KEY") {
        println!("Web Search: enabled (Tavily)");
        registry.register(TavilySearch::new(key));
    } else {
        println!("Web Search: disabled (set TAVILY_API_KEY)");
    }

    // Add Google Workspace tools
    if std::env::var("GOG_ACCOUNT").is_ok() {
        println!("Google Workspace: enabled (Gmail, Calendar, Drive)");
        registry.register(GoogleWorkspace::gmail());
        registry.register(GoogleWorkspace::gmail_send());
        registry.register(GoogleWorkspace::calendar());
        registry.register(GoogleWorkspace::drive());
    } else {
        println!("Google Workspace: disabled (set GOG_ACCOUNT)");
    }

    println!("Registered tools: {:?}", registry.tool_names());

    // Setup LLM
    let llm: Arc<Box<dyn LLMAdapter>> =
        Arc::new(if let Ok(key) = std::env::var("GEMINI_API_KEY") {
            println!("LLM: Gemini");
            Box::new(dasein_agentic_llm::GeminiAdapter::new(
                &key,
                "gemini-2.0-flash",
            ))
        } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            println!("LLM: OpenAI");
            Box::new(dasein_agentic_llm::OpenAIAdapter::new(&key, "gpt-4o-mini"))
        } else {
            panic!("Set GEMINI_API_KEY or OPENAI_API_KEY");
        });

    let tools = registry.get_tools();
    let registry = Arc::new(registry);
    let history: History = Arc::new(RwLock::new(HashMap::new()));
    let bot = Arc::new(DiscordBot::new(discord_token));
    bot.start().await?;

    println!("\nBot ready! Talk to it on Discord.");
    println!("Try: 'Search for latest AI news' or 'Check my calendar'\n");

    let mut rx = bot.subscribe();
    while let Ok(msg) = rx.recv().await {
        if msg.content.trim().is_empty() || msg.content.starts_with('!') {
            continue;
        }

        println!(
            "\n[{}] {}",
            msg.username.as_deref().unwrap_or("?"),
            msg.content
        );

        // Route to skill using semantic search or LLM fallback
        let matched_skill = route_skill(&skill_mode, llm.as_ref().as_ref(), &msg.content).await;
        if let Some(ref skill) = matched_skill {
            println!("  -> Skill: {} ({:?})", skill.name, skill.skill_type);
        } else {
            println!("  -> No skill matched");
        }

        // Build system prompt with skill instructions
        let system_prompt = match &matched_skill {
            Some(skill) => format!(
                "Tu es un assistant utile. Tu utilises la skill '{}'.\n\n{}\n\nReponds en francais.",
                skill.name, skill.instructions
            ),
            None => "Tu es un assistant utile. Tu peux utiliser des outils pour chercher sur le web, acceder a Gmail, Calendar et Drive. Reponds en francais.".to_string(),
        };

        // Build conversation
        let mut messages = {
            let mut h = history.write().await;
            let conv = h
                .entry(msg.channel_id.clone())
                .or_insert_with(|| vec![LLMMessage::system(&system_prompt)]);
            // Update system prompt with current skill
            conv[0] = LLMMessage::system(&system_prompt);
            conv.push(LLMMessage::user(&msg.content));
            if conv.len() > 20 {
                conv.remove(1);
            }
            conv.clone()
        };

        // Collect search results for embeds
        let mut search_results: Vec<SearchResult> = Vec::new();

        // Tool loop
        let mut max_iterations = 3;
        let final_response = loop {
            max_iterations -= 1;
            if max_iterations == 0 {
                break "Trop d'iterations de tools".to_string();
            }

            let response = match llm.generate_with_tools(&messages, &tools).await {
                Ok(r) => r,
                Err(e) => break format!("Erreur LLM: {}", e),
            };

            println!(
                "  -> LLM: {} | tools: {}",
                response.content.chars().take(50).collect::<String>(),
                response.tool_calls.len()
            );

            if response.tool_calls.is_empty() {
                break response.content;
            }

            // Execute tools
            let mut tool_results_text = String::new();
            for tc in &response.tool_calls {
                println!("  -> Tool: {}({:?})", tc.name, tc.arguments);

                match registry.execute_call(tc).await {
                    Ok(result) => {
                        println!("  -> OK: {} chars", result.text.len());
                        tool_results_text.push_str(&format!("[{}]: {}\n", tc.name, result.text));

                        // Extract search results for embeds
                        if tc.name == "web_search" {
                            if let Some(data) = &result.data {
                                if let Ok(results) =
                                    serde_json::from_value::<Vec<SearchResult>>(data.clone())
                                {
                                    search_results.extend(results);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("  -> ERROR: {}", e);
                        tool_results_text.push_str(&format!("[{}]: Error: {}\n", tc.name, e));
                    }
                }
            }

            messages.push(LLMMessage::assistant_with_tool_calls(
                &response.content,
                response.tool_calls.clone(),
            ));
            messages.push(LLMMessage::tool_result("tools", &tool_results_text));
        };

        // Save to history
        {
            let mut h = history.write().await;
            if let Some(conv) = h.get_mut(&msg.channel_id) {
                conv.push(LLMMessage::assistant(&final_response));
            }
        }

        // Build response with embeds
        let reply_text = if final_response.len() > 1900 {
            format!("{}...", &final_response[..1900])
        } else {
            final_response
        };

        let mut outgoing = OutgoingMessage::reply(&msg, &reply_text);

        // Add search result embeds (max 3)
        for result in search_results.iter().take(3) {
            let description = if result.content.len() > 200 {
                format!("{}...", &result.content[..200])
            } else {
                result.content.clone()
            };

            let mut embed = Embed::new(&result.title)
                .with_url(&result.url)
                .with_description(description)
                .with_color(0x5865F2)
                .with_footer("via Tavily Search");

            if let Some(img) = &result.image {
                embed = embed.with_thumbnail(img);
            }

            outgoing = outgoing.with_embed(embed);
        }

        let _ = bot.send(outgoing).await;
    }

    Ok(())
}
