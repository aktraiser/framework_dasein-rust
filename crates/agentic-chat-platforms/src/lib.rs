//! # agentic-chat-platforms
//!
//! Chat platform integrations for agents.
//!
//! This crate provides a unified interface for receiving and sending messages
//! across multiple chat platforms (Telegram, Discord, Slack, etc.).
//!
//! # Features
//!
//! - **telegram**: Enable Telegram Bot API support (via teloxide)
//! - **discord**: Enable Discord Bot support (via serenity)
//! - **slack**: Enable Slack App support (via slack-morphism)
//! - **all**: Enable all platforms
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_chat_platforms::{
//!     MessageRouter, RouteConfig, Platform,
//!     TelegramBot, DiscordBot, ChatPlatform,
//! };
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create router
//!     let router = MessageRouter::new();
//!
//!     // Register platforms
//!     let telegram = Arc::new(TelegramBot::new("TELEGRAM_TOKEN"));
//!     let discord = Arc::new(DiscordBot::new("DISCORD_TOKEN"));
//!
//!     router.register_platform(telegram).await;
//!     router.register_platform(discord).await;
//!
//!     // Configure routing
//!     router.set_route("telegram:123456", RouteConfig::new("support-agent", "tenant-1")).await;
//!     router.set_route("*:*", RouteConfig::new("default-agent", "default")).await;
//!
//!     // Start all platforms
//!     router.start_all().await?;
//!
//!     // Process messages
//!     let mut rx = router.subscribe();
//!     while let Ok(msg) = rx.recv().await {
//!         println!("From {}: {}", msg.platform, msg.content);
//!
//!         // Route to agent
//!         if let Some(agent_id) = router.route(&msg).await {
//!             println!("Routing to agent: {}", agent_id);
//!
//!             // Send reply
//!             router.reply(&msg, "Got your message!").await?;
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```

mod error;
mod router;
mod traits;
mod types;

// Platform implementations
#[cfg(feature = "discord")]
mod discord;
#[cfg(feature = "slack")]
mod slack;
#[cfg(feature = "telegram")]
mod telegram;

// Re-exports
pub use error::{ChatError, ChatResult};
pub use router::{MessageRouter, RouteConfig, RoutingRule};
pub use traits::{ChatPlatform, MessageHandler, PlatformConfig};
pub use types::{
    Attachment, AttachmentType, Embed, EmbedField, IncomingMessage, MessageFormat, OutgoingMessage,
    Platform,
};

// Platform-specific exports
#[cfg(feature = "discord")]
pub use discord::DiscordBot;
#[cfg(feature = "slack")]
pub use slack::SlackBot;
#[cfg(feature = "telegram")]
pub use telegram::TelegramBot;
