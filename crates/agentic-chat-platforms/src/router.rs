//! Message router for routing messages to agents.
//!
//! The router manages multiple chat platforms and routes incoming messages
//! to the appropriate agent based on configuration.
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_chat_platforms::{MessageRouter, RouteConfig, Platform};
//!
//! let router = MessageRouter::new();
//!
//! // Register platforms
//! router.register_platform(telegram_bot).await;
//! router.register_platform(discord_bot).await;
//!
//! // Configure routing
//! router.set_route("telegram:123456", RouteConfig {
//!     default_agent: "support-agent".into(),
//!     tenant_id: "tenant-1".into(),
//!     rules: vec![],
//! }).await;
//!
//! // Start listening
//! router.start_all().await?;
//!
//! // Process messages
//! while let Some((platform, msg)) = router.recv().await {
//!     let agent_id = router.route(&msg).await;
//!     // ... dispatch to agent
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use crate::{
    error::{ChatError, ChatResult},
    traits::ChatPlatform,
    types::{IncomingMessage, OutgoingMessage, Platform},
};

// ============================================================================
// ROUTE CONFIG
// ============================================================================

/// Configuration for routing messages from a channel.
#[derive(Debug, Clone)]
pub struct RouteConfig {
    /// Default agent ID for this channel.
    pub default_agent: String,

    /// Tenant/organization ID.
    pub tenant_id: String,

    /// Custom routing rules (evaluated in order).
    pub rules: Vec<RoutingRule>,
}

impl RouteConfig {
    /// Create a new route config.
    #[must_use]
    pub fn new(default_agent: impl Into<String>, tenant_id: impl Into<String>) -> Self {
        Self {
            default_agent: default_agent.into(),
            tenant_id: tenant_id.into(),
            rules: vec![],
        }
    }

    /// Add a routing rule.
    #[must_use]
    pub fn with_rule(mut self, rule: RoutingRule) -> Self {
        self.rules.push(rule);
        self
    }
}

/// A rule for routing messages to specific agents.
#[derive(Debug, Clone)]
pub struct RoutingRule {
    /// Regex pattern to match against message content.
    pub pattern: String,

    /// Agent ID to route to if pattern matches.
    pub target_agent: String,

    /// Optional description of this rule.
    pub description: Option<String>,
}

impl RoutingRule {
    /// Create a new routing rule.
    #[must_use]
    pub fn new(pattern: impl Into<String>, target_agent: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            target_agent: target_agent.into(),
            description: None,
        }
    }

    /// Add a description.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Check if this rule matches a message.
    pub fn matches(&self, content: &str) -> bool {
        regex::Regex::new(&self.pattern)
            .map(|re| re.is_match(content))
            .unwrap_or(false)
    }
}

// ============================================================================
// MESSAGE ROUTER
// ============================================================================

/// Routes messages from multiple platforms to agents.
pub struct MessageRouter {
    /// Registered platforms.
    platforms: Arc<RwLock<HashMap<Platform, Arc<dyn ChatPlatform>>>>,

    /// Routing configuration by channel key (platform:channel_id).
    routes: Arc<RwLock<HashMap<String, RouteConfig>>>,

    /// Aggregated message channel.
    tx: broadcast::Sender<IncomingMessage>,

    /// Running state.
    running: Arc<RwLock<bool>>,
}

impl MessageRouter {
    /// Create a new message router.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(10000);

        Self {
            platforms: Arc::new(RwLock::new(HashMap::new())),
            routes: Arc::new(RwLock::new(HashMap::new())),
            tx,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Register a chat platform.
    pub async fn register_platform(&self, platform: Arc<dyn ChatPlatform>) {
        let platform_type = platform.platform();

        let mut platforms = self.platforms.write().await;
        platforms.insert(platform_type, platform);

        info!(platform = %platform_type, "Platform registered");
    }

    /// Unregister a platform.
    pub async fn unregister_platform(&self, platform: Platform) -> Option<Arc<dyn ChatPlatform>> {
        let mut platforms = self.platforms.write().await;
        let removed = platforms.remove(&platform);

        if removed.is_some() {
            info!(platform = %platform, "Platform unregistered");
        }

        removed
    }

    /// Set routing configuration for a channel.
    pub async fn set_route(&self, channel_key: &str, config: RouteConfig) {
        let mut routes = self.routes.write().await;
        routes.insert(channel_key.to_string(), config);

        debug!(channel_key = %channel_key, "Route configured");
    }

    /// Remove routing configuration.
    pub async fn remove_route(&self, channel_key: &str) {
        let mut routes = self.routes.write().await;
        routes.remove(channel_key);
    }

    /// Get routing configuration for a channel.
    pub async fn get_route(&self, channel_key: &str) -> Option<RouteConfig> {
        let routes = self.routes.read().await;
        routes.get(channel_key).cloned()
    }

    /// Determine which agent should handle a message.
    ///
    /// Returns the agent ID based on routing rules, or None if no route is configured.
    pub async fn route(&self, message: &IncomingMessage) -> Option<String> {
        let channel_key = message.routing_key();
        let routes = self.routes.read().await;

        if let Some(config) = routes.get(&channel_key) {
            // Check custom rules first
            for rule in &config.rules {
                if rule.matches(&message.content) {
                    debug!(
                        channel_key = %channel_key,
                        agent = %rule.target_agent,
                        rule = ?rule.description,
                        "Routing by rule"
                    );
                    return Some(rule.target_agent.clone());
                }
            }

            // Fall back to default agent
            debug!(
                channel_key = %channel_key,
                agent = %config.default_agent,
                "Routing to default agent"
            );
            return Some(config.default_agent.clone());
        }

        // Try platform-wide default (platform:*)
        let platform_key = format!("{}:*", message.platform);
        if let Some(config) = routes.get(&platform_key) {
            return Some(config.default_agent.clone());
        }

        // Try global default (*:*)
        if let Some(config) = routes.get("*:*") {
            return Some(config.default_agent.clone());
        }

        warn!(
            channel_key = %channel_key,
            "No route configured for channel"
        );
        None
    }

    /// Get the tenant ID for a message.
    pub async fn tenant_for(&self, message: &IncomingMessage) -> Option<String> {
        let channel_key = message.routing_key();
        let routes = self.routes.read().await;

        routes.get(&channel_key).map(|c| c.tenant_id.clone())
    }

    /// Start all registered platforms.
    pub async fn start_all(&self) -> ChatResult<()> {
        let platforms = self.platforms.read().await;

        for (platform_type, platform) in platforms.iter() {
            info!(platform = %platform_type, "Starting platform");
            platform.start().await?;

            // Forward messages from this platform to the aggregated channel
            let mut rx = platform.subscribe();
            let tx = self.tx.clone();
            let platform_name = *platform_type;

            tokio::spawn(async move {
                while let Ok(msg) = rx.recv().await {
                    debug!(
                        platform = %platform_name,
                        message_id = %msg.id,
                        "Forwarding message to router"
                    );

                    if tx.send(msg).is_err() {
                        warn!("No subscribers for router messages");
                    }
                }
            });
        }

        {
            let mut running = self.running.write().await;
            *running = true;
        }

        info!("All platforms started");
        Ok(())
    }

    /// Stop all registered platforms.
    pub async fn stop_all(&self) -> ChatResult<()> {
        let platforms = self.platforms.read().await;

        for (platform_type, platform) in platforms.iter() {
            info!(platform = %platform_type, "Stopping platform");
            platform.stop().await?;
        }

        {
            let mut running = self.running.write().await;
            *running = false;
        }

        info!("All platforms stopped");
        Ok(())
    }

    /// Subscribe to all incoming messages.
    pub fn subscribe(&self) -> broadcast::Receiver<IncomingMessage> {
        self.tx.subscribe()
    }

    /// Send a response to a specific platform.
    pub async fn send(&self, platform: Platform, message: OutgoingMessage) -> ChatResult<String> {
        let platforms = self.platforms.read().await;

        let platform_client = platforms
            .get(&platform)
            .ok_or_else(|| ChatError::PlatformNotFound(platform.to_string()))?;

        platform_client.send(message).await
    }

    /// Send a reply to an incoming message.
    pub async fn reply(
        &self,
        incoming: &IncomingMessage,
        content: impl Into<String>,
    ) -> ChatResult<String> {
        let reply = OutgoingMessage::reply(incoming, content);
        self.send(incoming.platform, reply).await
    }

    /// Check if all platforms are healthy.
    pub async fn is_healthy(&self) -> bool {
        let platforms = self.platforms.read().await;

        for platform in platforms.values() {
            if !platform.is_healthy().await {
                return false;
            }
        }

        true
    }

    /// Get list of registered platforms.
    pub async fn platforms(&self) -> Vec<Platform> {
        let platforms = self.platforms.read().await;
        platforms.keys().copied().collect()
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_rule_matches() {
        let rule = RoutingRule::new(r"(?i)help|support", "support-agent");

        assert!(rule.matches("I need help"));
        assert!(rule.matches("Contact SUPPORT"));
        assert!(!rule.matches("Hello world"));
    }

    #[test]
    fn test_route_config_builder() {
        let config = RouteConfig::new("default-agent", "tenant-1")
            .with_rule(RoutingRule::new(r"code|programming", "code-agent"))
            .with_rule(RoutingRule::new(r"billing|payment", "billing-agent"));

        assert_eq!(config.default_agent, "default-agent");
        assert_eq!(config.tenant_id, "tenant-1");
        assert_eq!(config.rules.len(), 2);
    }

    #[tokio::test]
    async fn test_router_routing() {
        let router = MessageRouter::new();

        // Configure routes
        router
            .set_route(
                "telegram:123",
                RouteConfig::new("support-agent", "tenant-1")
                    .with_rule(RoutingRule::new(r"(?i)code", "code-agent")),
            )
            .await;

        // Test routing
        let msg1 = IncomingMessage::new(Platform::Telegram, "123", "user1", "Hello");
        let msg2 = IncomingMessage::new(Platform::Telegram, "123", "user1", "Help with code");
        let msg3 = IncomingMessage::new(Platform::Telegram, "999", "user1", "No route");

        assert_eq!(router.route(&msg1).await, Some("support-agent".to_string()));
        assert_eq!(router.route(&msg2).await, Some("code-agent".to_string()));
        assert_eq!(router.route(&msg3).await, None);
    }

    #[tokio::test]
    async fn test_router_wildcard_routes() {
        let router = MessageRouter::new();

        // Configure platform-wide default
        router
            .set_route("telegram:*", RouteConfig::new("telegram-agent", "tenant-1"))
            .await;

        // Configure global default
        router
            .set_route("*:*", RouteConfig::new("global-agent", "tenant-1"))
            .await;

        let telegram_msg = IncomingMessage::new(Platform::Telegram, "any", "user1", "Test");
        let discord_msg = IncomingMessage::new(Platform::Discord, "any", "user1", "Test");

        assert_eq!(
            router.route(&telegram_msg).await,
            Some("telegram-agent".to_string())
        );
        assert_eq!(
            router.route(&discord_msg).await,
            Some("global-agent".to_string())
        );
    }
}
