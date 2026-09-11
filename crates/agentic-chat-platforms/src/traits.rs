//! Traits for chat platform implementations.

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::{
    error::ChatResult,
    types::{IncomingMessage, OutgoingMessage, Platform},
};

// ============================================================================
// CHAT PLATFORM TRAIT
// ============================================================================

/// Trait for chat platform implementations.
///
/// Each platform (Telegram, Discord, etc.) implements this trait to provide
/// a unified interface for sending and receiving messages.
///
/// # Example
///
/// ```rust,ignore
/// use dasein_agentic_chat_platforms::{ChatPlatform, Platform, IncomingMessage};
///
/// let bot = TelegramBot::new("BOT_TOKEN");
/// bot.start().await?;
///
/// // Listen for messages
/// let mut rx = bot.subscribe();
/// while let Ok(msg) = rx.recv().await {
///     println!("Received: {}", msg.content);
///
///     // Reply
///     let reply = OutgoingMessage::reply(&msg, "Got it!");
///     bot.send(reply).await?;
/// }
/// ```
#[async_trait]
pub trait ChatPlatform: Send + Sync {
    /// Get the platform type.
    fn platform(&self) -> Platform;

    /// Start listening for incoming messages.
    ///
    /// This typically starts a background task that polls or connects
    /// to the platform's API.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform fails to start (e.g., auth failure).
    async fn start(&self) -> ChatResult<()>;

    /// Stop listening for messages.
    ///
    /// This should cleanly shut down any background tasks.
    async fn stop(&self) -> ChatResult<()>;

    /// Send a message to the platform.
    ///
    /// # Returns
    ///
    /// The message ID assigned by the platform.
    ///
    /// # Errors
    ///
    /// Returns an error if the message fails to send.
    async fn send(&self, message: OutgoingMessage) -> ChatResult<String>;

    /// Subscribe to incoming messages.
    ///
    /// Returns a broadcast receiver that will receive all incoming messages.
    /// Multiple subscribers can be created.
    fn subscribe(&self) -> broadcast::Receiver<IncomingMessage>;

    /// Check if the platform is healthy/connected.
    async fn is_healthy(&self) -> bool;

    /// Get the bot's own user ID (if applicable).
    fn bot_user_id(&self) -> Option<&str> {
        None
    }
}

// ============================================================================
// MESSAGE HANDLER TRAIT
// ============================================================================

/// Trait for handling incoming messages.
///
/// Implement this trait to process messages from chat platforms.
#[async_trait]
pub trait MessageHandler: Send + Sync {
    /// Handle an incoming message.
    ///
    /// # Returns
    ///
    /// An optional response message. Return `None` to not respond.
    async fn handle(&self, message: &IncomingMessage) -> ChatResult<Option<OutgoingMessage>>;

    /// Check if this handler should process the message.
    ///
    /// Return `false` to skip this handler for certain messages.
    fn should_handle(&self, message: &IncomingMessage) -> bool {
        // By default, handle all messages
        let _ = message;
        true
    }
}

// ============================================================================
// PLATFORM CONFIG
// ============================================================================

/// Configuration for a chat platform.
#[derive(Debug, Clone)]
pub struct PlatformConfig {
    /// Platform type.
    pub platform: Platform,

    /// API token or credentials.
    pub token: String,

    /// Optional webhook URL (for platforms that support it).
    pub webhook_url: Option<String>,

    /// Buffer size for the message channel.
    pub channel_buffer_size: usize,

    /// Additional platform-specific configuration.
    pub extra: serde_json::Value,
}

impl PlatformConfig {
    /// Create a new platform config with token.
    #[must_use]
    pub fn new(platform: Platform, token: impl Into<String>) -> Self {
        Self {
            platform,
            token: token.into(),
            webhook_url: None,
            channel_buffer_size: 1000,
            extra: serde_json::Value::Null,
        }
    }

    /// Set webhook URL.
    #[must_use]
    pub fn with_webhook(mut self, url: impl Into<String>) -> Self {
        self.webhook_url = Some(url.into());
        self
    }

    /// Set channel buffer size.
    #[must_use]
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.channel_buffer_size = size;
        self
    }

    /// Set extra configuration.
    #[must_use]
    pub fn with_extra(mut self, extra: serde_json::Value) -> Self {
        self.extra = extra;
        self
    }
}
