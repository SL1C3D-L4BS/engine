//! Locale identifiers, text direction, and the locale registry.

use std::fmt;

/// A BCP-47-style locale identifier: a language and an optional region, such
/// as `en-US`, `pt-BR`, or bare `fr`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocaleId {
    language: String,
    region: Option<String>,
}

impl LocaleId {
    /// Builds a locale from a language subtag and an optional region subtag.
    ///
    /// The language is lowercased and the region uppercased, so callers need
    /// not pre-normalize.
    pub fn new(language: impl AsRef<str>, region: Option<&str>) -> Self {
        Self {
            language: language.as_ref().to_ascii_lowercase(),
            region: region.map(|r| r.to_ascii_uppercase()),
        }
    }

    /// Parses a `language` or `language-region` tag (also accepting `_`).
    ///
    /// Returns `None` if the language subtag is empty or non-alphabetic.
    pub fn parse(tag: &str) -> Option<Self> {
        let mut parts = tag.split(['-', '_']);
        let language = parts.next()?;
        if language.is_empty() || !language.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        let region = parts.next().filter(|r| !r.is_empty());
        Some(Self::new(language, region))
    }

    /// The language subtag, lowercased.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// The region subtag, uppercased, if any.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// The next-broader locale: a region-qualified locale falls back to the
    /// bare language, which has no further fallback.
    pub fn fallback(&self) -> Option<LocaleId> {
        self.region.as_ref().map(|_| Self {
            language: self.language.clone(),
            region: None,
        })
    }

    /// The writing direction of this locale's script.
    pub fn direction(&self) -> Direction {
        // The right-to-left languages the engine ships UI for.
        match self.language.as_str() {
            "ar" | "he" | "fa" | "ur" | "ps" | "syr" => Direction::RightToLeft,
            _ => Direction::LeftToRight,
        }
    }
}

impl fmt::Display for LocaleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.region {
            Some(region) => write!(f, "{}-{region}", self.language),
            None => write!(f, "{}", self.language),
        }
    }
}

/// Writing direction — a runtime property the UI layer reads to mirror layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Left-to-right (Latin, Cyrillic, CJK, …).
    LeftToRight,
    /// Right-to-left (Arabic, Hebrew, …).
    RightToLeft,
}

/// Tracks the active locale and derives the fallback chain used for lookups.
#[derive(Clone, Debug)]
pub struct LocaleRegistry {
    default: LocaleId,
    current: LocaleId,
}

impl LocaleRegistry {
    /// Creates a registry whose default and current locale is `default`
    /// (conventionally `en-US`).
    pub fn new(default: LocaleId) -> Self {
        Self {
            current: default.clone(),
            default,
        }
    }

    /// The locale all lookups ultimately fall back to.
    pub fn default_locale(&self) -> &LocaleId {
        &self.default
    }

    /// The currently selected locale.
    pub fn current(&self) -> &LocaleId {
        &self.current
    }

    /// Switches the active locale.
    pub fn set_locale(&mut self, locale: LocaleId) {
        self.current = locale;
    }

    /// The ordered, de-duplicated lookup chain: the current locale, its bare
    /// language, then the default locale and its bare language.
    pub fn fallback_chain(&self) -> Vec<LocaleId> {
        let mut chain = Vec::new();
        let mut push = |locale: LocaleId| {
            if !chain.contains(&locale) {
                chain.push(locale);
            }
        };
        push(self.current.clone());
        if let Some(broader) = self.current.fallback() {
            push(broader);
        }
        push(self.default.clone());
        if let Some(broader) = self.default.fallback() {
            push(broader);
        }
        chain
    }
}

impl Default for LocaleRegistry {
    fn default() -> Self {
        Self::new(LocaleId::new("en", Some("US")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_normalizes_subtags() {
        let loc = LocaleId::parse("EN_us").unwrap();
        assert_eq!(loc.language(), "en");
        assert_eq!(loc.region(), Some("US"));
        assert_eq!(loc.to_string(), "en-US");

        let bare = LocaleId::parse("fr").unwrap();
        assert_eq!(bare.region(), None);
        assert_eq!(bare.to_string(), "fr");

        assert!(LocaleId::parse("").is_none());
        assert!(LocaleId::parse("12").is_none());
    }

    #[test]
    fn region_falls_back_to_language() {
        let loc = LocaleId::parse("pt-BR").unwrap();
        assert_eq!(loc.fallback(), Some(LocaleId::new("pt", None)));
        assert_eq!(loc.fallback().unwrap().fallback(), None);
    }

    #[test]
    fn direction_is_a_runtime_property() {
        assert_eq!(
            LocaleId::parse("ar-EG").unwrap().direction(),
            Direction::RightToLeft
        );
        assert_eq!(
            LocaleId::parse("en-US").unwrap().direction(),
            Direction::LeftToRight
        );
    }

    #[test]
    fn fallback_chain_is_ordered_and_deduplicated() {
        let mut reg = LocaleRegistry::default(); // en-US
        reg.set_locale(LocaleId::parse("fr-CA").unwrap());
        assert_eq!(
            reg.fallback_chain(),
            vec![
                LocaleId::parse("fr-CA").unwrap(),
                LocaleId::parse("fr").unwrap(),
                LocaleId::parse("en-US").unwrap(),
                LocaleId::parse("en").unwrap(),
            ]
        );

        // When current == default the chain does not repeat entries.
        reg.set_locale(LocaleId::parse("en-US").unwrap());
        assert_eq!(
            reg.fallback_chain(),
            vec![
                LocaleId::parse("en-US").unwrap(),
                LocaleId::parse("en").unwrap(),
            ]
        );
    }
}
