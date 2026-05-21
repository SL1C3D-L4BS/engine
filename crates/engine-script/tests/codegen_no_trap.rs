//! TRAP-collision fuzz oracle (PR 2, ADR-035).
//!
//! ADR-035 establishes four layers of TRAP-collision defence; this is
//! the runtime backstop. Compiles the PR-1 fixture corpus plus a
//! deterministic synthetic AST corpus (seeded with BLAKE3 over a small
//! set of templates) and asserts that none of them produce bytecode in
//! which an *opcode byte* equals `0xFF`. Operand bytes can absolutely
//! contain `0xFF` (negative jump offsets, integer immediates), so the
//! oracle walks the code stream via `Opcode::instr_len` and checks each
//! opcode position. This is exactly the invariant the verifier
//! enforces; the oracle proves codegen never produces it in the first
//! place.

mod common;

use blake3::Hasher;
use engine_script::bytecode::FunctionBytecode;
use engine_script::verify::verify;
use engine_script::{Compiler, Opcode, Source, SourceMap};

fn assert_no_trap_opcode(name: &str, f: &FunctionBytecode) {
    let code = &f.code;
    let mut pc = 0;
    while pc < code.len() {
        let byte = code[pc];
        assert_ne!(
            byte, 0xFF,
            "{} emitted TRAP opcode at byte offset {pc}\n  code: {code:?}",
            name
        );
        let op = Opcode::from_u8(byte)
            .unwrap_or_else(|| panic!("{name}@{pc}: unknown opcode 0x{byte:02x}"));
        pc += op.instr_len(code, pc);
    }
}

fn compile(name: &str, src: &str) -> engine_script::bytecode::Module {
    let mut sm = SourceMap::new();
    let id = sm.add(Source::new(name, src));
    let compiled = Compiler::new().compile(id, sm.get(id)).unwrap();
    compiled.bytecode
}

#[test]
fn corpus_emits_no_trap_opcode() {
    for fix in common::corpus() {
        let module = compile(fix.name, fix.source);
        for f in &module.functions {
            assert_no_trap_opcode(fix.name, f);
        }
        // Verifier is the architectural backstop — every program in
        // the corpus must verify.
        verify(&module).unwrap_or_else(|e| panic!("{} failed to verify: {e}", fix.name));
    }
}

#[test]
fn synthetic_corpus_emits_no_trap_opcode() {
    let templates = &[
        "fn f() -> i64 { return {LIT}; }",
        "fn f() -> i64 { let mut x: i64 = {LIT}; x = x + {LIT}; return x; }",
        "fn f() -> i64 { if {LIT} < {LIT} { return {LIT}; } else { return -{LIT}; } }",
        "fn f(a: i64, b: i64) -> i64 { return a * {LIT} + b - {LIT}; }",
        "fn f() -> i64 { let mut i: i64 = 0; let mut acc: i64 = 0; while i < {LIT} { acc = acc + i; i = i + 1; } return acc; }",
    ];
    let mut hasher = Hasher::new();
    hasher.update(b"sli-codegen-fuzz-v1");
    let mut state = hasher.finalize().as_bytes()[..16].to_vec();
    for n in 0..500u32 {
        let mut h = Hasher::new();
        h.update(&state);
        h.update(&n.to_le_bytes());
        let next = h.finalize().as_bytes()[..16].to_vec();
        state = next.clone();
        let tpl = templates[(next[0] as usize) % templates.len()];
        let mut filled = String::new();
        let mut lit_n = 1usize;
        let mut chars = tpl.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '{' && chars.peek() == Some(&'L') {
                // Consume `LIT}`.
                for _ in 0..4 {
                    chars.next();
                }
                let lit = (next[lit_n % next.len()] as i32) - 128;
                lit_n += 1;
                filled.push_str(&lit.to_string());
            } else {
                filled.push(c);
            }
        }
        let module = compile(&format!("syn_{n}"), &filled);
        for f in &module.functions {
            assert_no_trap_opcode(&format!("syn_{n}"), f);
        }
        verify(&module).unwrap_or_else(|e| panic!("syn_{n} failed to verify: {e}\nsrc: {filled}"));
    }
}
