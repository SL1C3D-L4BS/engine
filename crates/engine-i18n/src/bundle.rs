//! Translation bundles and the `.ftl` message parser.
//!
//! The engine's localized strings live in Fluent-style `.ftl` files. This
//! module parses the subset the engine uses — simple messages, `{ $variable }`
//! interpolation, and plural/literal `select` expressions — into a
//! [`FluentBundle`] keyed by message id.
//!
//! The full Fluent runtime (terms, message references, attributes) and
//! CLDR-data-driven formatting are a later enhancement; the parsed
//! representation here is intentionally a strict subset (see the crate docs).

use crate::format::PluralCategory;
use crate::locale::LocaleId;
use std::collections::HashMap;

/// One element of a parsed message pattern.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Element {
    /// Literal text.
    Text(String),
    /// A `{ $name }` interpolation.
    Var(String),
    /// A `{ $name -> ... }` selection.
    Select {
        /// The variable the selection switches on.
        selector: String,
        /// The variants, in source order.
        variants: Vec<Variant>,
        /// Index into `variants` of the `*`-marked default.
        default: usize,
    },
}

/// One arm of a `select` expression.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Variant {
    pub(crate) key: VariantKey,
    pub(crate) pattern: Pattern,
}

/// The match key of a [`Variant`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum VariantKey {
    /// A CLDR plural category (`one`, `other`, …).
    Plural(PluralCategory),
    /// An exact integer match.
    Number(i64),
    /// An exact string match.
    Literal(String),
}

/// A parsed message: an ordered list of elements.
pub(crate) type Pattern = Vec<Element>;

/// A failure parsing an `.ftl` source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FtlError {
    /// A message line had no `=` separator.
    MissingEquals(usize),
    /// A `{` placeable was never closed.
    UnclosedPlaceable(usize),
    /// A `select` expression had no `*`-marked default variant.
    NoDefaultVariant(String),
    /// A `select` expression had no variants at all.
    EmptySelect(String),
}

/// A set of localized messages for one locale.
#[derive(Clone, Debug)]
pub struct FluentBundle {
    locale: LocaleId,
    messages: HashMap<String, Pattern>,
}

impl FluentBundle {
    /// Creates an empty bundle for `locale`.
    pub fn new(locale: LocaleId) -> Self {
        Self {
            locale,
            messages: HashMap::new(),
        }
    }

    /// The locale this bundle holds messages for.
    pub fn locale(&self) -> &LocaleId {
        &self.locale
    }

    /// Parses `source` and adds its messages, returning how many were added.
    /// A later definition of the same id replaces the earlier one.
    pub fn add_ftl(&mut self, source: &str) -> Result<usize, FtlError> {
        let parsed = parse_ftl(source)?;
        let count = parsed.len();
        self.messages.extend(parsed);
        Ok(count)
    }

    /// Whether `id` resolves in this bundle.
    pub fn has_message(&self, id: &str) -> bool {
        self.messages.contains_key(id)
    }

    /// The number of messages in the bundle.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// The parsed pattern for `id`, if present.
    pub(crate) fn pattern(&self, id: &str) -> Option<&Pattern> {
        self.messages.get(id)
    }
}

/// Parses a whole `.ftl` source into `id -> pattern` pairs.
fn parse_ftl(source: &str) -> Result<HashMap<String, Pattern>, FtlError> {
    let lines: Vec<&str> = source.lines().collect();
    let mut messages = HashMap::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let Some(eq) = line.find('=') else {
            return Err(FtlError::MissingEquals(i + 1));
        };
        let id = line[..eq].trim().to_string();
        let mut value = line[eq + 1..].trim().to_string();

        // Consume indented / continuation lines that belong to this message.
        i += 1;
        while i < lines.len() {
            let cont = lines[i];
            let cont_trim = cont.trim();
            let is_new_message =
                cont.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && cont.contains('=');
            if cont_trim.is_empty() || cont_trim.starts_with('#') || is_new_message {
                break;
            }
            value.push(' ');
            value.push_str(cont_trim);
            i += 1;
        }

        let pattern = parse_pattern_str(value.trim(), &id)?;
        messages.insert(id, pattern);
    }
    Ok(messages)
}

/// Parses one message's (whitespace-joined) value text into a [`Pattern`].
fn parse_pattern_str(text: &str, id: &str) -> Result<Pattern, FtlError> {
    let chars: Vec<char> = text.chars().collect();
    let mut parser = PatternParser {
        chars: &chars,
        pos: 0,
        id,
    };
    parser.parse_until_end()
}

/// A recursive-descent parser over one pattern's characters.
struct PatternParser<'a> {
    chars: &'a [char],
    pos: usize,
    id: &'a str,
}

impl PatternParser<'_> {
    /// Parses elements until the characters are exhausted.
    fn parse_until_end(&mut self) -> Result<Pattern, FtlError> {
        let mut elements = Vec::new();
        let mut text = String::new();
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c == '{' {
                if !text.is_empty() {
                    elements.push(Element::Text(std::mem::take(&mut text)));
                }
                elements.push(self.parse_placeable()?);
            } else {
                text.push(c);
                self.pos += 1;
            }
        }
        if !text.is_empty() {
            elements.push(Element::Text(text));
        }
        Ok(elements)
    }

    /// Parses a `{ ... }` placeable, with `pos` on the opening brace.
    fn parse_placeable(&mut self) -> Result<Element, FtlError> {
        let open = self.pos;
        let close = self.matching_brace(open)?;
        // Inner content, excluding the braces.
        let inner: String = self.chars[open + 1..close].iter().collect();
        self.pos = close + 1;

        let inner = inner.trim();
        let body = inner.strip_prefix('$').unwrap_or(inner).trim();

        if let Some(arrow) = body.find("->") {
            let selector = body[..arrow].trim().to_string();
            let variants_src = body[arrow + 2..].trim();
            let (variants, default) = self.parse_variants(variants_src)?;
            Ok(Element::Select {
                selector,
                variants,
                default,
            })
        } else {
            Ok(Element::Var(body.to_string()))
        }
    }

    /// Finds the index of the `}` matching the `{` at `open`.
    fn matching_brace(&self, open: usize) -> Result<usize, FtlError> {
        let mut depth = 0usize;
        for (offset, &c) in self.chars[open..].iter().enumerate() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(open + offset);
                    }
                }
                _ => {}
            }
        }
        Err(FtlError::UnclosedPlaceable(open))
    }

    /// Parses the variant list of a `select`, returning the variants and the
    /// index of the `*`-marked default.
    fn parse_variants(&self, src: &str) -> Result<(Vec<Variant>, usize), FtlError> {
        let chars: Vec<char> = src.chars().collect();
        let mut variants = Vec::new();
        let mut default = None;
        let mut i = 0;

        while i < chars.len() {
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }
            let is_default = chars[i] == '*';
            if is_default {
                i += 1;
            }
            if chars.get(i) != Some(&'[') {
                // Stray text between variants — skip a char and retry.
                i += 1;
                continue;
            }
            // Read the key up to ']'.
            let key_start = i + 1;
            let key_end = chars[key_start..]
                .iter()
                .position(|&c| c == ']')
                .map(|p| key_start + p)
                .ok_or_else(|| FtlError::EmptySelect(self.id.to_string()))?;
            let key: String = chars[key_start..key_end].iter().collect();
            i = key_end + 1;

            // The variant's pattern runs until the next variant marker.
            let pat_start = i;
            let pat_end = next_variant_start(&chars, i);
            let pattern_src: String = chars[pat_start..pat_end].iter().collect();
            i = pat_end;

            if is_default {
                default = Some(variants.len());
            }
            variants.push(Variant {
                key: classify_key(key.trim()),
                pattern: parse_pattern_str(pattern_src.trim(), self.id)?,
            });
        }

        if variants.is_empty() {
            return Err(FtlError::EmptySelect(self.id.to_string()));
        }
        let default = default.ok_or_else(|| FtlError::NoDefaultVariant(self.id.to_string()))?;
        Ok((variants, default))
    }
}

/// Index of the next variant marker (`[` or `*[`) at or after `from`, or the
/// end of the slice. Braces are skipped so a `{...}` inside a pattern is safe.
fn next_variant_start(chars: &[char], from: usize) -> usize {
    let mut i = from;
    let mut depth = 0usize;
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '[' if depth == 0 => return i,
            '*' if depth == 0 && chars.get(i + 1) == Some(&'[') => return i,
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

/// Classifies a raw variant key into a [`VariantKey`].
fn classify_key(key: &str) -> VariantKey {
    if let Ok(n) = key.parse::<i64>() {
        VariantKey::Number(n)
    } else if let Some(cat) = PluralCategory::from_keyword(key) {
        VariantKey::Plural(cat)
    } else {
        VariantKey::Literal(key.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(src: &str) -> FluentBundle {
        let mut b = FluentBundle::new(LocaleId::parse("en-US").unwrap());
        b.add_ftl(src).expect("parses");
        b
    }

    #[test]
    fn parses_plain_and_interpolated_messages() {
        let b = bundle("hello = Hello, world!\ngreet = Hi, { $name }!");
        assert_eq!(b.message_count(), 2);
        assert_eq!(
            b.pattern("hello"),
            Some(&vec![Element::Text("Hello, world!".into())])
        );
        assert_eq!(
            b.pattern("greet"),
            Some(&vec![
                Element::Text("Hi, ".into()),
                Element::Var("name".into()),
                Element::Text("!".into()),
            ])
        );
    }

    #[test]
    fn parses_a_multiline_select() {
        let src = "items =\n    { $count ->\n        [one] one item\n       *[other] { $count } items\n    }\n";
        let b = bundle(src);
        let Some(
            [
                Element::Select {
                    selector,
                    variants,
                    default,
                },
            ],
        ) = b.pattern("items").map(Vec::as_slice)
        else {
            panic!("expected a single select element");
        };
        assert_eq!(selector, "count");
        assert_eq!(variants.len(), 2);
        assert_eq!(*default, 1);
        assert_eq!(variants[0].key, VariantKey::Plural(PluralCategory::One));
        assert_eq!(variants[1].key, VariantKey::Plural(PluralCategory::Other));
    }

    #[test]
    fn a_select_without_a_default_is_rejected() {
        let mut b = FluentBundle::new(LocaleId::parse("en-US").unwrap());
        let err = b.add_ftl("x = { $n -> [one] a [other] b }").unwrap_err();
        assert_eq!(err, FtlError::NoDefaultVariant("x".into()));
    }

    #[test]
    fn a_line_without_equals_is_rejected() {
        let mut b = FluentBundle::new(LocaleId::parse("en-US").unwrap());
        assert_eq!(
            b.add_ftl("not a message").unwrap_err(),
            FtlError::MissingEquals(1)
        );
    }
}
