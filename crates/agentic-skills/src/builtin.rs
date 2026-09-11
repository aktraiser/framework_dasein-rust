//! Built-in skills for common tasks.
//!
//! These skills can be registered directly into a SkillRegistry.

use crate::types::{Skill, SkillType};

/// Get all built-in skills.
pub fn all_skills() -> Vec<Skill> {
    vec![google_workspace(), web_search(), code_review()]
}

/// Google Workspace skill (Gmail, Calendar, Drive, Sheets, Docs).
pub fn google_workspace() -> Skill {
    Skill::new(
        "google_workspace",
        "Manage Gmail, Calendar, Drive, Contacts, Sheets and Docs via gog CLI",
    )
    .with_type(SkillType::Prompt)
    .with_instructions(
        r#"
# Google Workspace via gog CLI

Use `gog` for Gmail, Calendar, Drive, Contacts, Sheets, Docs.
Set GOG_ACCOUNT environment variable or use --account flag.

## Gmail
```bash
gog gmail search 'newer_than:7d' --max 10 --json
gog gmail search 'from:someone@example.com' --max 5 --json
gog gmail send --to recipient@example.com --subject "Subject" --body "Message"
gog gmail thread <threadId> --json
```

## Calendar
```bash
gog calendar events primary --from 2024-01-01T00:00:00Z --to 2024-01-31T23:59:59Z --json
gog calendar create <calendarId> --summary "Meeting" --start <iso> --end <iso>
```

## Drive
```bash
gog drive search "name contains 'report'" --max 10 --json
gog drive download <fileId> --out /tmp/file.pdf
gog drive upload /path/to/file.txt --parent <folderId>
```

## Contacts
```bash
gog contacts list --max 20 --json
gog contacts search "John" --max 10 --json
```

## Sheets
```bash
gog sheets get <sheetId> "Sheet!A1:D10" --json
gog sheets update <sheetId> "Sheet!A1:B2" --values-json '[["A","B"],["1","2"]]' --input USER_ENTERED
gog sheets append <sheetId> "Sheet!A:C" --values-json '[["x","y","z"]]' --insert INSERT_ROWS
gog sheets clear <sheetId> "Sheet!A2:Z"
gog sheets metadata <sheetId> --json
```

## Docs
```bash
gog docs cat <docId>
gog docs export <docId> --format txt --out /tmp/doc.txt
```

**Notes**: Use --json for parsing. Gmail search doesn't handle accents well.
"#,
    )
    .with_trigger([
        "send email",
        "check my emails",
        "search gmail",
        "read my inbox",
        "create calendar event",
        "what's on my calendar",
        "schedule meeting",
        "search drive",
        "find file in drive",
        "update spreadsheet",
        "read google sheet",
        "list contacts",
        "find contact",
        "read google doc",
    ])
    .with_required_tools(["bash"])
    .with_tags([
        "google", "gmail", "calendar", "drive", "sheets", "docs", "gog", "email",
    ])
}

/// Web search skill using Tavily API.
pub fn web_search() -> Skill {
    Skill::new(
        "web_search",
        "Search the web for current information using Tavily AI search",
    )
    .with_type(SkillType::Prompt)
    .with_instructions(
        r#"
# Web Search via Tavily

AI-optimized web search. Requires TAVILY_API_KEY.

## Search
```bash
node scripts/search.mjs "query"              # Basic (5 results)
node scripts/search.mjs "query" -n 10        # More results
node scripts/search.mjs "query" --deep       # Deep research
node scripts/search.mjs "query" --topic news # News only
node scripts/search.mjs "query" --topic news --days 7  # Recent news
```

## Extract URL Content
```bash
node scripts/extract.mjs "https://example.com/article"
```

## Options
- `-n <count>`: Results count (max 20)
- `--deep`: Comprehensive research mode
- `--topic news`: News articles only
- `--days <n>`: Limit to last N days

**Tips**: Use --deep for complex questions, --topic news for current events.
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
}

/// Code review skill.
pub fn code_review() -> Skill {
    Skill::new(
        "code_review",
        "Review code for quality, bugs, security, and best practices",
    )
    .with_type(SkillType::Prompt)
    .with_instructions(
        r#"
# Code Review

Analyze code for quality, bugs, security issues, and best practices.

## Review Checklist

### 1. Correctness
- Logic errors and edge cases
- Off-by-one errors
- Null/undefined handling
- Error handling completeness

### 2. Security
- Input validation
- SQL injection, XSS, CSRF
- Authentication/authorization
- Sensitive data exposure
- Dependency vulnerabilities

### 3. Performance
- Unnecessary computations
- N+1 queries
- Memory leaks
- Blocking operations in async code

### 4. Maintainability
- Code clarity and naming
- DRY violations
- Function length and complexity
- Documentation

### 5. Best Practices
- Language idioms
- Framework conventions
- Testing coverage
- Error messages

## Output Format

For each issue found:
1. **Location**: File and line number
2. **Severity**: Critical / High / Medium / Low
3. **Issue**: What's wrong
4. **Fix**: How to fix it
"#,
    )
    .with_trigger([
        "review my code",
        "check this code",
        "code review",
        "find bugs",
        "security review",
        "audit this code",
        "is this code good",
        "improve this code",
    ])
    .with_tags(["code", "review", "quality", "security", "bugs"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_skills() {
        let skills = all_skills();
        assert_eq!(skills.len(), 3);
        assert!(skills.iter().any(|s| s.name == "google_workspace"));
        assert!(skills.iter().any(|s| s.name == "web_search"));
        assert!(skills.iter().any(|s| s.name == "code_review"));
    }
}
