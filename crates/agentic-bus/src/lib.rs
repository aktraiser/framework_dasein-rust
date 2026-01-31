//! # agentic-bus
//!
//! Message bus for inter-agent communication using NATS.
//!
//! Provides pub/sub, request/reply, and queue groups for distributed agents.

mod error;
mod nats;
mod patterns;
mod traits;

pub use error::BusError;
pub use nats::NatsBus;
pub use patterns::SubjectPatterns;
pub use traits::{Channel, MessageBus};
