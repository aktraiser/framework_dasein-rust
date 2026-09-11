//! Slack App implementation.
//!
//! Uses the slack-morphism library to interact with the Slack API.
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_chat_platforms::{SlackBot, ChatPlatform};
//!
//! let bot = SlackBot::new("BOT_TOKEN", "APP_TOKEN");
//! bot.start().await?;
//!
//! let mut rx = bot.subscribe();
//! while let Ok(msg) = rx.recv().await {
//!     println!("Received from {}: {}", msg.user_id, msg.content);
//! }
//! ```

#[cfg(feature = "slack")]
mod slack_impl {
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::{broadcast, RwLock};
    use tracing::{debug, info};

    use slack_morphism::prelude::*;

    use crate::{
        error::{ChatError, ChatResult},
        traits::{ChatPlatform, PlatformConfig},
        types::{IncomingMessage, OutgoingMessage, Platform},
    };

    /// Slack bot implementation using Socket Mode.
    pub struct SlackBot {
        config: PlatformConfig,
        app_token: String,
        tx: broadcast::Sender<IncomingMessage>,
        running: Arc<RwLock<bool>>,
        bot_user_id: Arc<RwLock<Option<String>>>,
        client: Arc<RwLock<Option<SlackClient<SlackClientHyperHttpsConnector>>>>,
    }

    impl SlackBot {
        /// Create a new Slack bot with bot token and app token.
        ///
        /// The bot token (xoxb-...) is used to send messages.
        /// The app token (xapp-...) is used for Socket Mode connections.
        #[must_use]
        pub fn new(bot_token: impl Into<String>, app_token: impl Into<String>) -> Self {
            let (tx, _) = broadcast::channel(1000);

            Self {
                config: PlatformConfig::new(Platform::Slack, bot_token),
                app_token: app_token.into(),
                tx,
                running: Arc::new(RwLock::new(false)),
                bot_user_id: Arc::new(RwLock::new(None)),
                client: Arc::new(RwLock::new(None)),
            }
        }

        /// Socket-mode application token.
        pub fn app_token(&self) -> &str {
            &self.app_token
        }

        /// Create from a platform config.
        ///
        /// The config should have the bot token, and the app token should be
        /// in `config.extra["app_token"]`.
        pub fn from_config(config: PlatformConfig) -> ChatResult<Self> {
            let app_token = config.extra["app_token"]
                .as_str()
                .ok_or_else(|| {
                    ChatError::ConfigurationError("Missing app_token in config.extra".into())
                })?
                .to_string();

            let (tx, _) = broadcast::channel(config.channel_buffer_size);

            Ok(Self {
                config,
                app_token,
                tx,
                running: Arc::new(RwLock::new(false)),
                bot_user_id: Arc::new(RwLock::new(None)),
                client: Arc::new(RwLock::new(None)),
            })
        }
    }

    #[async_trait]
    impl ChatPlatform for SlackBot {
        fn platform(&self) -> Platform {
            Platform::Slack
        }

        async fn start(&self) -> ChatResult<()> {
            // Check if already running
            {
                let running = self.running.read().await;
                if *running {
                    return Ok(());
                }
            }

            // Create client
            let client = SlackClient::new(
                SlackClientHyperConnector::new()
                    .map_err(|e| ChatError::ConnectionFailed(e.to_string()))?,
            );

            // Test authentication
            let token = SlackApiToken::new(self.config.token.clone().into());
            let session = client.open_session(&token);

            let auth_response = session
                .auth_test()
                .await
                .map_err(|e| ChatError::AuthenticationFailed(e.to_string()))?;

            info!(
                bot_id = ?auth_response.bot_id,
                user_id = ?auth_response.user_id,
                team = ?auth_response.team,
                "Slack bot authenticated"
            );

            // Store bot user ID
            {
                let mut id = self.bot_user_id.write().await;
                *id = Some(auth_response.user_id.to_string());
            }

            // Store client
            {
                let mut c = self.client.write().await;
                *c = Some(client.clone());
            }

            // Mark as running
            {
                let mut running = self.running.write().await;
                *running = true;
            }

            // Note: Socket Mode requires implementing SlackSocketModeClientListener trait
            // and using SlackSocketModeClientsManager. For now, we provide send-only
            // capability. Full event listening will be added in a future version.
            //
            // To receive messages, you would need to:
            // 1. Implement SlackSocketModeClientListener trait
            // 2. Use SlackSocketModeClientsManager::register_new_token()
            // 3. Call manager.start() to begin receiving events
            let running = self.running.clone();

            tokio::spawn(async move {
                info!("Slack bot started (send-only mode)");

                // Keep alive until stopped
                loop {
                    {
                        let is_running = running.read().await;
                        if !*is_running {
                            info!("Slack bot stopping");
                            break;
                        }
                    }

                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            });

            Ok(())
        }

        async fn stop(&self) -> ChatResult<()> {
            let mut running = self.running.write().await;
            *running = false;
            info!("Slack bot stopped");
            Ok(())
        }

        async fn send(&self, message: OutgoingMessage) -> ChatResult<String> {
            let client = {
                let c = self.client.read().await;
                c.clone().ok_or(ChatError::NotInitialized)?
            };

            let token = SlackApiToken::new(self.config.token.clone().into());
            let session = client.open_session(&token);

            // Build the message content
            let channel = SlackChannelId::new(message.channel_id.clone());
            let content = SlackMessageContent::new().with_text(message.content.clone());

            let request = SlackApiChatPostMessageRequest::new(channel, content);

            // Add thread_ts if replying
            let request = if let Some(reply_to) = &message.reply_to {
                request.with_thread_ts(SlackTs::new(reply_to.clone()))
            } else {
                request
            };

            let response = session
                .chat_post_message(&request)
                .await
                .map_err(|e| ChatError::SendFailed(e.to_string()))?;

            let message_id = response.ts.to_string();

            debug!(
                message_id = %message_id,
                channel_id = %message.channel_id,
                "Sent Slack message"
            );

            Ok(message_id)
        }

        fn subscribe(&self) -> broadcast::Receiver<IncomingMessage> {
            self.tx.subscribe()
        }

        async fn is_healthy(&self) -> bool {
            let client = {
                let c = self.client.read().await;
                c.clone()
            };

            if let Some(client) = client {
                let token = SlackApiToken::new(self.config.token.clone().into());
                let session = client.open_session(&token);
                session.auth_test().await.is_ok()
            } else {
                false
            }
        }

        fn bot_user_id(&self) -> Option<&str> {
            None
        }
    }
}

#[cfg(feature = "slack")]
pub use slack_impl::SlackBot;
