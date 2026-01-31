//! Protocol definitions for inter-agent communication.

mod message;

pub use message::{
    AgentRef, ControlSettings, Message, MessageContent, MessageType, Metadata, RoutingInfo,
};
