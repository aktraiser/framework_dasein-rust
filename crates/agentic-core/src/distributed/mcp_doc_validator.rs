//! MCP Documentation Validator.
//!
//! Enriches validation feedback with relevant documentation from MCP servers.
//! Analyzes errors from previous validators and fetches documentation to help fix them.

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, info};

use super::validator_pipeline::{DocSnippet, PipelineValidator, ValidatorInput, ValidatorOutput};

/// Configuration for MCP documentation queries.
#[derive(Debug, Clone)]
pub struct MCPDocConfig {
    /// MCP server URL
    pub url: String,
    /// API key header name
    pub api_key_header: String,
    /// API key value
    pub api_key: String,
    /// Request timeout in ms
    pub timeout_ms: u64,
}

impl MCPDocConfig {
    pub fn context7(api_key: impl Into<String>) -> Self {
        Self {
            url: "https://mcp.context7.com/mcp".to_string(),
            api_key_header: "Authorization".to_string(),
            api_key: format!("Bearer {}", api_key.into()),
            timeout_ms: 30000,
        }
    }
}

/// Validator that fetches documentation for errors.
pub struct MCPDocValidator {
    config: MCPDocConfig,
    client: reqwest::Client,
}

impl MCPDocValidator {
    pub fn new(config: MCPDocConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(config.timeout_ms))
            .build()
            .expect("Failed to create HTTP client");

        Self { config, client }
    }

    /// Extract error topics from previous validation results.
    fn extract_error_topics(&self, input: &ValidatorInput) -> Vec<String> {
        let mut topics = Vec::new();

        for result in &input.previous_results {
            for error in &result.errors {
                // Extract relevant keywords from errors
                if let Some(topic) = self.extract_topic_from_error(error, &input.language) {
                    if !topics.contains(&topic) {
                        topics.push(topic);
                    }
                }
            }
        }

        // Limit to top 3 topics to avoid too many API calls
        topics.truncate(3);
        topics
    }

    /// Extract a searchable topic from an error message.
    fn extract_topic_from_error(&self, error: &str, language: &str) -> Option<String> {
        match language {
            "rust" => self.extract_rust_topic(error),
            "python" => self.extract_python_topic(error),
            "typescript" | "javascript" => self.extract_ts_topic(error),
            _ => None,
        }
    }

    fn extract_rust_topic(&self, error: &str) -> Option<String> {
        // Pattern: E0658 unstable feature - extract the feature name
        if error.contains("E0658") || error.contains("unstable library feature") {
            if let Some(start) = error.find('`') {
                if let Some(end) = error[start + 1..].find('`') {
                    let feature = &error[start + 1..start + 1 + end];
                    // Search for stable alternatives
                    return Some(format!("rust {} stable alternative", feature));
                }
            }
            // Generic search for LRU cache alternatives
            if error.contains("linked_list") {
                return Some("rust lru cache implementation stable".to_string());
            }
        }

        // Pattern: "use of unresolved module or unlinked crate `tokio`"
        if error.contains("unresolved") {
            if let Some(start) = error.find('`') {
                if let Some(end) = error[start + 1..].find('`') {
                    let crate_name = &error[start + 1..start + 1 + end];
                    return Some(format!("{} usage examples", crate_name));
                }
            }
        }

        // Pattern: "error[E0277]: the trait bound `X: Y` is not satisfied"
        if error.contains("trait bound") {
            if let Some(start) = error.find('`') {
                if let Some(end) = error[start + 1..].find('`') {
                    let trait_info = &error[start + 1..start + 1 + end];
                    return Some(format!("rust {}", trait_info));
                }
            }
        }

        // Pattern: "error[E0599]: no method named `foo` found"
        if error.contains("no method named") {
            if let Some(start) = error.find('`') {
                if let Some(end) = error[start + 1..].find('`') {
                    let method = &error[start + 1..start + 1 + end];
                    return Some(format!("rust {} method", method));
                }
            }
        }

        // Pattern: tokio specific
        if error.contains("tokio") {
            if error.contains("Semaphore") {
                return Some("tokio Semaphore acquire permit".to_string());
            }
            if error.contains("JoinSet") || error.contains("JoinHandle") {
                return Some("tokio JoinSet spawn join_next".to_string());
            }
            return Some("tokio async runtime".to_string());
        }

        None
    }

    fn extract_python_topic(&self, error: &str) -> Option<String> {
        if error.contains("ModuleNotFoundError") || error.contains("ImportError") {
            if let Some(start) = error.find('\'') {
                if let Some(end) = error[start + 1..].find('\'') {
                    let module = &error[start + 1..start + 1 + end];
                    return Some(format!("python {} module", module));
                }
            }
        }

        if error.contains("AttributeError") {
            return Some("python AttributeError fix".to_string());
        }

        None
    }

    fn extract_ts_topic(&self, error: &str) -> Option<String> {
        if error.contains("Cannot find module") {
            if let Some(start) = error.find('\'') {
                if let Some(end) = error[start + 1..].find('\'') {
                    let module = &error[start + 1..start + 1 + end];
                    return Some(format!("npm {}", module));
                }
            }
        }

        None
    }

    /// Query MCP server for documentation.
    async fn query_docs(&self, library_id: &str, query: &str) -> Result<String, String> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "query-docs",
                "arguments": {
                    "libraryId": library_id,
                    "query": query
                }
            }
        });

        debug!("Querying MCP docs: {} - {}", library_id, query);

        let response = self
            .client
            .post(&self.config.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header(&self.config.api_key_header, &self.config.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let json: Value = response
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        // Extract content from response
        if let Some(error) = json.get("error") {
            return Err(format!("MCP error: {}", error));
        }

        // Navigate to result.content[0].text
        let content = json["result"]["content"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|item| item["text"].as_str())
            .unwrap_or("No documentation found");

        Ok(content.to_string())
    }

    /// Resolve library ID for a query.
    async fn resolve_library_id(&self, query: &str) -> Result<String, String> {
        // Extract library name from query
        let library_name = if query.contains("tokio") {
            "tokio"
        } else if query.contains("serde") {
            "serde"
        } else if query.contains("async") || query.contains("futures") {
            "futures"
        } else {
            // Try to extract first word as library name
            query.split_whitespace().next().unwrap_or("rust")
        };

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "resolve-library-id",
                "arguments": {
                    "libraryName": library_name,
                    "query": query
                }
            }
        });

        let response = self
            .client
            .post(&self.config.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header(&self.config.api_key_header, &self.config.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let json: Value = response
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        // Extract library ID from response
        let content = json["result"]["content"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|item| item["text"].as_str())
            .unwrap_or("");

        // Parse library ID from response (usually in format /org/project)
        if let Some(id_start) = content.find('/') {
            let id_part = &content[id_start..];
            if let Some(id_end) = id_part.find(|c: char| c.is_whitespace() || c == '\n') {
                return Ok(id_part[..id_end].to_string());
            }
            // Take until end of line or reasonable length
            let id: String = id_part.chars().take(50).collect();
            if let Some(end) = id.find('\n') {
                return Ok(id[..end].to_string());
            }
            return Ok(id);
        }

        // Fallback to common library IDs
        Ok(match library_name {
            "tokio" => "/tokio-rs/tokio".to_string(),
            "serde" => "/serde-rs/serde".to_string(),
            "futures" => "/rust-lang/futures-rs".to_string(),
            _ => format!("/rust/{}", library_name),
        })
    }
}

#[async_trait]
impl PipelineValidator for MCPDocValidator {
    fn name(&self) -> &str {
        "mcp-doc"
    }

    async fn validate(&self, input: &ValidatorInput) -> Result<ValidatorOutput, String> {
        // Check if there are errors from previous validators
        let has_errors = input.previous_results.iter().any(|r| !r.passed);

        if !has_errors {
            // No errors, no need to fetch docs
            return Ok(ValidatorOutput::success("mcp-doc"));
        }

        // Extract topics from errors
        let topics = self.extract_error_topics(input);

        if topics.is_empty() {
            info!("No extractable topics from errors");
            return Ok(
                ValidatorOutput::success("mcp-doc").with_recommendations(vec![
                    "Could not extract specific topics from errors".to_string(),
                ]),
            );
        }

        info!("Fetching docs for {} topics: {:?}", topics.len(), topics);

        let mut docs = Vec::new();
        let mut recommendations = Vec::new();

        for topic in topics {
            // Resolve library ID
            match self.resolve_library_id(&topic).await {
                Ok(library_id) => {
                    // Query documentation
                    match self.query_docs(&library_id, &topic).await {
                        Ok(content) => {
                            docs.push(DocSnippet {
                                source: "context7".to_string(),
                                topic: topic.clone(),
                                content,
                            });
                            recommendations
                                .push(format!("See {} documentation for {}", library_id, topic));
                        }
                        Err(e) => {
                            debug!("Failed to query docs for {}: {}", topic, e);
                        }
                    }
                }
                Err(e) => {
                    debug!("Failed to resolve library ID for {}: {}", topic, e);
                }
            }
        }

        Ok(ValidatorOutput::success("mcp-doc")
            .with_documentation(docs)
            .with_recommendations(recommendations))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_rust_topic_unresolved() {
        let config = MCPDocConfig::context7("test");
        let validator = MCPDocValidator::new(config);

        let error =
            "error[E0433]: failed to resolve: use of unresolved module or unlinked crate `tokio`";
        let topic = validator.extract_rust_topic(error);

        assert!(topic.is_some());
        assert!(topic.unwrap().contains("tokio"));
    }

    #[test]
    fn test_extract_rust_topic_trait_bound() {
        let config = MCPDocConfig::context7("test");
        let validator = MCPDocValidator::new(config);

        let error = "error[E0277]: the trait bound `Foo: Clone` is not satisfied";
        let topic = validator.extract_rust_topic(error);

        assert!(topic.is_some());
        assert!(topic.unwrap().contains("Foo: Clone"));
    }
}
