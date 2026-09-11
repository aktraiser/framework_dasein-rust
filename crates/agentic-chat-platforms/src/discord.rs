//! Discord Bot implementation.
//!
//! Uses the serenity library to interact with the Discord API.
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_chat_platforms::{DiscordBot, ChatPlatform};
//!
//! let bot = DiscordBot::new("BOT_TOKEN");
//! bot.start().await?;
//!
//! let mut rx = bot.subscribe();
//! while let Ok(msg) = rx.recv().await {
//!     println!("Received from {}: {}", msg.username.unwrap_or_default(), msg.content);
//! }
//! ```

#[cfg(feature = "discord")]
mod discord_impl {
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::{broadcast, RwLock};
    use tracing::{debug, error, info, warn};

    use serenity::{
        all::{
            Context, CreateEmbed, CreateEmbedFooter, CreateMessage, EventHandler, GatewayIntents,
            Message as DiscordMessage, Ready,
        },
        Client,
    };

    use crate::{
        error::{ChatError, ChatResult},
        traits::{ChatPlatform, PlatformConfig},
        types::{IncomingMessage, OutgoingMessage, Platform},
    };

    /// Discord bot implementation.
    pub struct DiscordBot {
        config: PlatformConfig,
        tx: broadcast::Sender<IncomingMessage>,
        running: Arc<RwLock<bool>>,
        bot_user_id: Arc<RwLock<Option<String>>>,
        http: Arc<RwLock<Option<Arc<serenity::http::Http>>>>,
    }

    impl DiscordBot {
        /// Create a new Discord bot with the given token.
        #[must_use]
        pub fn new(token: impl Into<String>) -> Self {
            let (tx, _) = broadcast::channel(1000);

            Self {
                config: PlatformConfig::new(Platform::Discord, token),
                tx,
                running: Arc::new(RwLock::new(false)),
                bot_user_id: Arc::new(RwLock::new(None)),
                http: Arc::new(RwLock::new(None)),
            }
        }

        /// Create from a platform config.
        #[must_use]
        pub fn from_config(config: PlatformConfig) -> Self {
            let (tx, _) = broadcast::channel(config.channel_buffer_size);

            Self {
                config,
                tx,
                running: Arc::new(RwLock::new(false)),
                bot_user_id: Arc::new(RwLock::new(None)),
                http: Arc::new(RwLock::new(None)),
            }
        }
    }

    /// Event handler for Discord events.
    struct Handler {
        tx: broadcast::Sender<IncomingMessage>,
        bot_user_id: Arc<RwLock<Option<String>>>,
    }

    #[async_trait]
    impl EventHandler for Handler {
        async fn message(&self, _ctx: Context, msg: DiscordMessage) {
            // Skip messages from bots (including self)
            if msg.author.bot {
                return;
            }

            // Check if this is from ourselves
            {
                let bot_id = self.bot_user_id.read().await;
                if let Some(id) = bot_id.as_ref() {
                    if msg.author.id.to_string() == *id {
                        return;
                    }
                }
            }

            let incoming = IncomingMessage::new(
                Platform::Discord,
                msg.channel_id.to_string(),
                msg.author.id.to_string(),
                &msg.content,
            )
            .with_id(msg.id.to_string())
            .with_username(&msg.author.name);

            // Check if it's a DM
            let incoming = if msg.guild_id.is_none() {
                incoming.as_direct()
            } else {
                incoming
            };

            // Set reply_to if this is a reply
            let incoming = if let Some(ref_msg) = &msg.referenced_message {
                incoming.replying_to(ref_msg.id.to_string())
            } else {
                incoming
            };

            // Add metadata
            let metadata = serde_json::json!({
                "guild_id": msg.guild_id.map(|g| g.to_string()),
                "channel_id": msg.channel_id.to_string(),
            });
            let incoming = incoming.with_metadata(metadata);

            debug!(
                message_id = %incoming.id,
                channel_id = %incoming.channel_id,
                "Received Discord message"
            );

            if self.tx.send(incoming).is_err() {
                warn!("No subscribers for Discord messages");
            }
        }

        async fn ready(&self, _ctx: Context, ready: Ready) {
            info!(
                username = %ready.user.name,
                user_id = %ready.user.id,
                "Discord bot connected"
            );

            // Store bot user ID
            let mut bot_id = self.bot_user_id.write().await;
            *bot_id = Some(ready.user.id.to_string());
        }
    }

    #[async_trait]
    impl ChatPlatform for DiscordBot {
        fn platform(&self) -> Platform {
            Platform::Discord
        }

        async fn start(&self) -> ChatResult<()> {
            // Check if already running
            {
                let running = self.running.read().await;
                if *running {
                    return Ok(());
                }
            }

            let intents = GatewayIntents::GUILD_MESSAGES
                | GatewayIntents::DIRECT_MESSAGES
                | GatewayIntents::MESSAGE_CONTENT;

            let handler = Handler {
                tx: self.tx.clone(),
                bot_user_id: self.bot_user_id.clone(),
            };

            let mut client = Client::builder(&self.config.token, intents)
                .event_handler(handler)
                .await
                .map_err(|e| ChatError::ConnectionFailed(e.to_string()))?;

            // Store HTTP client for sending messages
            {
                let mut http = self.http.write().await;
                *http = Some(client.http.clone());
            }

            // Mark as running
            {
                let mut running = self.running.write().await;
                *running = true;
            }

            let running = self.running.clone();

            // Start the client in a background task
            tokio::spawn(async move {
                info!("Starting Discord client");

                if let Err(e) = client.start().await {
                    error!(error = %e, "Discord client error");
                }

                // Mark as not running when client stops
                let mut is_running = running.write().await;
                *is_running = false;
            });

            Ok(())
        }

        async fn stop(&self) -> ChatResult<()> {
            let mut running = self.running.write().await;
            *running = false;
            info!("Discord bot stopped");
            Ok(())
        }

        async fn send(&self, message: OutgoingMessage) -> ChatResult<String> {
            let http = {
                let http = self.http.read().await;
                http.clone().ok_or(ChatError::NotInitialized)?
            };

            let channel_id: u64 = message
                .channel_id
                .parse()
                .map_err(|_| ChatError::InvalidChannelId(message.channel_id.clone()))?;

            let channel_id = serenity::model::id::ChannelId::new(channel_id);

            // Build the message
            let mut builder = CreateMessage::new().content(&message.content);

            // Add reply reference if needed
            if let Some(reply_to) = &message.reply_to {
                if let Ok(msg_id) = reply_to.parse::<u64>() {
                    builder = builder.reference_message(
                        serenity::model::channel::MessageReference::from((
                            channel_id,
                            serenity::model::id::MessageId::new(msg_id),
                        )),
                    );
                }
            }

            // Add embeds if present
            if !message.embeds.is_empty() {
                let embeds: Vec<CreateEmbed> = message
                    .embeds
                    .iter()
                    .map(|e| {
                        let mut embed = CreateEmbed::new();

                        if let Some(title) = &e.title {
                            embed = embed.title(title);
                        }
                        if let Some(desc) = &e.description {
                            embed = embed.description(desc);
                        }
                        if let Some(url) = &e.url {
                            embed = embed.url(url);
                        }
                        if let Some(color) = e.color {
                            embed = embed.color(color);
                        }
                        if let Some(thumb) = &e.thumbnail {
                            embed = embed.thumbnail(thumb);
                        }
                        if let Some(img) = &e.image {
                            embed = embed.image(img);
                        }
                        if let Some(footer) = &e.footer {
                            embed = embed.footer(CreateEmbedFooter::new(footer));
                        }

                        for field in &e.fields {
                            embed = embed.field(&field.name, &field.value, field.inline);
                        }

                        embed
                    })
                    .collect();

                builder = builder.embeds(embeds);
            }

            let sent = channel_id
                .send_message(&http, builder)
                .await
                .map_err(|e| ChatError::SendFailed(e.to_string()))?;

            let message_id = sent.id.to_string();

            debug!(
                message_id = %message_id,
                channel_id = %message.channel_id,
                "Sent Discord message"
            );

            Ok(message_id)
        }

        fn subscribe(&self) -> broadcast::Receiver<IncomingMessage> {
            self.tx.subscribe()
        }

        async fn is_healthy(&self) -> bool {
            let running = self.running.read().await;
            *running
        }

        fn bot_user_id(&self) -> Option<&str> {
            None // Would need different approach for this
        }
    }
}

#[cfg(feature = "discord")]
pub use discord_impl::DiscordBot;
