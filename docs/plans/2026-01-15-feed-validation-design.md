# Feed Validation Feature Design

## Overview

Add feed validation capability in two places:
- **CLI**: `federation validate <url>` command
- **Web**: `/validate` page with URL input form

Both use format-only validation (valid RSS/Atom XML parsing) via the existing `feed-rs` library.

## CLI Command

```
federation validate <url>
```

**Success output**:
```
✓ Valid feed: "Feed Title"
  Type: Atom/RSS 2.0
  Entries: 42
```

**Failure output**:
```
✗ Invalid feed: <error message>
```

## Web Page (`/validate`)

**URL**: `/validate`

**UI Flow**:
1. Form with single URL input field and "Validate" button
2. On submit, POST to `/validate`
3. Display result on same page

**Success display**:
```
✓ Valid feed
Title: "Feed Title"
Type: Atom 1.0
Entries: 42
Sample entries:
  - "Recipe One"
  - "Recipe Two"
  - "Recipe Three"
```

**Failure display**:
```
✗ Invalid feed
Error: <parser error message>
```

## Implementation

### New/Modified Files

1. `src/cli/mod.rs` - Add `Validate { url: String }` variant to CLI enum
2. `src/cli/commands.rs` - Add `validate_feed()` function
3. `src/api/routes.rs` - Add `/validate` GET and POST routes
4. `src/web/handlers.rs` - Add `validate_page()` and `validate_submit()` handlers
5. `src/web/templates/validate.html` - New template for the page

### Shared Logic

Create `src/utils/feed_validation.rs` with:

```rust
pub struct FeedInfo {
    pub title: String,
    pub feed_type: String,
    pub entry_count: usize,
    pub sample_entries: Vec<String>,
}

pub async fn validate_feed_url(url: &str) -> Result<FeedInfo, Error>
```

Both CLI and web handlers call this shared function. Uses existing `feed-rs` crate and URL validation from `src/utils/validation.rs`.
