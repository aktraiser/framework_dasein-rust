//! Web search tools.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::error::ToolError;
use crate::traits::{ToolExecutor, ToolResult};
use dasein_agentic_llm::Tool;

/// A search result from Tavily.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Result title.
    pub title: String,
    /// URL of the result.
    pub url: String,
    /// Content snippet.
    pub content: String,
    /// Thumbnail image URL (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// Tavily web search tool.
///
/// Uses the Tavily API to search the web for current information.
///
/// ## Example
///
/// ```rust,no_run
/// use dasein_agentic_tools::{ToolRegistry, TavilySearch};
///
/// # async fn example() {
/// let mut registry = ToolRegistry::new();
/// registry.register(TavilySearch::new("your-api-key"));
///
/// let result = registry.execute("web_search", &serde_json::json!({
///     "query": "latest AI news"
/// })).await;
/// # }
/// ```
pub struct TavilySearch {
    api_key: String,
    max_results: usize,
}

impl TavilySearch {
    /// Create a new Tavily search tool.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            max_results: 5,
        }
    }

    /// Set the maximum number of results.
    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    /// Perform the search and return structured results.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>, ToolError> {
        let resp = reqwest::Client::new()
            .post("https://api.tavily.com/search")
            .json(&serde_json::json!({
                "api_key": self.api_key,
                "query": query,
                "max_results": self.max_results
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(ToolError::ServiceError(format!(
                "Tavily API error: {}",
                resp.status()
            )));
        }

        let data: Value = resp.json().await?;
        let mut results = Vec::new();

        if let Some(arr) = data["results"].as_array() {
            for r in arr.iter().take(self.max_results) {
                results.push(SearchResult {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    url: r["url"].as_str().unwrap_or("").to_string(),
                    content: r["content"].as_str().unwrap_or("").to_string(),
                    image: r["image"].as_str().map(String::from),
                });
            }
        }

        Ok(results)
    }
}

#[async_trait]
impl ToolExecutor for TavilySearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn definition(&self) -> Tool {
        Tool::new("web_search", "Search the web for current information")
            .with_string_param("query", "The search query")
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult, ToolError> {
        let query = arguments["query"]
            .as_str()
            .ok_or_else(|| ToolError::MissingArgument("query".to_string()))?;

        debug!(query = %query, "Searching with Tavily");

        let results = self.search(query).await?;

        // Format text for LLM
        let mut text = String::new();
        for r in &results {
            text.push_str(&format!("- {} ({})\n  {}\n\n", r.title, r.url, r.content));
        }

        if text.is_empty() {
            text = "No results found.".to_string();
        }

        // Include structured data for rich display
        let data = serde_json::to_value(&results)?;

        Ok(ToolResult::ok_with_data(text, data))
    }
}
