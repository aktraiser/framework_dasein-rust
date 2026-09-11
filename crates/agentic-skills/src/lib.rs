//! # agentic-skills
//!
//! Skills registry and routing for AI agents.
//!
//! Skills are "instruction manuals" for agents - they describe how to perform
//! specific tasks. The skill system provides:
//!
//! - **Skill Types**: Prompt, Code, Workflow, McpTool
//! - **Registry**: Storage in Redis (data) + Qdrant (semantic search)
//! - **Router**: Semantic matching of user queries to skills
//! - **Self-Improvement**: Track success rates for skill evolution
//!
//! ## Architecture
//!
//! ```text
//! User Query
//!     │
//!     ▼
//! ┌─────────────┐     ┌─────────────┐
//! │ SkillRouter │────▶│ SkillRegistry│
//! └─────────────┘     └─────────────┘
//!                           │
//!              ┌────────────┴────────────┐
//!              ▼                         ▼
//!        ┌──────────┐             ┌──────────┐
//!        │  Redis   │             │  Qdrant  │
//!        │ (data)   │             │(vectors) │
//!        └──────────┘             └──────────┘
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use dasein_agentic_skills::{Skill, SkillType, SkillRegistry, SkillRouter, RouterConfig};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create registry with storage backends
//!     let registry = Arc::new(
//!         SkillRegistry::new(state_store, vector_store, embedder).await?
//!     );
//!
//!     // Register a skill
//!     let skill = Skill::new("code_review", "Reviews code for quality and issues")
//!         .with_type(SkillType::Prompt)
//!         .with_instructions("Analyze the code for: bugs, performance, security...")
//!         .with_trigger(["review my code", "check for bugs"]);
//!
//!     registry.register(skill).await?;
//!
//!     // Create router
//!     let router = SkillRouter::new(registry.clone(), RouterConfig::default());
//!
//!     // Route a user query
//!     let result = router.route("can you review this code?").await?;
//!
//!     if let Some(skill) = result.skill() {
//!         println!("Using skill: {}", skill.name);
//!         println!("Instructions: {}", skill.instructions);
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod builtin;
mod error;
mod registry;
mod router;
mod types;

pub use error::SkillError;
pub use registry::{SkillRegistry, SkillRegistryTrait};
pub use router::{
    ExecutionResult, RouteResult, RouterConfig, SkillExecutor, SkillRouter, SkillRouterTrait,
};
pub use types::{Skill, SkillMatch, SkillStats, SkillType};
