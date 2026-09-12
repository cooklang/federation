//! Turns Cooklang source into the plain prose a searcher expects to match.
//!
//! Raw `.cook` files carry markup (`@olive oil{2%tbsp}`, `#bowl{}`, `~{1%minute}`),
//! comments and YAML frontmatter. Indexing that verbatim pollutes the term
//! dictionary with units, quantities and metadata keys, and splits multi-word
//! ingredient names in odd places. We index the rendered text instead.

use crate::indexer::cooklang_parser::{parse_recipe, StepItem};

/// The searchable prose of a recipe: step text with ingredient and cookware
/// names inlined, section names, notes and the description. Quantities, units,
/// timers, comments and frontmatter are dropped.
pub fn instructions_text(content: &str) -> String {
    match parse_recipe(content) {
        Ok(parsed) => {
            let mut out = String::new();

            if let Some(description) = parsed
                .metadata
                .as_ref()
                .and_then(|m| m.description.as_ref())
            {
                push_line(&mut out, description);
            }

            for section in &parsed.sections {
                if let Some(name) = &section.name {
                    push_line(&mut out, name);
                }
                for step in &section.steps {
                    let mut line = String::new();
                    for item in &step.items {
                        match item {
                            StepItem::Text { value } => line.push_str(value),
                            StepItem::Ingredient { name, .. } | StepItem::Cookware { name, .. } => {
                                line.push_str(name)
                            }
                            StepItem::Timer { .. } | StepItem::Quantity { .. } => {}
                        }
                    }
                    push_line(&mut out, &line);
                }
                for note in &section.notes {
                    push_line(&mut out, note);
                }
            }

            out
        }
        Err(_) => strip_markup(content),
    }
}

fn push_line(out: &mut String, text: &str) {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&text);
}

/// Best-effort fallback for content the parser rejects: drop frontmatter,
/// comments, `{...}` blocks and the `@`/`#`/`~` markers.
fn strip_markup(content: &str) -> String {
    let body = content
        .strip_prefix("---")
        .and_then(|rest| rest.find("\n---").map(|end| &rest[end + 4..]))
        .unwrap_or(content);

    let mut out = String::new();
    let mut in_block_comment = false;
    for line in body.lines() {
        let mut line = line.to_string();
        if in_block_comment {
            match line.find("-]") {
                Some(end) => {
                    line = line[end + 2..].to_string();
                    in_block_comment = false;
                }
                None => continue,
            }
        }
        while let Some(start) = line.find("[-") {
            match line[start..].find("-]") {
                Some(len) => line.replace_range(start..start + len + 2, " "),
                None => {
                    line.truncate(start);
                    in_block_comment = true;
                }
            }
        }
        if let Some(comment) = line.find("--") {
            line.truncate(comment);
        }
        while let Some(start) = line.find('{') {
            match line[start..].find('}') {
                Some(len) => line.replace_range(start..start + len + 1, " "),
                None => line.truncate(start),
            }
        }
        let cleaned: String = line
            .chars()
            .filter(|c| !matches!(c, '@' | '#' | '~'))
            .collect();
        push_line(&mut out, &cleaned);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_steps_with_ingredient_names_and_without_quantities() {
        let text = instructions_text(
            "---\ntitle: Dressing\n---\nWhisk @olive oil{2%tbsp} in a #bowl{} for ~{1%minute}.\n",
        );
        assert_eq!(text, "Whisk olive oil in a bowl for .");
    }

    #[test]
    fn drops_comments_and_frontmatter_keys() {
        let text =
            instructions_text("---\nservings: 4\n---\nStir. -- secret\n[- hidden -]\nServe.\n");
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("hidden"), "{text}");
        assert!(!text.contains("servings"), "{text}");
        assert!(text.contains("Stir."), "{text}");
        assert!(text.contains("Serve."), "{text}");
    }

    #[test]
    fn fallback_strips_markup_when_parser_fails() {
        let text = strip_markup(
            "---\ntitle: X\n---\nAdd @sugar{1%cup} -- sweet\n[- note -]\nMix #pan{}.\n",
        );
        assert_eq!(text, "Add sugar\nMix pan .");
    }
}
