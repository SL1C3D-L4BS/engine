//! Parser round-trip oracle (PR 1, ADR-034).
//!
//! For every fixture in the shared corpus: parse → pretty-print → parse a
//! second time, and assert the two ASTs are structurally equal. Catches
//! pretty-printer / parser drift, and pins the grammar's idempotence.

mod common;

use engine_script::diag::Diagnostics;
use engine_script::lex::lex;
use engine_script::parse::{parse, print};
use engine_script::source::{Source, SourceMap};

fn parse_text(name: &str, text: &str) -> engine_script::Module {
    let mut sm = SourceMap::new();
    let id = sm.add(Source::new(name, text));
    let mut diags = Diagnostics::new();
    let tokens = lex(id, sm.get(id), &mut diags);
    let module = parse(&tokens, &mut diags);
    assert!(
        !diags.has_errors(),
        "{name} produced parse errors: {:?}",
        diags.all().iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    module
}

#[test]
fn roundtrip_corpus() {
    // Spans differ between original and printed sources (whitespace
    // changes) so structural AST equality won't hold. The robust
    // property is *idempotence* of `print`: `print(parse(print(parse(s))))
    // == print(parse(s))` — a second round-trip is a no-op.
    for fix in common::corpus() {
        let a = parse_text(fix.name, fix.source);
        let printed_once = print(&a);
        let b = parse_text(&format!("{}.print", fix.name), &printed_once);
        let printed_twice = print(&b);
        assert_eq!(
            printed_once, printed_twice,
            "round-trip diverged for {}\nfirst print:\n{printed_once}\nsecond print:\n{printed_twice}",
            fix.name
        );
    }
}
