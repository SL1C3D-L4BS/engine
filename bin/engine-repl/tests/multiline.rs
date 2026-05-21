//! Multi-line REPL input oracle (PR 3, ADR-036).
//!
//! The REPL itself runs each input as a single program. The
//! line-editor (in `bin/engine-repl/src/main.rs`) uses
//! `engine_script::repl::unmatched_brackets` to keep gathering input
//! until brackets balance — this oracle pins that behaviour.

use engine_script::repl::unmatched_brackets;

#[test]
fn balanced_one_liner_has_depth_zero() {
    assert_eq!(unmatched_brackets("1 + 2"), 0);
    assert_eq!(unmatched_brackets("(1 + 2)"), 0);
    assert_eq!(unmatched_brackets("{ let x = 1; x }"), 0);
}

#[test]
fn unbalanced_open_brace_has_positive_depth() {
    assert!(unmatched_brackets("fn main() {") > 0);
    assert!(unmatched_brackets("fn main() {\n    let x = 1;") > 0);
}

#[test]
fn close_balances_open() {
    let mut buf = String::from("fn main() {\n");
    assert!(unmatched_brackets(&buf) > 0);
    buf.push_str("    let x = 1;\n");
    assert!(unmatched_brackets(&buf) > 0);
    buf.push_str("}\n");
    assert_eq!(unmatched_brackets(&buf), 0);
}

#[test]
fn string_literals_dont_count_brackets() {
    assert_eq!(unmatched_brackets(r#"let s = "{{{";"#), 0);
}
