//! Standard subject patterns for NATS.

/// Standard subject patterns for agent communication.
pub struct SubjectPatterns;

impl SubjectPatterns {
    /// Inbox for an agent (receives tasks).
    #[must_use]
    pub fn agent_inbox(agent_id: &str) -> String {
        format!("agents.{agent_id}.inbox")
    }

    /// Outbox for an agent (sends results).
    #[must_use]
    pub fn agent_outbox(agent_id: &str) -> String {
        format!("agents.{agent_id}.outbox")
    }

    /// Events from an agent.
    #[must_use]
    pub fn agent_events(agent_id: &str) -> String {
        format!("agents.{agent_id}.events")
    }

    /// All agent inboxes (wildcard).
    #[must_use]
    pub fn all_agents() -> &'static str {
        "agents.*.inbox"
    }

    /// Orchestrator inbox.
    #[must_use]
    pub fn orchestrator() -> &'static str {
        "orchestrator.inbox"
    }

    /// System-wide events.
    #[must_use]
    pub fn system_events() -> &'static str {
        "system.events"
    }

    /// Task-related subjects.
    #[must_use]
    pub fn tasks() -> &'static str {
        "tasks.*"
    }
}
