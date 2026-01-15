use crate::utils::feed_validation::validate_feed_url;
use crate::{Error, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;
use tracing::warn;

/// Search for recipes
pub async fn search(
    server_url: &str,
    query: &str,
    tags: Option<String>,
    max_time: Option<i64>,
) -> Result<()> {
    let client = Client::new();

    // Build query params
    let mut url = format!("{}/api/search?q={}", server_url, urlencoding::encode(query));

    if let Some(tags) = tags {
        url.push_str(&format!("&tags={}", urlencoding::encode(&tags)));
    }

    if let Some(max_time) = max_time {
        url.push_str(&format!("&max_time={max_time}"));
    }

    // Make request
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(Error::Http(response.error_for_status().unwrap_err()));
    }

    let search_results: SearchResponse = response.json().await?;

    // Display results
    print_search_results(&search_results);

    Ok(())
}

/// Download a recipe by ID
pub async fn download(server_url: &str, recipe_id: i64, output_dir: Option<String>) -> Result<()> {
    let client = Client::new();

    // Get recipe details first
    let recipe_url = format!("{server_url}/api/recipes/{recipe_id}");
    let recipe_response = client.get(&recipe_url).send().await?;

    if !recipe_response.status().is_success() {
        return Err(Error::NotFound(format!("Recipe {recipe_id} not found")));
    }

    let recipe: RecipeDetail = recipe_response.json().await?;

    // Download the .cook file
    let download_url = format!("{server_url}/api/recipes/{recipe_id}/download");
    let download_response = client.get(&download_url).send().await?;

    if !download_response.status().is_success() {
        return Err(Error::Internal("Failed to download recipe".to_string()));
    }

    let content = download_response.text().await?;

    // Sanitize filename to prevent path traversal
    let safe_filename = sanitize_filename(&recipe.title);
    if safe_filename.is_empty() {
        return Err(Error::Validation(
            "Invalid recipe title for filename".to_string(),
        ));
    }

    // Determine and validate output path
    let output_path = if let Some(dir) = output_dir {
        let validated_dir = validate_output_dir(&dir)?;
        std::fs::create_dir_all(&validated_dir)?;
        validated_dir.join(format!("{safe_filename}.cook"))
    } else {
        std::path::PathBuf::from(format!("{safe_filename}.cook"))
    };

    // Write to file
    std::fs::write(&output_path, content)?;

    println!("✓ Downloaded: {}", output_path.display());
    println!("  Title: {}", recipe.title);
    if let Some(summary) = &recipe.summary {
        if !summary.is_empty() {
            println!("  Summary: {summary}");
        }
    }

    Ok(())
}

/// Publish recipes from a directory as an Atom feed
pub async fn publish(
    input_dir: &str,
    output_file: &str,
    author_name: Option<String>,
    feed_title: Option<String>,
) -> Result<()> {
    use std::fs;

    let input_path = Path::new(input_dir);

    if !input_path.exists() || !input_path.is_dir() {
        return Err(Error::Validation(format!(
            "Directory not found: {input_dir}"
        )));
    }

    // Find all .cook files
    let mut recipes = Vec::new();

    for entry in fs::read_dir(input_path)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("cook") {
            let content = fs::read_to_string(&path)?;
            let filename = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            recipes.push((filename, content));
        }
    }

    if recipes.is_empty() {
        return Err(Error::Validation(
            "No .cook files found in directory".to_string(),
        ));
    }

    // Generate Atom feed
    let feed_xml = generate_atom_feed(
        &recipes,
        &author_name.unwrap_or_else(|| "Unknown".to_string()),
        &feed_title.unwrap_or_else(|| "Recipe Collection".to_string()),
        input_dir,
    );

    // Write feed file
    fs::write(output_file, feed_xml)?;

    println!("✓ Generated feed: {output_file}");
    println!("  Recipes: {}", recipes.len());
    println!("\nTo publish:");
    println!("  1. Host this feed at a public URL");
    println!("  2. Add the feed URL to a federation server");

    Ok(())
}

// Helper functions

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Validate and canonicalize output directory path
fn validate_output_dir(dir: &str) -> Result<std::path::PathBuf> {
    let path = std::path::PathBuf::from(dir);

    // Check for path traversal attempts
    if dir.contains("..") {
        warn!(
            "Security: Blocked path traversal attempt in directory: {}",
            dir
        );
        return Err(Error::Validation("Path traversal not allowed".to_string()));
    }

    // Canonicalize the path to resolve any symbolic links or relative paths
    let canonical_path = if path.exists() {
        path.canonicalize()
            .map_err(|e| Error::Internal(format!("Failed to canonicalize path: {e}")))?
    } else {
        path
    };

    Ok(canonical_path)
}

fn print_search_results(results: &SearchResponse) {
    if results.results.is_empty() {
        println!("No recipes found");
        return;
    }

    println!("\nFound {} recipes:\n", results.pagination.total);
    println!("{:<5} {:<50} {:<20}", "ID", "Title", "Tags");
    println!("{}", "-".repeat(75));

    for recipe in &results.results {
        let tags = recipe.tags.join(", ");

        println!(
            "{:<5} {:<50} {:<20}",
            recipe.id,
            truncate(&recipe.title, 48),
            truncate(&tags, 18)
        );
    }

    println!(
        "\nPage {} of {}",
        results.pagination.page, results.pagination.total_pages
    );
    println!("\nTo download a recipe: federation download <ID>");
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

fn generate_atom_feed(
    recipes: &[(String, String)],
    author: &str,
    title: &str,
    base_url: &str,
) -> String {
    use chrono::Utc;

    let now = Utc::now().to_rfc3339();

    let mut feed = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>{}</title>
  <link href="{}/feed.xml" rel="self"/>
  <updated>{}</updated>
  <author>
    <name>{}</name>
  </author>
  <id>urn:uuid:{}</id>
"#,
        escape_xml(title),
        escape_xml(base_url),
        now,
        escape_xml(author),
        uuid::Uuid::new_v4()
    );

    for (filename, content) in recipes {
        // Parse recipe metadata
        let (recipe_title, tags, summary) = parse_recipe_metadata(content, filename);

        feed.push_str(&format!(
            r#"
  <entry>
    <id>recipe-{}</id>
    <title>{}</title>
    <link href="{}/{}.cook" rel="enclosure"/>
    <updated>{}</updated>
    <summary>{}</summary>
"#,
            filename,
            escape_xml(&recipe_title),
            escape_xml(base_url),
            filename,
            now,
            escape_xml(&summary)
        ));

        for tag in tags {
            feed.push_str(&format!("    <category term=\"{}\"/>\n", escape_xml(&tag)));
        }

        feed.push_str("  </entry>\n");
    }

    feed.push_str("</feed>\n");
    feed
}

fn parse_recipe_metadata(content: &str, filename: &str) -> (String, Vec<String>, String) {
    let mut title = filename.replace(['-', '_'], " ");
    let mut tags = Vec::new();
    let mut summary = String::new();

    for line in content.lines() {
        if let Some(meta) = line.strip_prefix(">>") {
            let meta = meta.trim();
            if let Some((key, value)) = meta.split_once(':') {
                let key = key.trim().to_lowercase();
                let value = value.trim();

                match key.as_str() {
                    "title" => title = value.to_string(),
                    "tags" => {
                        tags = value
                            .split(',')
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .collect();
                    }
                    "summary" | "description" => summary = value.to_string(),
                    _ => {}
                }
            }
        }
    }

    if summary.is_empty() {
        summary = format!("Recipe: {title}");
    }

    (title, tags, summary)
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Validate a feed URL
pub async fn validate_feed(url: &str) -> Result<()> {
    match validate_feed_url(url).await {
        Ok(info) => {
            println!("\x1b[32m\u{2713}\x1b[0m Valid feed: \"{}\"", info.title);
            println!("  Type: {}", info.feed_type);
            println!("  Entries: {}", info.entry_count);

            if !info.sample_entries.is_empty() {
                println!("  Sample entries:");
                for entry in &info.sample_entries {
                    println!("    - {}", entry);
                }
            }

            Ok(())
        }
        Err(e) => {
            println!("\x1b[31m\u{2717}\x1b[0m Invalid feed: {}", e);
            Err(e)
        }
    }
}

// Response types (matching API models)

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<RecipeCard>,
    pagination: Pagination,
}

#[derive(Debug, Deserialize)]
struct RecipeCard {
    id: i64,
    title: String,
    #[serde(rename = "summary")]
    _summary: Option<String>,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RecipeDetail {
    #[serde(rename = "id")]
    _id: i64,
    title: String,
    summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Pagination {
    page: usize,
    #[serde(rename = "limit")]
    _limit: usize,
    total: usize,
    total_pages: usize,
}
