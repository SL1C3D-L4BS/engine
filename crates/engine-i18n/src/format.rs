//! Number formatting and CLDR plural-category selection.
//!
//! Full locale-data-driven formatting (every CLDR locale, currency, calendar
//! systems) is a later enhancement; this module ships the subset the
//! foundation needs — grouped integers, fixed-point decimals, and plural
//! categories for the languages the engine's own UI is translated into.

use crate::locale::LocaleId;

/// A CLDR plural category. A message's plural `select` chooses the variant
/// whose key names the category [`plural_category`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluralCategory {
    /// The `zero` category.
    Zero,
    /// The `one` (singular) category.
    One,
    /// The `two` (dual) category.
    Two,
    /// The `few` (paucal) category.
    Few,
    /// The `many` category.
    Many,
    /// The catch-all `other` category — every locale has it.
    Other,
}

impl PluralCategory {
    /// The lowercase CLDR keyword for this category.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }

    /// Parses a CLDR keyword into a category.
    pub fn from_keyword(word: &str) -> Option<Self> {
        Some(match word {
            "zero" => Self::Zero,
            "one" => Self::One,
            "two" => Self::Two,
            "few" => Self::Few,
            "many" => Self::Many,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// The plural category of cardinal `count` in `locale`.
///
/// The rules here cover the engine's shipped UI languages; an unknown
/// language uses the English-like `one` / `other` split, which is the safe
/// default for a brand-new translation.
pub fn plural_category(locale: &LocaleId, count: f64) -> PluralCategory {
    let n = count.abs();
    let is_int = n.fract() == 0.0;
    let i = n.trunc() as i64;

    match locale.language() {
        // Asian languages with no plural distinction.
        "ja" | "zh" | "ko" | "th" | "vi" | "id" => PluralCategory::Other,
        // French: 0 and 1 are singular.
        "fr" | "pt" => {
            if n < 2.0 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
        // Polish: a genuine few/many distinction on integers.
        "pl" if is_int => {
            let m10 = i % 10;
            let m100 = i % 100;
            if i == 1 {
                PluralCategory::One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                PluralCategory::Few
            } else {
                PluralCategory::Many
            }
        }
        // Arabic: the fullest set the engine handles.
        "ar" if is_int => {
            let m100 = i % 100;
            match i {
                0 => PluralCategory::Zero,
                1 => PluralCategory::One,
                2 => PluralCategory::Two,
                _ if (3..=10).contains(&m100) => PluralCategory::Few,
                _ if (11..=99).contains(&m100) => PluralCategory::Many,
                _ => PluralCategory::Other,
            }
        }
        // English and everything else: one iff exactly 1.
        _ => {
            if is_int && i == 1 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
    }
}

/// The thousands and decimal separators for a locale's number format.
fn separators(locale: &LocaleId) -> (char, char) {
    match locale.language() {
        // Comma-decimal locales.
        "de" | "es" | "it" | "pt" | "nl" | "pl" => ('.', ','),
        "fr" => ('\u{202F}', ','), // narrow no-break space groups in French
        // Default: English-style.
        _ => (',', '.'),
    }
}

/// Formats a signed integer with locale-appropriate thousands grouping.
pub fn format_integer(locale: &LocaleId, value: i64) -> String {
    let (group, _) = separators(locale);
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();

    let mut grouped = String::new();
    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0 && (digits.len() - idx).is_multiple_of(3) {
            grouped.push(group);
        }
        grouped.push(ch);
    }
    if negative {
        grouped.insert(0, '-');
    }
    grouped
}

/// Formats a number with grouping and exactly `fraction_digits` decimals,
/// rounding half-away-from-zero.
pub fn format_decimal(locale: &LocaleId, value: f64, fraction_digits: usize) -> String {
    let (_, decimal) = separators(locale);
    let scale = 10f64.powi(fraction_digits as i32);
    let scaled = (value.abs() * scale).round() as i64;
    let int_part = scaled / scale as i64;
    let frac_part = scaled % scale as i64;

    let mut out = format_integer(locale, int_part);
    if value < 0.0 && int_part == 0 {
        out.insert(0, '-');
    }
    if fraction_digits > 0 {
        out.push(decimal);
        out.push_str(&format!("{frac_part:0width$}", width = fraction_digits));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(tag: &str) -> LocaleId {
        LocaleId::parse(tag).unwrap()
    }

    #[test]
    fn english_plurals_split_one_and_other() {
        let en = loc("en-US");
        assert_eq!(plural_category(&en, 1.0), PluralCategory::One);
        assert_eq!(plural_category(&en, 0.0), PluralCategory::Other);
        assert_eq!(plural_category(&en, 2.0), PluralCategory::Other);
        assert_eq!(plural_category(&en, 1.5), PluralCategory::Other);
    }

    #[test]
    fn polish_plurals_have_a_few_category() {
        let pl = loc("pl");
        assert_eq!(plural_category(&pl, 1.0), PluralCategory::One);
        assert_eq!(plural_category(&pl, 3.0), PluralCategory::Few);
        assert_eq!(plural_category(&pl, 13.0), PluralCategory::Many);
        assert_eq!(plural_category(&pl, 22.0), PluralCategory::Few);
    }

    #[test]
    fn french_treats_zero_as_singular() {
        let fr = loc("fr");
        assert_eq!(plural_category(&fr, 0.0), PluralCategory::One);
        assert_eq!(plural_category(&fr, 1.0), PluralCategory::One);
        assert_eq!(plural_category(&fr, 2.0), PluralCategory::Other);
    }

    #[test]
    fn integers_group_per_locale() {
        assert_eq!(format_integer(&loc("en-US"), 1_234_567), "1,234,567");
        assert_eq!(format_integer(&loc("de"), 1_234_567), "1.234.567");
        assert_eq!(format_integer(&loc("en-US"), -12_345), "-12,345");
        assert_eq!(format_integer(&loc("en-US"), 42), "42");
    }

    #[test]
    fn decimals_round_and_use_the_locale_separator() {
        assert_eq!(format_decimal(&loc("en-US"), 1234.5678, 2), "1,234.57");
        assert_eq!(format_decimal(&loc("de"), 1234.5, 2), "1.234,50");
        assert_eq!(format_decimal(&loc("en-US"), 9.0, 0), "9");
    }

    #[test]
    fn keyword_round_trips() {
        for cat in [
            PluralCategory::Zero,
            PluralCategory::One,
            PluralCategory::Two,
            PluralCategory::Few,
            PluralCategory::Many,
            PluralCategory::Other,
        ] {
            assert_eq!(PluralCategory::from_keyword(cat.keyword()), Some(cat));
        }
        assert_eq!(PluralCategory::from_keyword("nope"), None);
    }
}
