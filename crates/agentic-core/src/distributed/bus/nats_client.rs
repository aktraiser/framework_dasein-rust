//! NATS Client - Real connection to NATS server with JetStream.
//!
//! Provides:
//! - Connection management with auto-reconnect
//! - JetStream stream creation
//! - Publish/Subscribe with serialization
//! - Queue groups for work distribution

use async_nats::jetstream::{self, consumer, stream, Context as JetStreamContext};
use async_nats::{Client, ConnectOptions, Message};
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;
use tracing::{debug, info};

use super::types::BusError;

/// NATS client configuration.
#[derive(Debug, Clone)]
pub struct NatsConfig {
    /// NATS server URL(s)
    pub urls: Vec<String>,
    /// Client name for identification
    pub client_name: String,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Request timeout
    pub request_timeout: Duration,
    /// Enable JetStream
    pub jetstream: bool,
    /// Auto-create streams
    pub auto_create_streams: bool,
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            urls: vec!["nats://localhost:4222".to_string()],
            client_name: "agentic-rs".to_string(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            jetstream: true,
            auto_create_streams: true,
        }
    }
}

impl NatsConfig {
    /// Create config with single URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            ..Default::default()
        }
    }

    /// Add additional URLs for clustering.
    pub fn with_urls(mut self, urls: Vec<String>) -> Self {
        self.urls = urls;
        self
    }

    /// Set client name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.client_name = name.into();
        self
    }
}

/// Stream configuration for JetStream.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub name: String,
    pub subjects: Vec<String>,
    pub max_messages: i64,
    pub max_bytes: i64,
    pub max_age: Duration,
    pub storage: StreamStorage,
    pub retention: StreamRetention,
}

#[derive(Debug, Clone, Copy)]
pub enum StreamStorage {
    File,
    Memory,
}

#[derive(Debug, Clone, Copy)]
pub enum StreamRetention {
    Limits,
    Interest,
    WorkQueue,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            name: "DEFAULT".to_string(),
            subjects: vec![],
            max_messages: 1_000_000,
            max_bytes: 1024 * 1024 * 1024,                  // 1GB
            max_age: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
            storage: StreamStorage::File,
            retention: StreamRetention::Limits,
        }
    }
}

impl StreamConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn subjects(mut self, subjects: Vec<String>) -> Self {
        self.subjects = subjects;
        self
    }

    pub fn work_queue(mut self) -> Self {
        self.retention = StreamRetention::WorkQueue;
        self
    }

    pub fn memory(mut self) -> Self {
        self.storage = StreamStorage::Memory;
        self
    }
}

/// Real NATS client with JetStream support.
pub struct NatsClient {
    _config: NatsConfig,
    client: Client,
    jetstream: Option<JetStreamContext>,
}

impl NatsClient {
    /// Connect to NATS server.
    pub async fn connect(config: NatsConfig) -> Result<Self, BusError> {
        info!("Connecting to NATS: {:?}", config.urls);

        let options = ConnectOptions::new()
            .name(&config.client_name)
            .connection_timeout(config.connect_timeout)
            .request_timeout(Some(config.request_timeout));

        let client = async_nats::connect_with_options(config.urls.join(","), options)
            .await
            .map_err(|e| BusError::ConnectionFailed(e.to_string()))?;

        info!("Connected to NATS server");

        let jetstream = if config.jetstream {
            Some(jetstream::new(client.clone()))
        } else {
            None
        };

        Ok(Self {
            _config: config,
            client,
            jetstream,
        })
    }

    /// Connect with default config.
    pub async fn connect_default() -> Result<Self, BusError> {
        Self::connect(NatsConfig::default()).await
    }

    /// Get the raw NATS client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Get JetStream context.
    pub fn jetstream(&self) -> Option<&JetStreamContext> {
        self.jetstream.as_ref()
    }

    /// Create or get a JetStream stream.
    pub async fn ensure_stream(&self, config: StreamConfig) -> Result<stream::Stream, BusError> {
        let js = self
            .jetstream
            .as_ref()
            .ok_or_else(|| BusError::StreamNotFound("JetStream not enabled".to_string()))?;

        let storage = match config.storage {
            StreamStorage::File => stream::StorageType::File,
            StreamStorage::Memory => stream::StorageType::Memory,
        };

        let retention = match config.retention {
            StreamRetention::Limits => stream::RetentionPolicy::Limits,
            StreamRetention::Interest => stream::RetentionPolicy::Interest,
            StreamRetention::WorkQueue => stream::RetentionPolicy::WorkQueue,
        };

        let stream_config = stream::Config {
            name: config.name.clone(),
            subjects: config.subjects,
            max_messages: config.max_messages,
            max_bytes: config.max_bytes,
            max_age: config.max_age,
            storage,
            retention,
            ..Default::default()
        };

        // Try to get existing stream, or create new one
        match js.get_stream(&config.name).await {
            Ok(stream) => {
                debug!("Using existing stream: {}", config.name);
                Ok(stream)
            }
            Err(_) => {
                info!("Creating new stream: {}", config.name);
                js.create_stream(stream_config)
                    .await
                    .map_err(|e| BusError::StreamNotFound(e.to_string()))
            }
        }
    }

    /// Publish a message to a subject.
    pub async fn publish<T: Serialize>(&self, subject: &str, payload: &T) -> Result<(), BusError> {
        let data = serde_json::to_vec(payload)
            .map_err(|e| BusError::SerializationFailed(e.to_string()))?;

        self.client
            .publish(subject.to_string(), data.into())
            .await
            .map_err(|e| BusError::PublishFailed(e.to_string()))?;

        debug!("Published to {}", subject);
        Ok(())
    }

    /// Publish to JetStream (with persistence).
    pub async fn publish_jetstream<T: Serialize>(
        &self,
        subject: &str,
        payload: &T,
    ) -> Result<jetstream::context::PublishAckFuture, BusError> {
        let js = self
            .jetstream
            .as_ref()
            .ok_or_else(|| BusError::PublishFailed("JetStream not enabled".to_string()))?;

        let data = serde_json::to_vec(payload)
            .map_err(|e| BusError::SerializationFailed(e.to_string()))?;

        js.publish(subject.to_string(), data.into())
            .await
            .map_err(|e| BusError::PublishFailed(e.to_string()))
    }

    /// Request-reply pattern.
    pub async fn request<T: Serialize, R: DeserializeOwned>(
        &self,
        subject: &str,
        payload: &T,
    ) -> Result<R, BusError> {
        let data = serde_json::to_vec(payload)
            .map_err(|e| BusError::SerializationFailed(e.to_string()))?;

        let response = self
            .client
            .request(subject.to_string(), data.into())
            .await
            .map_err(|e| BusError::PublishFailed(e.to_string()))?;

        serde_json::from_slice(&response.payload)
            .map_err(|e| BusError::DeserializationFailed(e.to_string()))
    }

    /// Subscribe to a subject.
    pub async fn subscribe(&self, subject: &str) -> Result<async_nats::Subscriber, BusError> {
        self.client
            .subscribe(subject.to_string())
            .await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))
    }

    /// Subscribe with queue group (work distribution).
    pub async fn queue_subscribe(
        &self,
        subject: &str,
        queue_group: &str,
    ) -> Result<async_nats::Subscriber, BusError> {
        self.client
            .queue_subscribe(subject.to_string(), queue_group.to_string())
            .await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))
    }

    /// Create a JetStream consumer.
    pub async fn create_consumer(
        &self,
        stream_name: &str,
        consumer_name: &str,
        filter_subject: Option<&str>,
    ) -> Result<consumer::Consumer<consumer::pull::Config>, BusError> {
        let js = self
            .jetstream
            .as_ref()
            .ok_or_else(|| BusError::SubscribeFailed("JetStream not enabled".to_string()))?;

        let stream = js
            .get_stream(stream_name)
            .await
            .map_err(|e| BusError::StreamNotFound(e.to_string()))?;

        let config = consumer::pull::Config {
            name: Some(consumer_name.to_string()),
            durable_name: Some(consumer_name.to_string()),
            filter_subject: filter_subject.map(std::string::ToString::to_string).unwrap_or_default(),
            ..Default::default()
        };

        stream
            .create_consumer(config)
            .await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.client.connection_state() == async_nats::connection::State::Connected
    }

    /// Close connection.
    pub async fn close(self) {
        // Client is dropped automatically
        info!("NATS connection closed");
    }
}

/// Helper to deserialize a NATS message.
pub fn deserialize_message<T: DeserializeOwned>(msg: &Message) -> Result<T, BusError> {
    serde_json::from_slice(&msg.payload).map_err(|e| BusError::DeserializationFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    // These tests require a running NATS server
    // Run with: docker run -d --name nats -p 4222:4222 nats:latest -js

    #[tokio::test]
    #[ignore = "requires NATS server"]
    async fn test_connect() {
        let client = NatsClient::connect_default().await.unwrap();
        assert!(client.is_connected());
    }

    #[tokio::test]
    #[ignore = "requires NATS server"]
    async fn test_publish_subscribe() {
        let client = NatsClient::connect_default().await.unwrap();

        let mut sub = client.subscribe("test.subject").await.unwrap();

        client.publish("test.subject", &"Hello").await.unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(1), sub.next())
            .await
            .unwrap()
            .unwrap();

        let payload: String = deserialize_message(&msg).unwrap();
        assert_eq!(payload, "Hello");
    }

    #[tokio::test]
    #[ignore = "requires NATS server"]
    async fn test_jetstream_stream() {
        let client = NatsClient::connect_default().await.unwrap();

        let config = StreamConfig::new("TEST_STREAM")
            .subjects(vec!["test.>".to_string()])
            .memory();

        let stream = client.ensure_stream(config).await.unwrap();
        assert_eq!(stream.cached_info().config.name, "TEST_STREAM");
    }
}
