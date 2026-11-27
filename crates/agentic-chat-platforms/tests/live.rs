//! Checks against the real chat platforms.
//!
//! Ignored by default; each test skips itself when its variables are not
//! set. Each one sends a single message to the channel you name. See
//! `docs/LIVE_TESTS.md`.
#![cfg(all(feature = "discord", feature = "telegram", feature = "slack"))]

use dasein_agentic_chat_platforms::{
    ChatPlatform, DiscordBot, OutgoingMessage, SlackBot, TelegramBot,
};
use std::sync::Arc;
use std::time::Duration;

fn settings<const N: usize>(names: [&str; N]) -> Option<[String; N]> {
    let mut values = Vec::with_capacity(N);
    for name in names {
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => values.push(value),
            _ => {
                eprintln!("skipped: {name} is not set");
                return None;
            }
        }
    }
    values.try_into().ok()
}

/// Start the bot, wait until it reports healthy, send one message, stop.
async fn send_one(bot: Arc<dyn ChatPlatform>, channel: String) {
    let runner = bot.clone();
    // `start` may run for as long as the bot is connected.
    let running = tokio::spawn(async move { runner.start().await });

    let mut healthy = false;
    for _ in 0..60 {
        if bot.is_healthy().await {
            healthy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(healthy, "the bot did not become healthy within 30 s");

    let sent = bot
        .send(OutgoingMessage::new(
            channel,
            "Live test from the Dasein Agentic SDK.",
        ))
        .await;
    assert!(sent.is_ok(), "send failed: {:?}", sent.err());

    let _ = bot.stop().await;
    running.abort();
}

#[tokio::test]
#[ignore = "connects to Discord and posts a message; see docs/LIVE_TESTS.md"]
async fn discord_live() {
    let Some([token, channel]) = settings(["DISCORD_BOT_TOKEN", "LIVE_DISCORD_CHANNEL_ID"]) else {
        return;
    };
    send_one(Arc::new(DiscordBot::new(token)), channel).await;
}

#[tokio::test]
#[ignore = "connects to Telegram and posts a message; see docs/LIVE_TESTS.md"]
async fn telegram_live() {
    let Some([token, chat]) = settings(["TELEGRAM_BOT_TOKEN", "LIVE_TELEGRAM_CHAT_ID"]) else {
        return;
    };
    send_one(Arc::new(TelegramBot::new(token)), chat).await;
}

#[tokio::test]
#[ignore = "connects to Slack and posts a message; see docs/LIVE_TESTS.md"]
async fn slack_live() {
    let Some([bot_token, app_token, channel]) = settings([
        "SLACK_BOT_TOKEN",
        "SLACK_APP_TOKEN",
        "LIVE_SLACK_CHANNEL_ID",
    ]) else {
        return;
    };
    send_one(Arc::new(SlackBot::new(bot_token, app_token)), channel).await;
}
