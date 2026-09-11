//! Example: Creating a Google Workspace skill
//!
//! This demonstrates how to create and register a skill for Gmail/Calendar/Drive
//! using the gog CLI tool.

use dasein_agentic_skills::{Skill, SkillType};

/// Create the Google Workspace skill
pub fn google_workspace_skill() -> Skill {
    Skill::new("google_workspace", "Manage Gmail, Calendar, Drive, Contacts, Sheets and Docs")
        .with_type(SkillType::Prompt)
        .with_instructions(r#"
# Google Workspace via gog CLI

Use `gog` for Gmail, Calendar, Drive, Contacts, Sheets, Docs.
Requires OAuth setup with GOG_ACCOUNT environment variable.

## Gmail

### Search emails
```bash
gog gmail search 'newer_than:7d' --max 10 --json
gog gmail search 'from:someone@example.com' --max 5 --json
gog gmail search 'subject:invoice' --max 10 --json
```

### Send email
```bash
gog gmail send --to recipient@example.com --subject "Subject" --body "Message body"
```

### Read thread
```bash
gog gmail thread <threadId> --json
```

## Calendar

### List events
```bash
gog calendar events <calendarId> --from <iso> --to <iso> --json
gog calendar events primary --from 2024-01-01T00:00:00Z --to 2024-01-31T23:59:59Z --json
```

### Create event
```bash
gog calendar create <calendarId> --summary "Meeting" --start <iso> --end <iso>
```

## Drive

### Search files
```bash
gog drive search "name contains 'report'" --max 10 --json
```

### Download/Upload
```bash
gog drive download <fileId> --out /tmp/file.pdf
gog drive upload /path/to/file.txt --parent <folderId>
```

## Contacts

### List contacts
```bash
gog contacts list --max 20 --json
```

### Search contacts
```bash
gog contacts search "John" --max 10 --json
```

## Google Sheets

### Read data
```bash
gog sheets get <sheetId> "SheetName!A1:D10" --json
```

### Write data
```bash
gog sheets update <sheetId> "SheetName!A1:B2" --values-json '[["A","B"],["1","2"]]' --input USER_ENTERED
```

### Append rows
```bash
gog sheets append <sheetId> "SheetName!A:C" --values-json '[["x","y","z"]]' --insert INSERT_ROWS
```

### Clear range
```bash
gog sheets clear <sheetId> "SheetName!A2:Z"
```

### Get metadata
```bash
gog sheets metadata <sheetId> --json
```

## Google Docs

### Read document
```bash
gog docs cat <docId>
gog docs export <docId> --format txt --out /tmp/doc.txt
```

## Important Notes

1. Always use `--json` for parsing output programmatically
2. Use `--no-input` for non-interactive scripts
3. Gmail search does NOT handle accented characters well - use ASCII
4. Confirm with user before sending emails or creating events
5. GOG_ACCOUNT must be set or use --account flag
"#)
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
        .with_tags(["google", "gmail", "calendar", "drive", "sheets", "docs", "gog", "email"])
        .with_version("1.0.0")
}

fn main() {
    let skill = google_workspace_skill();

    println!("Skill: {}", skill.name);
    println!("Type: {:?}", skill.skill_type);
    println!("Triggers: {:?}", skill.triggers);
    println!("Tags: {:?}", skill.tags);
    println!("\n--- Instructions ---\n{}", skill.instructions);
}
