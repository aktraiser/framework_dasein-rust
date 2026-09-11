//! Skill router for intelligent skill selection.
//!
//! The router analyzes user queries and selects the most appropriate skills
//! using semantic similarity matching.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, instrument};

use crate::error::SkillError;
use crate::registry::SkillRegistryTrait;
use crate::types::{Skill, SkillMatch};

/// Configuration for the skill router.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Minimum similarity score for a match (0.0 to 1.0)
    pub min_score: f32,
    /// Maximum number of skills to return
    pub max_results: u64,
    /// Whether to require at least one match
    pub require_match: bool,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            min_score: 0.5,
            max_results: 3,
            require_match: false,
        }
    }
}

impl RouterConfig {
    /// Create a strict config (higher threshold, single result).
    #[must_use]
    pub fn strict() -> Self {
        Self {
            min_score: 0.7,
            max_results: 1,
            require_match: true,
        }
    }

    /// Create a permissive config (lower threshold, more results).
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            min_score: 0.3,
            max_results: 5,
            require_match: false,
        }
    }
}

/// Result of skill routing.
#[derive(Debug)]
pub struct RouteResult {
    /// Query that was routed
    pub query: String,
    /// Best matching skill (if any)
    pub best_match: Option<SkillMatch>,
    /// All matches above threshold
    pub all_matches: Vec<SkillMatch>,
    /// Whether a confident match was found
    pub confident: bool,
}

impl RouteResult {
    /// Get the best skill if confident.
    #[must_use]
    pub fn skill(&self) -> Option<&Skill> {
        if self.confident {
            self.best_match.as_ref().map(|m| &m.skill)
        } else {
            None
        }
    }

    /// Get all matched skills.
    #[must_use]
    pub fn skills(&self) -> Vec<&Skill> {
        self.all_matches.iter().map(|m| &m.skill).collect()
    }
}

/// Router for selecting appropriate skills for user queries.
///
/// The router uses semantic search to find skills that match the user's
/// intent, ranked by relevance score.
///
/// # Example
///
/// ```rust,ignore
/// use dasein_agentic_skills::{SkillRouter, RouterConfig};
///
/// let router = SkillRouter::new(registry, RouterConfig::default());
///
/// // Route a user query
/// let result = router.route("review my code for bugs").await?;
///
/// if let Some(skill) = result.skill() {
///     println!("Using skill: {}", skill.name);
///     println!("Instructions: {}", skill.instructions);
/// }
/// ```
pub struct SkillRouter<R: SkillRegistryTrait> {
    registry: Arc<R>,
    config: RouterConfig,
}

impl<R: SkillRegistryTrait> SkillRouter<R> {
    /// Create a new router with the given registry and config.
    pub fn new(registry: Arc<R>, config: RouterConfig) -> Self {
        Self { registry, config }
    }

    /// Create a router with default config.
    pub fn with_defaults(registry: Arc<R>) -> Self {
        Self::new(registry, RouterConfig::default())
    }

    /// Route a user query to matching skills.
    #[instrument(skip(self), fields(query = %query))]
    pub async fn route(&self, query: &str) -> Result<RouteResult, SkillError> {
        // Search for matching skills
        let matches = self.registry.search(query, self.config.max_results).await?;

        // Filter by minimum score
        let filtered: Vec<SkillMatch> = matches
            .into_iter()
            .filter(|m| m.score >= self.config.min_score)
            .collect();

        let best_match = filtered.first().cloned();
        let confident = best_match
            .as_ref()
            .map_or(false, |m| m.score >= self.config.min_score + 0.2);

        if filtered.is_empty() && self.config.require_match {
            return Err(SkillError::NoMatch(query.to_string()));
        }

        debug!(
            "Routed query to {} skills, confident: {}",
            filtered.len(),
            confident
        );

        Ok(RouteResult {
            query: query.to_string(),
            best_match,
            all_matches: filtered,
            confident,
        })
    }

    /// Route and get the single best skill (or error if none).
    pub async fn route_single(&self, query: &str) -> Result<Skill, SkillError> {
        let result = self.route(query).await?;

        result
            .best_match
            .map(|m| m.skill)
            .ok_or_else(|| SkillError::NoMatch(query.to_string()))
    }

    /// Check if a query would match any skill.
    pub async fn has_match(&self, query: &str) -> Result<bool, SkillError> {
        let result = self.route(query).await?;
        Ok(result.best_match.is_some())
    }

    /// Get multiple skills for a complex query that might need several skills.
    pub async fn route_multi(&self, query: &str, max: usize) -> Result<Vec<Skill>, SkillError> {
        let result = self.route(query).await?;

        Ok(result
            .all_matches
            .into_iter()
            .take(max)
            .map(|m| m.skill)
            .collect())
    }
}

/// Trait for skill routing operations.
#[async_trait]
pub trait SkillRouterTrait: Send + Sync {
    /// Route a query to matching skills.
    async fn route(&self, query: &str) -> Result<RouteResult, SkillError>;

    /// Route and get the single best skill.
    async fn route_single(&self, query: &str) -> Result<Skill, SkillError>;
}

#[async_trait]
impl<R: SkillRegistryTrait> SkillRouterTrait for SkillRouter<R> {
    async fn route(&self, query: &str) -> Result<RouteResult, SkillError> {
        self.route(query).await
    }

    async fn route_single(&self, query: &str) -> Result<Skill, SkillError> {
        self.route_single(query).await
    }
}

type SkillHandler = std::sync::Arc<
    dyn Fn(
            Skill,
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, SkillError>> + Send>,
        > + Send
        + Sync,
>;

/// Resolve a skill and execute it through an explicitly registered handler.
/// The handler is responsible for dispatching the skill type and bounding side effects.
pub struct SkillExecutor<R: SkillRegistryTrait> {
    handler: Option<SkillHandler>,
    router: SkillRouter<R>,
}

impl<R: SkillRegistryTrait> SkillExecutor<R> {
    /// Create a new executor.
    pub fn new(router: SkillRouter<R>) -> Self {
        Self {
            router,
            handler: None,
        }
    }

    /// Configure how a routed skill is actually executed.
    pub fn with_handler<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(Skill, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, SkillError>> + Send + 'static,
    {
        self.handler = Some(std::sync::Arc::new(move |skill, query| {
            Box::pin(handler(skill, query))
        }));
        self
    }

    /// Execute a skill for the given query.
    ///
    /// Returns an error if no handler is configured or if routing/execution fails.
    #[instrument(skip(self), fields(query = %query))]
    pub async fn execute(&self, query: &str) -> Result<ExecutionResult, SkillError> {
        let start = std::time::Instant::now();

        let skill = self.router.route_single(query).await?;

        let handler = self.handler.as_ref().ok_or_else(|| {
            SkillError::ConfigError(
                "No skill execution handler configured; routing only resolves instructions".into(),
            )
        })?;
        let output = handler(skill.clone(), query.to_owned()).await?;
        let execution_time = start.elapsed().as_millis() as u64;

        debug!(
            "Executing skill '{}' (type: {:?})",
            skill.name, skill.skill_type
        );

        Ok(ExecutionResult {
            skill_id: skill.id,
            skill_name: skill.name,
            output,
            execution_ms: execution_time,
            success: true,
        })
    }
}

/// Result of skill execution.
#[derive(Debug)]
pub struct ExecutionResult {
    /// ID of executed skill
    pub skill_id: String,
    /// Name of executed skill
    pub skill_name: String,
    /// Output/result of execution
    pub output: String,
    /// Execution time in milliseconds
    pub execution_ms: u64,
    /// Whether execution was successful
    pub success: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_config() {
        let default = RouterConfig::default();
        assert_eq!(default.max_results, 3);

        let strict = RouterConfig::strict();
        assert_eq!(strict.max_results, 1);
        assert!(strict.min_score > default.min_score);
    }
}
