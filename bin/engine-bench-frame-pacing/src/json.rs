//! Minimal JSON writer + reader for the bench report.
//!
//! Owned discipline (no serde): the bench binary's only output is a
//! flat object of strings, numbers, and one-level arrays of numbers, so
//! a hand-rolled writer is smaller than the type-tagging boilerplate
//! `serde_json` would impose. Reader is intentionally narrow — only
//! `read_top_level_number` is exposed, which is all the `--gate` mode
//! needs to recover `p99_ms` and `stddev_ms` from a prior run.

use std::fmt::Write as _;

/// Writer for a single top-level JSON object.
///
/// Fields are emitted in call order. Compact form: no whitespace
/// between tokens. Numeric `NaN`/`Infinity` are emitted as JSON `null`
/// since they are not legal JSON values — the gate treats `null` as
/// "missing", which fails the budget check (correct: a NaN frame-time
/// is a scenario error, not a passing result).
pub struct JsonWriter {
    buf: String,
    needs_comma: bool,
}

impl JsonWriter {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            needs_comma: false,
        }
    }

    pub fn begin_object(&mut self) {
        self.buf.push('{');
        self.needs_comma = false;
    }

    pub fn end_object(&mut self) {
        self.buf.push('}');
        self.needs_comma = true;
    }

    pub fn field_str(&mut self, name: &str, value: &str) {
        self.write_key(name);
        write_escaped(&mut self.buf, value);
        self.needs_comma = true;
    }

    pub fn field_u64(&mut self, name: &str, value: u64) {
        self.write_key(name);
        write!(self.buf, "{value}").expect("write to String never fails");
        self.needs_comma = true;
    }

    pub fn field_f64(&mut self, name: &str, value: f64) {
        self.write_key(name);
        write_number(&mut self.buf, value);
        self.needs_comma = true;
    }

    pub fn field_array_u32_2(&mut self, name: &str, value: [u32; 2]) {
        self.write_key(name);
        write!(self.buf, "[{},{}]", value[0], value[1]).expect("write to String never fails");
        self.needs_comma = true;
    }

    pub fn field_array_f64(&mut self, name: &str, values: Vec<f64>) {
        self.write_key(name);
        self.buf.push('[');
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                self.buf.push(',');
            }
            write_number(&mut self.buf, *v);
        }
        self.buf.push(']');
        self.needs_comma = true;
    }

    pub fn into_string(self) -> String {
        self.buf
    }

    fn write_key(&mut self, name: &str) {
        if self.needs_comma {
            self.buf.push(',');
        }
        write_escaped(&mut self.buf, name);
        self.buf.push(':');
    }
}

impl Default for JsonWriter {
    fn default() -> Self {
        Self::new()
    }
}

fn write_escaped(buf: &mut String, s: &str) {
    buf.push('"');
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            '\x08' => buf.push_str("\\b"),
            '\x0c' => buf.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                write!(buf, "\\u{:04x}", c as u32).expect("write to String never fails");
            }
            c => buf.push(c),
        }
    }
    buf.push('"');
}

fn write_number(buf: &mut String, value: f64) {
    if value.is_finite() {
        // `{}` uses the shortest round-trippable representation in
        // recent rustc (PR #110175). The bench round-trip test pins
        // that contract.
        write!(buf, "{value}").expect("write to String never fails");
    } else {
        buf.push_str("null");
    }
}

/// Read a top-level numeric field by name. Returns `None` if the key
/// is absent, if its value isn't a finite number, or if the JSON is
/// nested in a way this minimal reader can't handle (we never produce
/// nested objects, so that path is dead in practice).
///
/// The match is anchored on `"<key>":` and only considered if it sits
/// at brace-depth 1 (inside the outer object, not inside a string or
/// nested array). Strings are skipped over so a `"name":"p99_ms"`
/// payload can't masquerade as a key.
pub fn read_top_level_number(s: &str, key: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    let mut depth = 0_i32;
    let mut bracket = 0_i32;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => {
                // Walk past the string, honouring backslash escapes.
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'[' => bracket += 1,
            b']' => bracket -= 1,
            _ => {}
        }
        // Only treat `"key":` as a match at depth 1, bracket 0.
        if depth == 1 && bracket == 0 && b == b'"' && i < bytes.len() {
            // `i` points at the closing `"`. Scan the matched span.
            // We need the *opening* quote position to compare against
            // `key`. Walk back to find it.
            let close = i;
            let mut open = close;
            while open > 0 && bytes[open - 1] != b'"' {
                open -= 1;
            }
            // Now `open..close` is the unescaped-ish key body. Compare
            // by reading the original string slice — the bench never
            // emits escaped key names, so byte-equality is sufficient.
            if open <= close && &s[open..close] == key {
                // Expect `:` immediately after the closing quote.
                let after = close + 1;
                if after < bytes.len() && bytes[after] == b':' {
                    return parse_number_after(s, after + 1);
                }
            }
        }
        i += 1;
    }
    None
}

fn parse_number_after(s: &str, start: usize) -> Option<f64> {
    let rest = s.get(start..)?.trim_start();
    // Number ends at the first delimiter or whitespace.
    let end = rest
        .find(|c: char| matches!(c, ',' | ']' | '}' | ' ' | '\t' | '\n' | '\r'))
        .unwrap_or(rest.len());
    rest[..end].parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_emits_flat_object_in_call_order() {
        let mut w = JsonWriter::new();
        w.begin_object();
        w.field_str("name", "engine");
        w.field_u64("count", 7);
        w.field_f64("p99_ms", 12.5);
        w.field_array_u32_2("extent", [1280, 720]);
        w.field_array_f64("series", vec![1.0, 2.0, 3.5]);
        w.end_object();
        let s = w.into_string();
        assert!(s.starts_with('{') && s.ends_with('}'));
        assert!(s.contains("\"name\":\"engine\""));
        assert!(s.contains("\"count\":7"));
        assert!(s.contains("\"p99_ms\":12.5"));
        assert!(s.contains("\"extent\":[1280,720]"));
        assert!(s.contains("\"series\":[1,2,3.5]"));
    }

    #[test]
    fn writer_escapes_quotes_and_control_chars() {
        let mut w = JsonWriter::new();
        w.begin_object();
        w.field_str("k", "a\"b\nc\\d");
        w.end_object();
        let s = w.into_string();
        assert!(s.contains(r#""k":"a\"b\nc\\d""#));
    }

    #[test]
    fn writer_emits_null_for_nan_and_infinity() {
        let mut w = JsonWriter::new();
        w.begin_object();
        w.field_f64("nan", f64::NAN);
        w.field_f64("inf", f64::INFINITY);
        w.end_object();
        let s = w.into_string();
        assert!(s.contains("\"nan\":null"));
        assert!(s.contains("\"inf\":null"));
    }

    #[test]
    fn reader_extracts_top_level_number() {
        let s = r#"{"scenario":"x","p99_ms":12.5,"stddev_ms":1.1,"arr":[42.0,99.0]}"#;
        assert_eq!(read_top_level_number(s, "p99_ms"), Some(12.5));
        assert_eq!(read_top_level_number(s, "stddev_ms"), Some(1.1));
    }

    #[test]
    fn reader_returns_none_for_missing_key() {
        let s = r#"{"a":1,"b":2}"#;
        assert!(read_top_level_number(s, "c").is_none());
    }

    #[test]
    fn reader_ignores_keys_buried_inside_strings() {
        // A string value that *contains* `"p99_ms":99.0` should NOT match.
        let s = r#"{"note":"\"p99_ms\":99.0 is the budget","actual":7.0}"#;
        assert_eq!(read_top_level_number(s, "p99_ms"), None);
        assert_eq!(read_top_level_number(s, "actual"), Some(7.0));
    }

    #[test]
    fn reader_returns_none_for_null_value() {
        let s = r#"{"p99_ms":null}"#;
        assert!(read_top_level_number(s, "p99_ms").is_none());
    }
}
