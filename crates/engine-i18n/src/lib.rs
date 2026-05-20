//! `engine-i18n` — Fluent-style internationalization.
//!
//! Level 1 crate. See `ENGINE_SPECIFICATION_v2.0.md` Part IV.1 and II.6.
//!
//! Localized strings are authored in Fluent `.ftl` files, parsed into
//! [`FluentBundle`]s, and resolved through a [`Translator`] that walks the
//! locale fallback chain. The [`tr!`] macro is the call site front door.
//!
//! # Modules
//!
//! - [`locale`] — locale identifiers, text [`Direction`], and the registry.
//! - [`bundle`] — `.ftl` parsing into message patterns.
//! - [`tr`] — argument binding and message resolution.
//! - [`format`] — number formatting and CLDR plural categories.
//!
//! # Scope
//!
//! This crate implements the Fluent *subset* the engine uses today — plain
//! messages, `{ $variable }` interpolation, and plural/literal `select`
//! expressions — with owned plural rules and number formatting for the
//! engine's shipped UI languages. The full Fluent runtime (terms, message
//! references, attributes) and CLDR-data-driven formatting via ICU4X are a
//! later enhancement; the parsed representation is forward-compatible.

pub mod bundle;
pub mod format;
pub mod locale;
pub mod tr;

pub use bundle::{FluentBundle, FtlError};
pub use format::{PluralCategory, format_decimal, format_integer, plural_category};
pub use locale::{Direction, LocaleId, LocaleRegistry};
pub use tr::{Arg, FluentArgs, Translator};
