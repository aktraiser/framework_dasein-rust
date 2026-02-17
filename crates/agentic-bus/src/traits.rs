//! Traits for message bus implementations.

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

use dasein_agentic_core::protocol::Message;

use crate::error::BusError;

/// A communication channel for a specific subject.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Publish a message to this channel.
    async fn publish(&self, message: &Message) -> Result<(), BusError>;

    /// Subscribe to messages on this channel.
    async fn subscribe(&self) -> Result<Pin<Box<dyn Stream<Item = Message> + Send>>, BusError>;

    /// Send a request and wait for response.
    async fn request(&self, message: &Message, timeout_ms: u64) -> Result<Message, BusError>;
}

/// Message bus for inter-agent communication.
#[async_trait]
pub trait MessageBus: Send + Sync {
    /// Disconnect from the bus.
    async fn disconnect(&self) -> Result<(), BusError>;

    /// Create a channel for a subject.
    fn channel(&self, subject: &str) -> Box<dyn Channel>;

    /// Check if connected.
    fn is_connected(&self) -> bool;
}
