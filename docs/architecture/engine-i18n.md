# engine-i18n

Fluent-style internationalization (spec IV.1 Level 1, II.6).

## Purpose

Loads localized strings, resolves them against the active locale's fallback
chain, and formats numbers and plurals correctly per locale.

## Modules

| Module    | Contents |
|-----------|----------|
| `locale`  | `LocaleId` (language + optional region), text `Direction` (LTR/RTL as a runtime property), and `LocaleRegistry` with the fallback chain. |
| `bundle`  | `.ftl` parsing into message patterns — plain messages, `{ $var }` interpolation, and plural/literal `select` expressions. |
| `tr`      | `Translator` resolution across the fallback chain; the `tr!` macro; `Arg` / `FluentArgs` argument binding. |
| `format`  | Locale-aware number grouping and decimals, plus CLDR plural categories. |

## Design notes

- Lookup walks the fallback chain — current locale, its bare language, the
  default locale, its bare language — and a missing message resolves to its
  own id (the Fluent convention: a missing string stays visible).
- Plural selection: a numeric argument is mapped to a CLDR category
  (`one`, `few`, `other`, …); an exact numeric variant key wins over the
  category. A string argument drives literal `select` arms (e.g. gender).
- Text direction is a runtime property the UI layer reads to mirror layout.

## Scope

This crate implements the Fluent *subset* the engine uses today, with owned
plural rules and number formatting for the shipped UI languages. The full
Fluent runtime (terms, message references, attributes) and CLDR-data-driven
formatting via ICU4X are a later enhancement; the parsed representation is
forward-compatible.

## Oracle

`tests/corpus.rs` parses the shipped `i18n/en-US/engine.ftl` corpus and
asserts that plural selection resolves every arm (`0` / `one` / `other`),
gender `select` resolves every arm, interpolation and grouping are correct,
and an unknown locale falls back to en-US.

## Dependencies

`std` only — the owned subset has no third-party dependencies.
