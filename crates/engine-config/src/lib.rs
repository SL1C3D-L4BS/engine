//! Owned line-oriented TOML reader (ADR-082).
//!
//! Level-1 foundation crate consolidating the parsers previously
//! duplicated in
//! [`engine_render::upscaler_config`](../engine_render/upscaler_config/index.html),
//! [`engine_bench_frame_pacing::budgets`](../engine_bench_frame_pacing/budgets/index.html),
//! and [`engine_script::breakpoints_toml`](../engine_script/breakpoints_toml/index.html).
//!
//! The crate exposes a narrow public surface:
//!
//! - [`Config`] — parsed structured view of a flat TOML file.
//! - [`Section`] — a `[section]` block plus its key/value entries.
//! - [`Value`] — the four primitive scalars the engine's configs need.
//! - [`parse`] — single entry-point parser returning a `Config`.
//! - [`strip_comment`] / [`unquote`] — quote-aware helpers re-used
//!   by call-site adapters that need string-level operations.
//!
//! ## Why owned
//!
//! ADR-051 entry 1 acknowledged the engine's owned TOML reader
//! pattern as a deliberate deviation from a vendored TOML library.
//! This crate keeps the same pattern; it does not introduce a
//! third-party dep. The crate is `no_std + alloc` compatible (the
//! current public surface uses `String` / `Vec<String>` which require
//! `alloc`, never `std::fs`).
//!
//! ## Schema scope
//!
//! The parser handles a strict subset of TOML:
//!
//! - Section headers: `[name]`. Section names cannot contain `]`.
//! - Key-value pairs: `key = value`. Keys are bare identifiers
//!   (alphanumeric + `_`); values are one of the four [`Value`]
//!   primitives.
//! - Quoted strings: `"..."` with `\\`, `\"`, `\n` escape sequences.
//! - Bare numbers: `i64` and `f64` (the parser picks `f64` if the
//!   value contains `.` or `e`).
//! - Booleans: literal `true` / `false`.
//! - Comments: `#` to end-of-line, quote-aware (a `#` inside a
//!   quoted string is preserved).
//!
//! Out of scope: arrays, tables, inline tables, dotted keys,
//! multi-line strings, dates. These do not appear in any of the
//! three consolidated call sites' schemas.

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Parsed structured view of a TOML body.
///
/// Sections are stored in source order; lookup is linear. The
/// engine's configs have ≤ 5 sections each, so this is fine. The
/// implicit "root" section (entries appearing before the first
/// `[section]` header) is stored under section name `""`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// Sections in source order. Lookup is linear via
    /// [`Config::section`].
    pub sections: Vec<Section>,
}

/// One `[section]` block plus its key/value entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    /// Section name. Empty string `""` for the implicit root
    /// (entries appearing before any `[header]`).
    pub name: String,
    /// Key/value pairs in source order. Duplicate keys retain the
    /// *last* assignment in source order (the prior assignments
    /// remain in the vec).
    pub entries: Vec<(String, Value)>,
}

/// The four primitive scalar value kinds the parser recognises.
///
/// The narrowness is deliberate: the consolidated parsers all need
/// these and only these. Future additions require an ADR-082
/// amendment + a new public-API minor version.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Quoted string (escapes decoded).
    String(String),
    /// Bare integer literal (`i64`).
    Integer(i64),
    /// Bare floating-point literal (`f64`).
    Float(f64),
    /// `true` / `false` keyword.
    Bool(bool),
}

/// Parse error variants. Lines + columns are 1-based for editor
/// integration; the parser never panics on malformed input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// A quoted string ended without a closing quote.
    UnterminatedString {
        /// 1-based source line.
        line: usize,
        /// 1-based source column.
        col: usize,
    },
    /// `[section]` header is missing its closing `]`.
    MalformedSectionHeader {
        /// 1-based source line.
        line: usize,
    },
    /// A key was empty (e.g. `= value`).
    EmptyKey {
        /// 1-based source line.
        line: usize,
    },
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::UnterminatedString { line, col } => {
                write!(f, "line {line} col {col}: unterminated string literal")
            }
            ParseError::MalformedSectionHeader { line } => {
                write!(f, "line {line}: malformed section header (missing `]`)")
            }
            ParseError::EmptyKey { line } => {
                write!(f, "line {line}: empty key on key/value pair")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

/// Parse `input` into a structured [`Config`].
///
/// Unknown lines (lines that don't match any of section header,
/// key/value, comment, or blank) are *silently skipped* — every
/// consolidated call site tolerates schema additions on old
/// readers, and the parser keeps that property.
pub fn parse(input: &str) -> Result<Config, ParseError> {
    let mut cfg = Config::default();
    let mut current = Section::default(); // root

    for (line_idx, raw) in input.lines().enumerate() {
        let line_num = line_idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        // Section header?
        if let Some(rest) = line.strip_prefix('[') {
            let Some(name) = rest.strip_suffix(']') else {
                return Err(ParseError::MalformedSectionHeader { line: line_num });
            };
            // Close current section; start new.
            cfg.sections.push(core::mem::take(&mut current));
            current.name = String::from(name.trim());
            continue;
        }

        // Key/value? Bare lines outside `[section]` go into the
        // implicit root section.
        let Some((key, value)) = line.split_once('=') else {
            continue; // unknown line; preserve forward-compat
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(ParseError::EmptyKey { line: line_num });
        }
        let value_str = value.trim();
        let value = parse_value(value_str, line_num)?;
        current.entries.push((String::from(key), value));
    }
    // Only push the trailing in-progress section if it carries
    // useful state (a non-empty section name, or at least one entry).
    if !current.name.is_empty() || !current.entries.is_empty() {
        cfg.sections.push(current);
    }
    // Trim leading empty root section if it has no entries (cleaner
    // call-site iteration).
    if cfg
        .sections
        .first()
        .is_some_and(|s| s.name.is_empty() && s.entries.is_empty())
        && cfg.sections.len() > 1
    {
        cfg.sections.remove(0);
    }
    Ok(cfg)
}

fn parse_value(raw: &str, line: usize) -> Result<Value, ParseError> {
    if raw.starts_with('"') {
        let Some(decoded) = unquote_strict(raw, line)? else {
            return Err(ParseError::UnterminatedString {
                line,
                col: 1 + raw.find('"').unwrap_or(0),
            });
        };
        return Ok(Value::String(decoded));
    }
    if raw == "true" {
        return Ok(Value::Bool(true));
    }
    if raw == "false" {
        return Ok(Value::Bool(false));
    }
    // Float if value contains '.' or 'e'/'E', otherwise integer.
    let looks_float = raw.contains('.') || raw.contains('e') || raw.contains('E');
    if !looks_float && let Ok(i) = raw.parse::<i64>() {
        return Ok(Value::Integer(i));
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Ok(Value::Float(f));
    }
    // Fall back to a bare string (no quoting). This matches the
    // historic budgets.rs behaviour where a stray value is silently
    // dropped to None — the caller's adapter typically rejects
    // unknown Value::String shapes for fields it expects numeric.
    Ok(Value::String(String::from(raw)))
}

/// Strip a `#` comment from a line. Quote-aware: a `#` inside a
/// double-quoted string is preserved.
///
/// Returns the slice of `line` up to but not including the first
/// un-quoted `#`. If no `#` appears (or all `#` are quoted), returns
/// `line` unchanged.
pub fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escape = false;
    for (i, c) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && in_string {
            escape = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            continue;
        }
        if c == '#' && !in_string {
            return &line[..i];
        }
    }
    line
}

/// Unquote a `"..."` literal, decoding `\\`, `\"`, `\n` escapes.
/// Returns `None` if the input is not a balanced double-quoted
/// string.
pub fn unquote(input: &str) -> Option<String> {
    unquote_strict(input, 1).ok().flatten()
}

fn unquote_strict(input: &str, line: usize) -> Result<Option<String>, ParseError> {
    let input = input.trim();
    let Some(rest) = input.strip_prefix('"') else {
        return Ok(None);
    };
    let Some(body) = rest.strip_suffix('"') else {
        return Err(ParseError::UnterminatedString { line, col: 1 });
    };
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {
                    return Err(ParseError::UnterminatedString { line, col: 1 });
                }
            }
        } else {
            out.push(c);
        }
    }
    Ok(Some(out))
}

impl Config {
    /// Look up the named section. Returns `None` if missing.
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Look up a `section.key` pair. Returns the *last* assignment
    /// if the key appears more than once in the section.
    pub fn get(&self, section: &str, key: &str) -> Option<&Value> {
        let s = self.section(section)?;
        s.entries
            .iter()
            .rev()
            .find_map(|(k, v)| if k == key { Some(v) } else { None })
    }
}

impl Value {
    /// Returns the string value if this is a [`Value::String`].
    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }

    /// Returns the integer value if this is a [`Value::Integer`].
    pub fn as_i64(&self) -> Option<i64> {
        if let Value::Integer(i) = self {
            Some(*i)
        } else {
            None
        }
    }

    /// Returns the floating-point value if this is a [`Value::Float`]
    /// or [`Value::Integer`] (integers widen losslessly into f64
    /// within ±2⁵³).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Returns the boolean value if this is a [`Value::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_input_returns_empty_config() {
        let cfg = parse("").expect("empty parses");
        assert!(cfg.sections.is_empty());
    }

    #[test]
    fn parse_section_with_string_and_numbers() {
        let src = r#"
            [upscaler]
            provider = "fsr"
            quality = "balanced"
            scale = 0.67
            samples = 8
            enabled = true
        "#;
        let cfg = parse(src).expect("parses");
        let s = cfg.section("upscaler").expect("section present");
        assert_eq!(
            s.entries.iter().find(|(k, _)| k == "provider").unwrap().1,
            Value::String("fsr".into())
        );
        assert_eq!(
            s.entries.iter().find(|(k, _)| k == "scale").unwrap().1,
            Value::Float(0.67)
        );
        assert_eq!(
            s.entries.iter().find(|(k, _)| k == "samples").unwrap().1,
            Value::Integer(8)
        );
        assert_eq!(
            s.entries.iter().find(|(k, _)| k == "enabled").unwrap().1,
            Value::Bool(true)
        );
    }

    #[test]
    fn parse_quote_aware_comment_keeps_hash_in_strings() {
        let src = r#"
            [issue]
            ref = "https://example.com/issue#42"  # this is a comment
        "#;
        let cfg = parse(src).expect("parses");
        let v = cfg.get("issue", "ref").expect("present");
        assert_eq!(v.as_str(), Some("https://example.com/issue#42"));
    }

    #[test]
    fn parse_escape_sequences_in_strings() {
        let src = r#"
            [path]
            quoted = "with \"inner\" quote"
            slashes = "C:\\path\\to\\file"
            newline = "line1\nline2"
        "#;
        let cfg = parse(src).expect("parses");
        assert_eq!(
            cfg.get("path", "quoted").and_then(Value::as_str),
            Some(r#"with "inner" quote"#)
        );
        assert_eq!(
            cfg.get("path", "slashes").and_then(Value::as_str),
            Some(r"C:\path\to\file")
        );
        assert_eq!(
            cfg.get("path", "newline").and_then(Value::as_str),
            Some("line1\nline2")
        );
    }

    #[test]
    fn parse_unterminated_string_errors() {
        let err = parse("[s]\nkey = \"oops").expect_err("error");
        assert!(matches!(err, ParseError::UnterminatedString { .. }));
    }

    #[test]
    fn parse_malformed_section_header_errors() {
        let err = parse("[unclosed\nkey = 1").expect_err("error");
        assert!(matches!(err, ParseError::MalformedSectionHeader { .. }));
    }

    #[test]
    fn parse_empty_key_errors() {
        let err = parse("[s]\n  = 1").expect_err("error");
        assert!(matches!(err, ParseError::EmptyKey { .. }));
    }

    #[test]
    fn parse_unknown_lines_are_silently_skipped() {
        // Forward-compat: a new schema field on an old reader is
        // tolerated by treating unknown lines (no `=`) as no-ops.
        let src = "[s]\nrandom\nkey = 1";
        let cfg = parse(src).expect("parses");
        assert_eq!(cfg.get("s", "key").and_then(Value::as_i64), Some(1));
    }

    #[test]
    fn parse_duplicate_key_returns_last_assignment() {
        let cfg = parse("[s]\nk = 1\nk = 2").expect("parses");
        assert_eq!(cfg.get("s", "k").and_then(Value::as_i64), Some(2));
    }

    #[test]
    fn parse_implicit_root_section_holds_bare_entries() {
        let src = "key = 7\n[s]\nother = 9";
        let cfg = parse(src).expect("parses");
        assert_eq!(cfg.get("", "key").and_then(Value::as_i64), Some(7));
        assert_eq!(cfg.get("s", "other").and_then(Value::as_i64), Some(9));
    }

    #[test]
    fn strip_comment_preserves_quoted_hashes() {
        assert_eq!(strip_comment("key = \"a#b\" # tail"), "key = \"a#b\" ");
        assert_eq!(strip_comment("plain # comment"), "plain ");
        assert_eq!(strip_comment("no comment here"), "no comment here");
    }

    #[test]
    fn unquote_round_trips_basic_escapes() {
        assert_eq!(unquote("\"plain\""), Some("plain".into()));
        assert_eq!(unquote("\"a\\nb\""), Some("a\nb".into()));
        assert_eq!(unquote("\"a\\\"b\""), Some("a\"b".into()));
        assert_eq!(unquote("bare"), None);
    }

    #[test]
    fn value_accessors_narrow_correctly() {
        assert_eq!(Value::Integer(5).as_i64(), Some(5));
        assert_eq!(Value::Integer(5).as_f64(), Some(5.0));
        assert_eq!(Value::Float(1.5).as_f64(), Some(1.5));
        assert_eq!(Value::Float(1.5).as_i64(), None);
        assert_eq!(Value::String("hi".into()).as_str(), Some("hi"));
        assert_eq!(Value::Bool(true).as_bool(), Some(true));
    }
}

#[cfg(feature = "std")]
pub mod fs {
    //! Filesystem-backed conveniences (std only).
    //!
    //! Default-on for the workspace; `default-features = false` consumers
    //! keep the crate `no_std + alloc`.
    use super::{Config, ParseError, parse};
    use std::io;
    use std::path::Path;

    /// Read + parse a config file from disk.
    pub fn read_from_path(path: &Path) -> Result<Config, FileError> {
        let body = std::fs::read_to_string(path).map_err(FileError::Io)?;
        parse(&body).map_err(FileError::Parse)
    }

    /// File-read or parse failure.
    #[derive(Debug)]
    pub enum FileError {
        /// Filesystem-level error opening or reading the file.
        Io(io::Error),
        /// Parse error in the file body.
        Parse(ParseError),
    }
}
