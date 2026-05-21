//! Breakpoint persistence oracle (PR 3, ADR-036).
//!
//! Writes the breakpoint table out as TOML, reads it back, and
//! asserts every field round-trips. Spec XII calls for RON; PR 3
//! uses TOML for the reasons recorded in ADR-036.

use engine_script::breakpoints_toml::{Persisted, read, write};
use std::path::PathBuf;

#[test]
fn roundtrip_owned_toml_writer_and_reader() {
    let original = vec![
        Persisted {
            file: PathBuf::from("game/main.sli"),
            line: 12,
            condition: None,
            hit_count: None,
        },
        Persisted {
            file: PathBuf::from("game/util.sli"),
            line: 42,
            condition: Some("x > 0".into()),
            hit_count: Some(3),
        },
    ];
    let text = write(&original);
    let back = read(&text);
    assert_eq!(back, original);
}

#[test]
fn comments_and_blank_lines_tolerated() {
    let text = r#"# user's notes
[[breakpoint]]
file = "main.sli"
line = 1

# another one
[[breakpoint]]
file = "util.sli"
line = 7
"#;
    let bps = read(text);
    assert_eq!(bps.len(), 2);
    assert_eq!(bps[0].line, 1);
    assert_eq!(bps[1].line, 7);
}

#[test]
fn strings_with_quotes_round_trip() {
    let bp = Persisted {
        file: PathBuf::from("a.sli"),
        line: 1,
        condition: Some(r#"name == "alice""#.into()),
        hit_count: None,
    };
    let text = write(std::slice::from_ref(&bp));
    let back = read(&text);
    assert_eq!(back, vec![bp]);
}
