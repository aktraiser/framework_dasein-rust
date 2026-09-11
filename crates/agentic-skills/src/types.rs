//! Core types for the skill system.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Type of skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillType {
    /// Pure prompt instructions (e.g., how to analyze code)
    Prompt,
    /// Executable code/function
    Code,
    /// Multi-step workflow with defined stages
    Workflow,
    /// Wrapper around an MCP tool
    McpTool,
}

impl Default for SkillType {
    fn default() -> Self {
        Self::Prompt
    }
}

/// Execution statistics for a skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillStats {
    /// Total number of executions
    pub execution_count: u64,
    /// Number of successful executions
    pub success_count: u64,
    /// Number of failed executions
    pub failure_count: u64,
    /// Average execution time in milliseconds
    pub avg_execution_ms: f64,
    /// Last execution timestamp
    pub last_executed: Option<DateTime<Utc>>,
}

impl SkillStats {
    /// Calculate success rate (0.0 to 1.0).
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.execution_count == 0 {
            1.0 // No data = assume good
        } else {
            self.success_count as f64 / self.execution_count as f64
        }
    }

    /// Record a successful execution.
    pub fn record_success(&mut self, execution_ms: u64) {
        self.execution_count += 1;
        self.success_count += 1;
        self.update_avg_time(execution_ms);
        self.last_executed = Some(Utc::now());
    }

    /// Record a failed execution.
    pub fn record_failure(&mut self, execution_ms: u64) {
        self.execution_count += 1;
        self.failure_count += 1;
        self.update_avg_time(execution_ms);
        self.last_executed = Some(Utc::now());
    }

    fn update_avg_time(&mut self, execution_ms: u64) {
        let n = self.execution_count as f64;
        self.avg_execution_ms = ((self.avg_execution_ms * (n - 1.0)) + execution_ms as f64) / n;
    }
}

/// A skill definition.
///
/// Skills are "instruction manuals" for agents - they describe how to perform
/// specific tasks. Skills can be:
/// - Prompt-based (instructions in natural language)
/// - Code-based (executable functions)
/// - Workflow-based (multi-step procedures)
/// - MCP tool wrappers
///
/// # Example
///
/// ```rust
/// use dasein_agentic_skills::{Skill, SkillType};
///
/// let skill = Skill::new("code_review", "Reviews code for quality and issues")
///     .with_type(SkillType::Prompt)
///     .with_instructions("Analyze the provided code for: 1. Bugs...")
///     .with_trigger(["review my code", "code quality check", "find bugs"]);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Unique identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what the skill does
    pub description: String,
    /// Type of skill
    #[serde(rename = "type")]
    pub skill_type: SkillType,
    /// Main instructions/prompt for the skill
    pub instructions: String,
    /// Example trigger phrases (for semantic matching)
    pub triggers: Vec<String>,
    /// Required tools/capabilities (MCP tool names)
    pub required_tools: Vec<String>,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Version string
    pub version: String,
    /// Whether the skill is enabled
    pub enabled: bool,
    /// Execution statistics
    pub stats: SkillStats,
    /// Additional metadata (JSON)
    pub metadata: Value,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl Skill {
    /// Create a new skill with the given name and description.
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.into(),
            skill_type: SkillType::default(),
            instructions: String::new(),
            triggers: Vec::new(),
            required_tools: Vec::new(),
            tags: Vec::new(),
            version: "1.0.0".to_string(),
            enabled: true,
            stats: SkillStats::default(),
            metadata: Value::Null,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the skill type.
    #[must_use]
    pub fn with_type(mut self, skill_type: SkillType) -> Self {
        self.skill_type = skill_type;
        self
    }

    /// Set the instructions/prompt.
    #[must_use]
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = instructions.into();
        self
    }

    /// Add trigger phrases for semantic matching.
    #[must_use]
    pub fn with_trigger(mut self, triggers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.triggers.extend(triggers.into_iter().map(Into::into));
        self
    }

    /// Add required tools.
    #[must_use]
    pub fn with_required_tools(
        mut self,
        tools: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.required_tools
            .extend(tools.into_iter().map(Into::into));
        self
    }

    /// Add tags.
    #[must_use]
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    /// Set the version.
    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Set custom metadata.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Generate text for embedding (combines name, description, triggers).
    #[must_use]
    pub fn embedding_text(&self) -> String {
        let mut parts = vec![self.name.clone(), self.description.clone()];
        parts.extend(self.triggers.clone());
        parts.extend(self.tags.clone());
        parts.join(" | ")
    }

    /// Check if the skill needs improvement (low success rate).
    #[must_use]
    pub fn needs_improvement(&self, threshold: f64) -> bool {
        self.stats.execution_count >= 10 && self.stats.success_rate() < threshold
    }
}

/// Result of a skill match from the router.
#[derive(Debug, Clone)]
pub struct SkillMatch {
    /// The matched skill
    pub skill: Skill,
    /// Similarity score (0.0 to 1.0)
    pub score: f32,
}

impl SkillMatch {
    /// Create a new skill match.
    pub fn new(skill: Skill, score: f32) -> Self {
        Self { skill, score }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_creation() {
        let skill = Skill::new("test_skill", "A test skill")
            .with_type(SkillType::Prompt)
            .with_instructions("Do the thing")
            .with_trigger(["do it", "make it happen"])
            .with_tags(["test", "demo"]);

        assert_eq!(skill.name, "test_skill");
        assert_eq!(skill.skill_type, SkillType::Prompt);
        assert_eq!(skill.triggers.len(), 2);
        assert_eq!(skill.tags.len(), 2);
    }

    #[test]
    fn test_skill_stats() {
        let mut stats = SkillStats::default();

        stats.record_success(100);
        stats.record_success(200);
        stats.record_failure(150);

        assert_eq!(stats.execution_count, 3);
        assert_eq!(stats.success_count, 2);
        assert_eq!(stats.failure_count, 1);
        assert!((stats.success_rate() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_embedding_text() {
        let skill = Skill::new("code_review", "Reviews code quality")
            .with_trigger(["review my code", "check code"])
            .with_tags(["code", "quality"]);

        let text = skill.embedding_text();
        assert!(text.contains("code_review"));
        assert!(text.contains("Reviews code quality"));
        assert!(text.contains("review my code"));
    }
}
