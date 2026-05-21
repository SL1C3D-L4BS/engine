//! Minimal owned TOML writer + key-line reader for
//! `.engine/debug/breakpoints.toml`.
//!
//! Spec XII calls for RON; the repo has no RON parser, so PR 3 uses a
//! TOML subset compatible with `engine.toml`'s shape. ADR-036 records
//! this deviation alongside the existing
//! [[foundation-layer-deviations]] tradition.
//!
//! Schema:
//!
//! ```toml
//! [[breakpoint]]
//! file = "game/main.sli"
//! line = 42
//! condition = "x > 0"   # optional
//! hit_count = 3         # optional
//! ```

use std::path::{Path, PathBuf};

/// One persisted breakpoint, source-keyed (the byte-offset is recomputed
/// on reload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Persisted {
    /// Source-file path as the user wrote it on the command line.
    pub file: PathBuf,
    /// 1-based source line.
    pub line: u32,
    /// Optional side-effect-free condition expression.
    pub condition: Option<String>,
    /// Optional hit-count gate.
    pub hit_count: Option<u32>,
}

/// Renders `breakpoints` into the TOML subset described in the module
/// docstring. Deterministic: keys are written in a fixed order.
pub fn write(breakpoints: &[Persisted]) -> String {
    let mut out = String::new();
    for (i, bp) in breakpoints.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str("[[breakpoint]]\n");
        out.push_str(&format!("file = {}\n", quote(&bp.file.to_string_lossy())));
        out.push_str(&format!("line = {}\n", bp.line));
        if let Some(c) = &bp.condition {
            out.push_str(&format!("condition = {}\n", quote(c)));
        }
        if let Some(h) = bp.hit_count {
            out.push_str(&format!("hit_count = {}\n", h));
        }
    }
    out
}

/// Parses the subset of TOML produced by [`write`]. Tolerates
/// blank lines, comments (`#`), and whitespace. Any line outside the
/// schema is ignored.
pub fn read(source: &str) -> Vec<Persisted> {
    let mut out = Vec::new();
    let mut current: Option<Persisted> = None;
    for raw in source.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[breakpoint]]" {
            if let Some(bp) = current.take() {
                out.push(bp);
            }
            current = Some(Persisted {
                file: PathBuf::new(),
                line: 0,
                condition: None,
                hit_count: None,
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let Some(cur) = current.as_mut() else {
            continue;
        };
        match key {
            "file" => cur.file = Path::new(&unquote(value)).to_path_buf(),
            "line" => {
                if let Ok(v) = value.parse() {
                    cur.line = v;
                }
            }
            "condition" => cur.condition = Some(unquote(value)),
            "hit_count" => {
                if let Ok(v) = value.parse() {
                    cur.hit_count = Some(v);
                }
            }
            _ => {}
        }
    }
    if let Some(bp) = current {
        out.push(bp);
    }
    out
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').unwrap_or(s);
    let s = s.strip_suffix('"').unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(c);
        }
    }
    out
}
