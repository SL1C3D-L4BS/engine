//! Type-checker positive + negative oracles (PR 1, ADR-034).

mod common;

use engine_script::diag::Diagnostics;
use engine_script::source::{Source, SourceMap};
use engine_script::{Compiler, Module};

fn check(text: &str) -> Diagnostics {
    let mut sm = SourceMap::new();
    let id = sm.add(Source::new("test.sli", text));
    Compiler::new().compile(id, sm.get(id)).unwrap().diagnostics
}

fn check_module(text: &str) -> Module {
    let mut sm = SourceMap::new();
    let id = sm.add(Source::new("test.sli", text));
    let mut diags = Diagnostics::new();
    let tokens = engine_script::lex::lex(id, sm.get(id), &mut diags);
    let mut m = engine_script::parse::parse(&tokens, &mut diags);
    engine_script::typeck::check(&mut m, &mut diags);
    m
}

// --- positive cases ---------------------------------------------------------

#[test]
fn whole_corpus_clean_typecheck() {
    for fix in common::corpus() {
        let diags = check(fix.source);
        assert!(
            !diags.has_errors(),
            "{} produced errors: {:?}",
            fix.name,
            diags
                .all()
                .iter()
                .map(|d| (&d.message, d.span))
                .collect::<Vec<_>>(),
        );
    }
}

#[test]
fn numeric_promotion_in_let_with_annotation() {
    let text = r#"
fn main() -> i32 {
    let x: i32 = 5;
    return x;
}
"#;
    let diags = check(text);
    assert!(!diags.has_errors(), "{:?}", diags.all());
}

#[test]
fn query_t_is_recognised() {
    let text = r#"
struct Position { x: f32, y: f32 }
fn s(q: Query<Position>) -> nil { return; }
"#;
    let diags = check(text);
    assert!(!diags.has_errors(), "{:?}", diags.all());
}

#[test]
fn res_and_resmut_and_entity_are_recognised() {
    let text = r#"
struct Score { v: i32 }
fn s(a: Res<Score>, b: ResMut<Score>, e: Entity) -> nil { return; }
"#;
    let diags = check(text);
    assert!(!diags.has_errors(), "{:?}", diags.all());
}

#[test]
fn struct_field_access_is_typed() {
    let text = r#"
struct V2 { x: f32, y: f32 }
fn d(a: V2) -> f32 { return a.x + a.y; }
"#;
    let m = check_module(text);
    // The return expression should now have type `f32`.
    if let engine_script::Decl::Fn(f) = &m.decls[1] {
        let tail_stmt = f
            .body
            .stmts
            .last()
            .expect("function body has at least one statement");
        if let engine_script::ast::StmtKind::Return(Some(e)) = &tail_stmt.kind {
            assert_eq!(e.ty, engine_script::Type::F32);
            return;
        }
    }
    panic!("did not find typed return expression");
}

// --- negative cases ---------------------------------------------------------

#[test]
fn type_mismatch_on_let() {
    let text = "fn m() -> nil { let x: i32 = true; return; }";
    let diags = check(text);
    assert!(diags.has_errors());
}

#[test]
fn undefined_name() {
    let text = "fn m() -> i64 { return missing + 1; }";
    let diags = check(text);
    assert!(diags.has_errors());
}

#[test]
fn unknown_struct_field() {
    let text = r#"
struct P { x: f32 }
fn m(p: P) -> f32 { return p.z; }
"#;
    let diags = check(text);
    assert!(diags.has_errors());
}

#[test]
fn missing_struct_field_in_literal() {
    let text = r#"
struct P { x: f32, y: f32 }
fn m() -> P { return P { x: 1.0 }; }
"#;
    let diags = check(text);
    assert!(diags.has_errors());
}

#[test]
fn arithmetic_on_mismatched_numeric_types() {
    let text = r#"
fn m() -> f32 {
    let a: i32 = 1;
    let b: f32 = 2.0;
    return a + b;
}
"#;
    let diags = check(text);
    assert!(diags.has_errors());
}

#[test]
fn arity_mismatch_on_call() {
    let text = r#"
fn add(a: i64, b: i64) -> i64 { return a + b; }
fn m() -> i64 { return add(1); }
"#;
    let diags = check(text);
    assert!(diags.has_errors());
}

#[test]
fn if_condition_must_be_bool() {
    let text = r#"
fn m() -> i64 {
    if 7 { return 1; } else { return 2; }
}
"#;
    let diags = check(text);
    assert!(diags.has_errors());
}
