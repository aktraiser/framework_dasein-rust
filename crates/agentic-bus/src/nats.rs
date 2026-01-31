//! NATS message bus implementation.

use async_nats::Client;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use tracing::{debug, info};

use agentic_core::protocol::Message;

use crate::{
    error::BusError,
    traits::{Channel, MessageBus},
};

/// NATS-based message bus.
pub struct NatsBus {
    client: Client,
    url: String,
}

impl NatsBus {
    /// Connect to a NATS server.
    ///
    /// # Errors
    ///
    /// Returns an error if connection fails.
    pub async fn connect(url: &str) -> Result<Self, BusError> {
        info!(url = %url, "Connecting to NATS");

        let client = async_nats::connect(url)
            .await
            .map_err(|e| BusError::ConnectionFailed(e.to_string()))?;

        info!("Connected to NATS");

        Ok(Self {
            client,
            url: url.to_string(),
        })
    }

    /// Get the connection URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

#[async_trait]
impl MessageBus for NatsBus {
    async fn disconnect(&self) -> Result<(), BusError> {
        info!("Disconnecting from NATS");
        self.client
            .drain()
            .await
            .map_err(|e| BusError::DisconnectFailed(e.to_string()))
    }

    fn channel(&self, subject: &str) -> Box<dyn Channel> {
        Box::new(NatsChannel {
            client: self.client.clone(),
            subject: subject.to_string(),
        })
    }

    fn is_connected(&self) -> bool {
        self.client.connection_state() == async_nats::connection::State::Connected
    }
}

/// A NATS channel for a specific subject.
pub struct NatsChannel {
    client: Client,
    subject: String,
}

#[async_trait]
impl Channel for NatsChannel {
    async fn publish(&self, message: &Message) -> Result<(), BusError> {
        let payload =
            serde_json::to_vec(message).map_err(|e| BusError::SerializationError(e.to_string()))?;

        debug!(subject = %self.subject, "Publishing message");

        self.client
            .publish(self.subject.clone(), payload.into())
            .await
            .map_err(|e| BusError::PublishFailed(e.to_string()))
    }

    async fn subscribe(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = Message> + Send>>, BusError> {
        debug!(subject = %self.subject, "Subscribing to subject");

        let subscriber = self
            .client
            .subscribe(self.subject.clone())
            .await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))?;

        let stream = subscriber.filter_map(|msg| async move {
            serde_json::from_slice::<Message>(&msg.payload).ok()
        });

        Ok(Box::pin(stream))
    }

    async fn request(&self, message: &Message, timeout_ms: u64) -> Result<Message, BusError> {
        let payload =
            serde_json::to_vec(message).map_err(|e| BusError::SerializationError(e.to_string()))?;

        debug!(subject = %self.subject, timeout_ms = %timeout_ms, "Sending request");

        let response = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            self.client.request(self.subject.clone(), payload.into()),
        )
        .await
        .map_err(|_| BusError::Timeout)?
        .map_err(|e| BusError::RequestFailed(e.to_string()))?;

        serde_json::from_slice(&response.payload)
            .map_err(|e| BusError::DeserializationError(e.to_string()))
    }
}
