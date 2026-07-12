//! Recipe locale resolution.
//!
//! A declared Cooklang `locale:` key always wins. Otherwise the language is
//! detected from the recipe's plain text — assembled from the *parsed* recipe so
//! that Cooklang markup and quantities never reach the detector. When detection
//! is unreliable we return `None` rather than storing a guess.

use serde::{Deserialize, Serialize};

use crate::indexer::cooklang_parser::{ParsedRecipeData, StepItem};

/// Below this many characters of plain text there is not enough signal to detect.
const MIN_DETECTION_CHARS: usize = 25;

/// Where a recipe's locale came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocaleSource {
    /// The recipe declared a `locale:` metadata key.
    Declared,
    /// We detected it from the recipe text.
    Detected,
}

impl LocaleSource {
    /// The value stored in the `recipes.locale_source` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            LocaleSource::Declared => "declared",
            LocaleSource::Detected => "detected",
        }
    }
}

/// A resolved locale: a BCP-47-style code (`en`, `de`, `en-US`) and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeLocale {
    pub code: String,
    pub source: LocaleSource,
}

/// Resolve a recipe's locale: declared metadata first, then detection.
pub fn resolve_locale(parsed: &ParsedRecipeData) -> Option<RecipeLocale> {
    if let Some(code) = parsed.metadata.as_ref().and_then(|m| m.locale.clone()) {
        return Some(RecipeLocale {
            code,
            source: LocaleSource::Declared,
        });
    }

    detect_language_code(parsed).map(|code| RecipeLocale {
        code,
        source: LocaleSource::Detected,
    })
}

/// Detect the recipe's language code.
///
/// Narrative prose is the most trustworthy signal. Ingredient names are often
/// culturally-marked proper nouns ("Prosciutto", "Focaccia") that can outvote a short
/// English method, so they only get a say when the prose alone can't decide.
fn detect_language_code(parsed: &ParsedRecipeData) -> Option<String> {
    detect_reliable(&narrative_text(parsed)).or_else(|| detect_reliable(&detection_text(parsed)))
}

/// The recipe's prose: title, description, section names, step text and notes.
/// Quantities, units, cookware and ingredient names are excluded.
fn narrative_text(parsed: &ParsedRecipeData) -> String {
    let mut parts: Vec<&str> = Vec::new();

    if let Some(meta) = &parsed.metadata {
        if let Some(title) = &meta.title {
            parts.push(title);
        }
        if let Some(description) = &meta.description {
            parts.push(description);
        }
    }

    for section in &parsed.sections {
        if let Some(name) = &section.name {
            parts.push(name);
        }
        for step in &section.steps {
            for item in &step.items {
                if let StepItem::Text { value } = item {
                    parts.push(value);
                }
            }
        }
        for note in &section.notes {
            parts.push(note);
        }
    }

    parts.join(" ")
}

/// The recipe's prose plus its ingredient names — the fallback signal for recipes
/// that are ingredient-heavy but say little. Quantities, units and cookware stay
/// excluded: they are noise, not language signal.
pub(crate) fn detection_text(parsed: &ParsedRecipeData) -> String {
    let mut parts = vec![narrative_text(parsed)];

    for ingredient in &parsed.ingredients {
        parts.push(ingredient.name.clone());
    }

    parts.join(" ")
}

/// Detect a language code from plain text, or `None` if we can't trust the result.
fn detect_reliable(text: &str) -> Option<String> {
    if text.chars().count() < MIN_DETECTION_CHARS {
        return None;
    }

    let info = whatlang::detect(text)?;
    if !info.is_reliable() {
        return None;
    }

    Some(to_bcp47(info.lang().code()))
}

/// Map whatlang's ISO 639-3 code to a two-letter code where one exists.
/// Languages without a 639-1 code (e.g. Cebuano) keep their 639-3 code.
///
/// whatlang reports individual-language codes for two macrolanguages, and ISO 639-1
/// only has codes for their macrolanguage parents. isolang can't bridge this, so map
/// them explicitly.
fn to_bcp47(code_639_3: &str) -> String {
    match code_639_3 {
        "cmn" => return "zh".to_string(),
        "pes" => return "fa".to_string(),
        _ => {}
    }

    isolang::Language::from_639_3(code_639_3)
        .and_then(|lang| lang.to_639_1())
        .map(str::to_string)
        .unwrap_or_else(|| code_639_3.to_string())
}

/// English display name for a stored code: `"de"` → `"German"`, `"en-US"` → `"English"`.
/// Falls back to 639-3 so the codes `to_bcp47` leaves as 639-3 (e.g. `"ceb"`) can be named.
pub fn display_name(code: &str) -> Option<String> {
    let language = code.split('-').next()?;

    isolang::Language::from_639_1(language)
        .or_else(|| isolang::Language::from_639_3(language))
        .map(|lang| lang.to_name().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::cooklang_parser::parse_recipe;

    const GERMAN: &str = "Den @Mehl{200%g} und das @Wasser{100%ml} in einer Schüssel \
verrühren, bis ein glatter Teig entsteht. Den Teig ruhen lassen und anschließend \
im #Ofen{} goldbraun backen.";

    const ENGLISH: &str = "Mix the @flour{200%g} and the @water{100%ml} in a bowl until \
a smooth dough forms. Let the dough rest, then bake it in the #oven{} until golden brown.";

    /// Mandarin. whatlang reports this as `Cmn` (ISO 639-3), which has no 639-1 code
    /// of its own — only its macrolanguage parent `zho` does.
    const CHINESE: &str = "将@面粉{200%克}和@水{100%毫升}放入碗中搅拌均匀，揉成光滑的面团。\
让面团静置醒发，然后放入#烤箱{}中烤至金黄色。";

    /// An English method with culturally-marked Italian ingredient names. The prose is
    /// unambiguously English, but the ingredient names alone read as Italian.
    const ENGLISH_WITH_ITALIAN_INGREDIENTS: &str = "Chop and mix everything well, then bake \
it in the #oven{} until browned. Serve the dish hot.

Add @Prosciutto{}, @Parmesan{}, @Chorizo{}, @Bruschetta{}, @Focaccia{}, @Crostini{}, \
@Panzanella{}, @Mozzarella{}, @Gorgonzola{}, @Mascarpone{}.";

    #[test]
    fn test_detects_german() {
        let parsed = parse_recipe(GERMAN).unwrap();
        let locale = resolve_locale(&parsed).expect("should detect a locale");

        assert_eq!(locale.code, "de");
        assert_eq!(locale.source, LocaleSource::Detected);
    }

    #[test]
    fn test_detects_english() {
        let parsed = parse_recipe(ENGLISH).unwrap();
        let locale = resolve_locale(&parsed).expect("should detect a locale");

        assert_eq!(locale.code, "en");
        assert_eq!(locale.source, LocaleSource::Detected);
    }

    #[test]
    fn test_declared_locale_beats_detection() {
        // The body is unmistakably German, but the author declared French.
        let content = format!("---\nlocale: fr\n---\n\n{GERMAN}");
        let parsed = parse_recipe(&content).unwrap();
        let locale = resolve_locale(&parsed).expect("should resolve a locale");

        assert_eq!(locale.code, "fr");
        assert_eq!(locale.source, LocaleSource::Declared);
    }

    #[test]
    fn test_declared_region_is_preserved() {
        let content = format!("---\nlocale: en_US\n---\n\n{ENGLISH}");
        let parsed = parse_recipe(&content).unwrap();
        let locale = resolve_locale(&parsed).unwrap();

        assert_eq!(locale.code, "en-US");
        assert_eq!(locale.source, LocaleSource::Declared);
    }

    #[test]
    fn test_detection_never_invents_a_region() {
        let parsed = parse_recipe(ENGLISH).unwrap();
        let locale = resolve_locale(&parsed).unwrap();

        assert!(
            !locale.code.contains('-'),
            "detected code should be bare: {}",
            locale.code
        );
    }

    #[test]
    fn test_too_short_content_is_not_detected() {
        let parsed = parse_recipe("Mix @salt{}.").unwrap();
        assert!(resolve_locale(&parsed).is_none());
    }

    #[test]
    fn test_non_linguistic_content_is_not_detected() {
        let parsed = parse_recipe("12345 67890 12345 67890 12345 67890 12345").unwrap();
        assert!(resolve_locale(&parsed).is_none());
    }

    #[test]
    fn test_detection_text_excludes_markup() {
        let parsed = parse_recipe(ENGLISH).unwrap();
        let text = detection_text(&parsed);

        assert!(
            text.contains("flour"),
            "ingredient names carry language signal"
        );
        assert!(text.contains("smooth dough"), "step text must be present");
        assert!(
            !text.contains('@'),
            "cooklang markup must be stripped: {text}"
        );
        assert!(
            !text.contains('{'),
            "cooklang markup must be stripped: {text}"
        );
        assert!(
            !text.contains("200"),
            "quantities must not reach the detector: {text}"
        );
    }

    #[test]
    fn test_locale_source_as_str() {
        assert_eq!(LocaleSource::Declared.as_str(), "declared");
        assert_eq!(LocaleSource::Detected.as_str(), "detected");
    }

    #[test]
    fn test_display_name() {
        assert_eq!(display_name("de").as_deref(), Some("German"));
        assert_eq!(display_name("en-US").as_deref(), Some("English"));
        assert_eq!(display_name("zzz"), None);
    }

    #[test]
    fn test_macrolanguage_is_mapped_to_its_639_1_code() {
        // whatlang detects Mandarin as `cmn`, an individual language with no 639-1 code.
        // We must store the macrolanguage code `zh`, not the non-standard `cmn`.
        let parsed = parse_recipe(CHINESE).unwrap();
        let locale = resolve_locale(&parsed).expect("should detect a locale");

        assert_eq!(locale.code, "zh");
        assert_eq!(locale.source, LocaleSource::Detected);
    }

    #[test]
    fn test_display_name_for_language_without_a_639_1_code() {
        // `to_bcp47` keeps the 639-3 code for languages that have no 639-1 code, so
        // `display_name` must be able to name those stored values too.
        assert_eq!(display_name("ceb").as_deref(), Some("Cebuano"));
    }

    #[test]
    fn test_ingredient_names_do_not_outvote_the_narrative() {
        // The method is plainly English; only the ingredient names look Italian. The
        // narrative must win, otherwise an English recipe is filed as Italian.
        let parsed = parse_recipe(ENGLISH_WITH_ITALIAN_INGREDIENTS).unwrap();
        let locale = resolve_locale(&parsed).expect("should detect a locale");

        assert_eq!(locale.code, "en");
    }
}
