//! Message types for the protocol.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{ResultPayload, TaskPayload};

/// Reference to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRef {
    /// Agent ID
    pub agent_id: String,
    /// Optional agent name
    #[serde(default)]
    pub agent_name: Option<String>,
}

impl AgentRef {
    /// Create a new agent reference.
    #[must_use]
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            agent_name: None,
        }
    }

    /// Create with name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.agent_name = Some(name.into());
        self
    }
}

/// Message metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    /// Unique message ID
    pub id: String,
    /// Creation timestamp
    pub timestamp: DateTime<Utc>,
    /// Protocol version
    pub version: String,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            version: "1.0".to_string(),
        }
    }
}

/// Routing information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingInfo {
    /// Sender
    pub from: AgentRef,
    /// Recipient
    pub to: AgentRef,
    /// Correlation ID for request/response
    #[serde(default)]
    pub correlation_id: Option<String>,
}

/// Type of message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    /// Task request
    Task,
    /// Task result
    Result,
    /// Event notification
    Event,
    /// Error
    Error,
}

/// Message content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    /// Task message
    Task {
        /// The task payload
        payload: TaskPayload,
    },
    /// Result message
    Result {
        /// The result payload
        payload: ResultPayload,
    },
    /// Event message
    Event {
        /// Type of event
        event_type: String,
        /// Event data
        data: serde_json::Value,
    },
    /// Error message
    Error {
        /// Error code
        code: String,
        /// Error message
        message: String,
        /// Whether the error is recoverable
        recoverable: bool,
    },
}

/// Control settings for message handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlSettings {
    /// Priority (higher = more important)
    pub priority: i32,
    /// Time-to-live in milliseconds
    pub ttl_ms: u64,
    /// Task ID
    pub task_id: String,
    /// Parent task ID (for sub-tasks)
    #[serde(default)]
    pub parent_task_id: Option<String>,
}

impl Default for ControlSettings {
    fn default() -> Self {
        Self {
            priority: 0,
            ttl_ms: 60000,
            task_id: Uuid::new_v4().to_string(),
            parent_task_id: None,
        }
    }
}

/// A complete message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Metadata
    pub metadata: Metadata,
    /// Routing info
    pub routing: RoutingInfo,
    /// Content
    pub content: MessageContent,
    /// Control settings
    pub control: ControlSettings,
}

impl Message {
    /// Create a task message.
    #[must_use]
    pub fn task(from: AgentRef, to: AgentRef, payload: TaskPayload) -> Self {
        Self {
            metadata: Metadata::default(),
            routing: RoutingInfo {
                from,
                to,
                correlation_id: None,
            },
            content: MessageContent::Task { payload },
            control: ControlSettings::default(),
        }
    }

    /// Create a result message in response to another message.
    #[must_use]
    pub fn result(request: &Message, payload: ResultPayload) -> Self {
        Self {
            metadata: Metadata::default(),
            routing: RoutingInfo {
                from: request.routing.to.clone(),
                to: request.routing.from.clone(),
                correlation_id: Some(request.metadata.id.clone()),
            },
            content: MessageContent::Result { payload },
            control: ControlSettings {
                task_id: request.control.task_id.clone(),
                ..Default::default()
            },
        }
    }

    /// Get the message type.
    #[must_use]
    pub fn message_type(&self) -> MessageType {
        match &self.content {
            MessageContent::Task { .. } => MessageType::Task,
            MessageContent::Result { .. } => MessageType::Result,
            MessageContent::Event { .. } => MessageType::Event,
            MessageContent::Error { .. } => MessageType::Error,
        }
    }
}
