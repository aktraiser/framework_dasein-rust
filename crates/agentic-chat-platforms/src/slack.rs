//! Slack App implementation.
//!
//! Uses the slack-morphism library to interact with the Slack API.
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_chat_platforms::{SlackBot, ChatPlatform};
//!
//! // Socket Mode must be enabled for the app, with message events subscribed.
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

    type SlackConnector = SlackClientHyperHttpsConnector;

    /// What the Socket Mode callback needs to turn an event into a message.
    struct ListenerState {
        tx: broadcast::Sender<IncomingMessage>,
        bot_user_id: Option<String>,
    }

    /// Slack bot implementation using Socket Mode.
    ///
    /// Messages are sent with the bot token over the Web API and received
    /// over a Socket Mode connection opened with the app token, so no public
    /// URL is needed. The Slack app must have Socket Mode enabled and be
    /// subscribed to the message events it should see (`message.channels`,
    /// `message.im`...).
    pub struct SlackBot {
        config: PlatformConfig,
        app_token: String,
        tx: broadcast::Sender<IncomingMessage>,
        running: Arc<RwLock<bool>>,
        bot_user_id: Arc<RwLock<Option<String>>>,
        client: Arc<RwLock<Option<Arc<SlackClient<SlackConnector>>>>>,
        listener: Arc<RwLock<Option<Arc<SlackClientSocketModeListener<SlackConnector>>>>>,
    }

    /// Socket Mode callback: forward user messages to the subscribers.
    async fn on_push_event(
        event: SlackPushEventCallback,
        _client: Arc<SlackClient<SlackConnector>>,
        states: SlackClientEventsUserState,
    ) -> UserCallbackResult<()> {
        let SlackEventCallbackBody::Message(message) = event.event else {
            return Ok(());
        };
        let states = states.read().await;
        let Some(state) = states.get_user_state::<ListenerState>() else {
            return Ok(());
        };
        if let Some(incoming) = incoming_message(message, state.bot_user_id.as_deref()) {
            // No subscriber at the moment is not an error.
            let _ = state.tx.send(incoming);
        }
        Ok(())
    }

    /// Convert a Slack message event, or `None` for events that are not a
    /// message someone wrote: edits, joins and other subtypes, messages from
    /// bots (ours included, or the bot would answer itself), empty text.
    fn incoming_message(
        message: SlackMessageEvent,
        bot_user_id: Option<&str>,
    ) -> Option<IncomingMessage> {
        if message.subtype.is_some() || message.sender.bot_id.is_some() {
            return None;
        }
        let user = message.sender.user?.to_string();
        if bot_user_id == Some(user.as_str()) {
            return None;
        }
        let channel = message.origin.channel?.to_string();
        let text = message.content.and_then(|content| content.text)?;
        if text.is_empty() {
            return None;
        }
        let mut incoming = IncomingMessage::new(Platform::Slack, channel, user, text);
        incoming.id = message.origin.ts.to_string();
        incoming.reply_to_id = message.origin.thread_ts.map(|ts| ts.to_string());
        incoming.is_direct =
            matches!(message.origin.channel_type, Some(SlackChannelType(ref kind)) if kind == "im");
        Some(incoming)
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
                listener: Arc::new(RwLock::new(None)),
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
                listener: Arc::new(RwLock::new(None)),
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
            let client = Arc::new(SlackClient::new(
                SlackClientHyperConnector::new()
                    .map_err(|e| ChatError::ConnectionFailed(e.to_string()))?,
            ));

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
            let bot_user_id = auth_response.user_id.to_string();

            // Receive events over Socket Mode, with the app token.
            let environment = Arc::new(
                SlackClientEventsListenerEnvironment::new(client.clone()).with_user_state(
                    ListenerState {
                        tx: self.tx.clone(),
                        bot_user_id: Some(bot_user_id.clone()),
                    },
                ),
            );
            let listener = Arc::new(SlackClientSocketModeListener::new(
                &SlackClientSocketModeConfig::new(),
                environment,
                SlackSocketModeListenerCallbacks::new().with_push_events(on_push_event),
            ));
            let app_token = SlackApiToken::new(self.app_token.clone().into());
            listener
                .listen_for(&app_token)
                .await
                .map_err(|e| ChatError::ConnectionFailed(e.to_string()))?;
            listener.start().await;
            info!("Slack bot listening over Socket Mode");

            *self.bot_user_id.write().await = Some(bot_user_id);
            *self.client.write().await = Some(client);
            *self.listener.write().await = Some(listener);
            *self.running.write().await = true;

            Ok(())
        }

        async fn stop(&self) -> ChatResult<()> {
            if let Some(listener) = self.listener.write().await.take() {
                listener.shutdown().await;
            }
            *self.running.write().await = false;
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

    #[cfg(test)]
    mod tests {
        use super::*;

        fn event(json: serde_json::Value) -> SlackMessageEvent {
            serde_json::from_value(json).expect("a Slack message event")
        }

        #[test]
        fn user_message_is_converted() {
            let incoming = incoming_message(
                event(serde_json::json!({
                    "type": "message", "channel": "C123", "channel_type": "im",
                    "user": "U456", "text": "bonjour", "ts": "1700000000.000100",
                    "thread_ts": "1700000000.000001",
                })),
                Some("UBOT"),
            )
            .expect("a user message");
            assert_eq!(incoming.platform, Platform::Slack);
            assert_eq!(incoming.channel_id, "C123");
            assert_eq!(incoming.user_id, "U456");
            assert_eq!(incoming.content, "bonjour");
            assert_eq!(incoming.id, "1700000000.000100");
            assert_eq!(incoming.reply_to_id.as_deref(), Some("1700000000.000001"));
            assert!(incoming.is_direct);
        }

        #[test]
        fn bot_messages_and_non_messages_are_ignored() {
            let base = |extra: serde_json::Value| {
                let mut json = serde_json::json!({
                    "type": "message", "channel": "C123", "channel_type": "channel",
                    "user": "U456", "text": "hello", "ts": "1700000000.000100",
                });
                json.as_object_mut()
                    .unwrap()
                    .extend(extra.as_object().unwrap().clone());
                event(json)
            };
            // Our own message: answering it would loop forever.
            assert!(
                incoming_message(base(serde_json::json!({ "user": "UBOT" })), Some("UBOT"))
                    .is_none()
            );
            // Another bot.
            assert!(
                incoming_message(base(serde_json::json!({ "bot_id": "B1" })), Some("UBOT"))
                    .is_none()
            );
            // An edit, not a new message.
            assert!(incoming_message(
                base(serde_json::json!({ "subtype": "message_changed" })),
                Some("UBOT")
            )
            .is_none());
            // Nothing to read.
            assert!(
                incoming_message(base(serde_json::json!({ "text": "" })), Some("UBOT")).is_none()
            );
            // A normal message in a channel is kept, and is not direct.
            let kept = incoming_message(base(serde_json::json!({})), Some("UBOT")).unwrap();
            assert!(!kept.is_direct);
        }
    }
}

#[cfg(feature = "slack")]
pub use slack_impl::SlackBot;
