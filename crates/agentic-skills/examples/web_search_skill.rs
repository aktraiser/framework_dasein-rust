//! Example: Web Search skill using Tavily API
//!
//! AI-optimized web search that returns clean, relevant content.

use dasein_agentic_skills::{Skill, SkillType};

/// Create the Web Search skill
pub fn web_search_skill() -> Skill {
    Skill::new(
        "web_search",
        "Search the web for current information using Tavily AI search",
    )
    .with_type(SkillType::Prompt)
    .with_instructions(
        r#"
# Web Search via Tavily

AI-optimized web search that returns clean, relevant content.
Requires TAVILY_API_KEY environment variable.

## Basic Search

```bash
# Simple search (5 results)
node scripts/search.mjs "your search query"

# More results (up to 20)
node scripts/search.mjs "your search query" -n 10

# Deep research mode (slower but more comprehensive)
node scripts/search.mjs "your search query" --deep
```

## News Search

```bash
# Search news articles
node scripts/search.mjs "latest AI developments" --topic news

# News from last N days
node scripts/search.mjs "tech news" --topic news --days 7
```

## Extract Content from URL

```bash
# Get clean content from a specific URL
node scripts/extract.mjs "https://example.com/article"
```

## Options Summary

| Option | Description |
|--------|-------------|
| `-n <count>` | Number of results (default: 5, max: 20) |
| `--deep` | Advanced search for deeper research |
| `--topic general` | General web search (default) |
| `--topic news` | News articles only |
| `--days <n>` | For news, limit to last N days |

## When to Use

- **Current events**: Use `--topic news` for recent happenings
- **Research questions**: Use `--deep` for comprehensive answers
- **Fact checking**: Basic search for quick verification
- **Technical docs**: Search then extract for detailed content

## Best Practices

1. Be specific in queries for better results
2. Use `--deep` for complex research questions
3. Use `--topic news` + `--days` for current events
4. Extract full content when snippets aren't enough
5. Tavily returns AI-optimized clean text - no HTML parsing needed
"#,
    )
    .with_trigger([
        "search the web",
        "look up",
        "find information about",
        "what is",
        "who is",
        "search online",
        "google",
        "research",
        "find out about",
        "latest news about",
        "current events",
        "what happened",
        "recent news",
    ])
    .with_required_tools(["bash"])
    .with_tags(["search", "web", "tavily", "research", "news", "internet"])
    .with_version("1.0.0")
}

fn main() {
    let skill = web_search_skill();

    println!("Skill: {}", skill.name);
    println!("Type: {:?}", skill.skill_type);
    println!("Triggers: {:?}", skill.triggers);
    println!("Tags: {:?}", skill.tags);
    println!("\n--- Instructions ---\n{}", skill.instructions);
}
