//! Message resolution: turning a parsed pattern plus arguments into text.
//!
//! [`Translator`] ties a [`LocaleRegistry`] to the loaded [`FluentBundle`]s
//! and resolves a message id against the locale fallback chain. The [`tr!`]
//! macro is the ergonomic front door.

use crate::bundle::{Element, FluentBundle, Pattern, Variant, VariantKey};
use crate::format::{format_decimal, format_integer, plural_category};
use crate::locale::{LocaleId, LocaleRegistry};
use std::collections::HashMap;

/// A value passed into a message as an argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// A text value.
    Text(String),
    /// An integer value — drives plural selection and groups when formatted.
    Integer(i64),
    /// A floating-point value.
    Number(f64),
}

impl From<&str> for Arg {
    fn from(value: &str) -> Self {
        Arg::Text(value.to_string())
    }
}
impl From<String> for Arg {
    fn from(value: String) -> Self {
        Arg::Text(value)
    }
}
impl From<i64> for Arg {
    fn from(value: i64) -> Self {
        Arg::Integer(value)
    }
}
impl From<i32> for Arg {
    fn from(value: i32) -> Self {
        Arg::Integer(value as i64)
    }
}
impl From<f64> for Arg {
    fn from(value: f64) -> Self {
        Arg::Number(value)
    }
}
impl From<f32> for Arg {
    fn from(value: f32) -> Self {
        Arg::Number(value as f64)
    }
}

/// The named arguments supplied to a message.
#[derive(Clone, Debug, Default)]
pub struct FluentArgs {
    map: HashMap<String, Arg>,
}

impl FluentArgs {
    /// Creates an empty argument set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an argument, replacing any previous value for `name`.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<Arg>) -> &mut Self {
        self.map.insert(name.into(), value.into());
        self
    }

    /// Looks an argument up.
    pub fn get(&self, name: &str) -> Option<&Arg> {
        self.map.get(name)
    }
}

/// Resolves message ids across the locale fallback chain.
#[derive(Clone, Debug)]
pub struct Translator {
    registry: LocaleRegistry,
    bundles: HashMap<LocaleId, FluentBundle>,
}

impl Translator {
    /// Creates a translator driven by `registry`.
    pub fn new(registry: LocaleRegistry) -> Self {
        Self {
            registry,
            bundles: HashMap::new(),
        }
    }

    /// Registers a bundle under its own locale.
    pub fn add_bundle(&mut self, bundle: FluentBundle) {
        self.bundles.insert(bundle.locale().clone(), bundle);
    }

    /// Switches the active locale.
    pub fn set_locale(&mut self, locale: LocaleId) {
        self.registry.set_locale(locale);
    }

    /// The locale registry, for direction queries and the fallback chain.
    pub fn registry(&self) -> &LocaleRegistry {
        &self.registry
    }

    /// Resolves `id` with `args`, walking the fallback chain.
    ///
    /// If no locale in the chain defines `id`, the id itself is returned —
    /// the Fluent convention, which keeps a missing string visible rather
    /// than blank.
    pub fn translate(&self, id: &str, args: &FluentArgs) -> String {
        for locale in self.registry.fallback_chain() {
            if let Some(bundle) = self.bundles.get(&locale)
                && let Some(pattern) = bundle.pattern(id)
            {
                let mut out = String::new();
                resolve(pattern, args, bundle.locale(), &mut out);
                return out;
            }
        }
        id.to_string()
    }
}

/// Resolves a pattern, appending the result to `out`.
fn resolve(pattern: &Pattern, args: &FluentArgs, locale: &LocaleId, out: &mut String) {
    for element in pattern {
        match element {
            Element::Text(text) => out.push_str(text),
            Element::Var(name) => match args.get(name) {
                Some(Arg::Text(s)) => out.push_str(s),
                Some(Arg::Integer(n)) => out.push_str(&format_integer(locale, *n)),
                Some(Arg::Number(v)) => out.push_str(&format_number(locale, *v)),
                None => {
                    out.push_str("{$");
                    out.push_str(name);
                    out.push('}');
                }
            },
            Element::Select {
                selector,
                variants,
                default,
            } => {
                let chosen = select_variant(selector, variants, *default, args, locale);
                resolve(&variants[chosen].pattern, args, locale, out);
            }
        }
    }
}

/// Picks the index of the variant a `select` resolves to.
fn select_variant(
    selector: &str,
    variants: &[Variant],
    default: usize,
    args: &FluentArgs,
    locale: &LocaleId,
) -> usize {
    match args.get(selector) {
        Some(Arg::Integer(_)) | Some(Arg::Number(_)) => {
            let n = match args.get(selector) {
                Some(Arg::Integer(n)) => *n as f64,
                Some(Arg::Number(v)) => *v,
                _ => unreachable!(),
            };
            // An exact numeric key wins over the plural category.
            if let Some(i) = variants
                .iter()
                .position(|v| matches!(&v.key, VariantKey::Number(k) if (*k as f64) == n))
            {
                return i;
            }
            let category = plural_category(locale, n);
            variants
                .iter()
                .position(|v| matches!(&v.key, VariantKey::Plural(c) if *c == category))
                .unwrap_or(default)
        }
        Some(Arg::Text(s)) => variants
            .iter()
            .position(|v| matches!(&v.key, VariantKey::Literal(k) if k == s))
            .unwrap_or(default),
        None => default,
    }
}

/// Formats a floating-point argument: an integral value shows no decimals,
/// otherwise three are shown.
fn format_number(locale: &LocaleId, value: f64) -> String {
    if value.fract() == 0.0 {
        format_integer(locale, value as i64)
    } else {
        format_decimal(locale, value, 3)
    }
}

/// Resolves a message id against a translator.
///
/// ```ignore
/// tr!(translator, "save-button");
/// tr!(translator, "items-found", count = 3, query = "spear");
/// ```
#[macro_export]
macro_rules! tr {
    ($translator:expr, $id:expr $(,)?) => {
        $translator.translate($id, &$crate::FluentArgs::new())
    };
    ($translator:expr, $id:expr, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = $crate::FluentArgs::new();
        $( args.set(stringify!($name), $value); )+
        $translator.translate($id, &args)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translator(locale: &str, ftl: &str) -> Translator {
        let loc = LocaleId::parse(locale).unwrap();
        let mut bundle = FluentBundle::new(loc.clone());
        bundle.add_ftl(ftl).expect("parses");
        let mut t = Translator::new(LocaleRegistry::new(loc));
        t.add_bundle(bundle);
        t
    }

    #[test]
    fn interpolates_a_variable() {
        let t = translator("en-US", "greet = Hello, { $name }!");
        assert_eq!(tr!(t, "greet", name = "Ada"), "Hello, Ada!");
    }

    #[test]
    fn missing_message_returns_the_id() {
        let t = translator("en-US", "x = y");
        assert_eq!(tr!(t, "absent"), "absent");
    }

    #[test]
    fn plural_select_resolves_by_category() {
        let ftl = "items = { $count ->\n    [one] one item\n   *[other] { $count } items\n}";
        let t = translator("en-US", ftl);
        assert_eq!(tr!(t, "items", count = 1), "one item");
        assert_eq!(tr!(t, "items", count = 5), "5 items");
        assert_eq!(tr!(t, "items", count = 1_234), "1,234 items");
    }

    #[test]
    fn exact_number_key_beats_the_plural_category() {
        let ftl = "msg = { $n ->\n    [0] none\n    [one] one\n   *[other] many\n}";
        let t = translator("en-US", ftl);
        assert_eq!(tr!(t, "msg", n = 0), "none");
        assert_eq!(tr!(t, "msg", n = 1), "one");
        assert_eq!(tr!(t, "msg", n = 9), "many");
    }

    #[test]
    fn literal_select_handles_a_gender_arm() {
        let ftl =
            "won = { $gender ->\n    [male] He won\n    [female] She won\n   *[other] They won\n}";
        let t = translator("en-US", ftl);
        assert_eq!(tr!(t, "won", gender = "female"), "She won");
        assert_eq!(tr!(t, "won", gender = "male"), "He won");
        assert_eq!(tr!(t, "won", gender = "nonbinary"), "They won");
    }
}
