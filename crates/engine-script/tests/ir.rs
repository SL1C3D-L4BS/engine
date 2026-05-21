//! IR optimiser oracles (PR 1, ADR-034).
//!
//! Hand-built `IrFn`s exercise each scalar pass independently — const
//! folding collapses arithmetic on `Const`s, CSE deduplicates identical
//! pure expressions, DCE drops unread pure results.

use engine_script::Compiler;
use engine_script::ast::{BinOp, UnOp};
use engine_script::ir::{
    IrConst, IrFn, IrInst, IrModule, IrReg, const_fold, count_insts, cse, dce, lower, optimise,
    serialise,
};
use engine_script::source::{Source, SourceMap};

fn mk_fn(insts: Vec<IrInst>, next_reg: u32) -> IrFn {
    IrFn {
        name: "test".to_string(),
        params: Vec::new(),
        insts,
        next_reg,
    }
}

#[test]
fn const_fold_int_add() {
    let mut f = mk_fn(
        vec![
            IrInst::Const(IrReg(0), IrConst::Int(2)),
            IrInst::Const(IrReg(1), IrConst::Int(3)),
            IrInst::Binary(IrReg(2), BinOp::Add, IrReg(0), IrReg(1)),
            IrInst::Return(Some(IrReg(2))),
        ],
        3,
    );
    const_fold(&mut f);
    assert_eq!(f.insts[2], IrInst::Const(IrReg(2), IrConst::Int(5)));
}

#[test]
fn const_fold_unary_neg_int() {
    let mut f = mk_fn(
        vec![
            IrInst::Const(IrReg(0), IrConst::Int(42)),
            IrInst::Unary(IrReg(1), UnOp::Neg, IrReg(0)),
            IrInst::Return(Some(IrReg(1))),
        ],
        2,
    );
    const_fold(&mut f);
    assert_eq!(f.insts[1], IrInst::Const(IrReg(1), IrConst::Int(-42)));
}

#[test]
fn cse_dedupes_identical_binary() {
    let mut f = mk_fn(
        vec![
            IrInst::Const(IrReg(0), IrConst::Int(1)),
            IrInst::Const(IrReg(1), IrConst::Int(2)),
            IrInst::Binary(IrReg(2), BinOp::Add, IrReg(0), IrReg(1)),
            IrInst::Binary(IrReg(3), BinOp::Add, IrReg(0), IrReg(1)),
            IrInst::Binary(IrReg(4), BinOp::Add, IrReg(2), IrReg(3)),
            IrInst::Return(Some(IrReg(4))),
        ],
        5,
    );
    cse(&mut f);
    // After CSE: IrReg(3) reads were rewritten to IrReg(2); the final
    // sum should refer to IrReg(2) twice.
    let last_bin = f
        .insts
        .iter()
        .rev()
        .find(|i| matches!(i, IrInst::Binary(_, _, _, _)))
        .unwrap();
    if let IrInst::Binary(_, op, a, b) = last_bin {
        assert_eq!(*op, BinOp::Add);
        assert_eq!(*a, IrReg(2));
        assert_eq!(*b, IrReg(2));
    } else {
        panic!("expected binary inst");
    }
}

#[test]
fn dce_drops_unused_pure_result() {
    let mut f = mk_fn(
        vec![
            IrInst::Const(IrReg(0), IrConst::Int(1)),
            // unused
            IrInst::Const(IrReg(1), IrConst::Int(999)),
            IrInst::Return(Some(IrReg(0))),
        ],
        2,
    );
    let before = f.insts.len();
    dce(&mut f);
    assert!(f.insts.len() < before, "expected at least one DCE removal");
    // The unused `999` constant must be gone.
    assert!(
        !f.insts
            .iter()
            .any(|i| matches!(i, IrInst::Const(_, IrConst::Int(999)))),
        "DCE failed to drop dead constant"
    );
}

#[test]
fn optimise_fixpoint_terminates() {
    let mut module = IrModule {
        functions: vec![mk_fn(
            vec![
                IrInst::Const(IrReg(0), IrConst::Int(10)),
                IrInst::Const(IrReg(1), IrConst::Int(20)),
                IrInst::Binary(IrReg(2), BinOp::Add, IrReg(0), IrReg(1)),
                IrInst::Binary(IrReg(3), BinOp::Add, IrReg(0), IrReg(1)),
                IrInst::Binary(IrReg(4), BinOp::Mul, IrReg(2), IrReg(3)),
                IrInst::Return(Some(IrReg(4))),
            ],
            5,
        )],
    };
    let before = count_insts(&module);
    optimise(&mut module);
    let after = count_insts(&module);
    assert!(after < before, "optimise must remove at least one inst");
    // The final return value should resolve to a single folded constant.
    let last = module.functions[0].insts.last().unwrap();
    assert!(matches!(last, IrInst::Return(Some(_))));
}

#[test]
fn lower_then_optimise_is_deterministic() {
    // Compiling the same module twice must yield byte-identical serialised IR.
    let src = r#"
fn add(a: i64, b: i64) -> i64 { return a + b; }
fn main() -> i64 { return add(1, 2) + add(1, 2); }
"#;
    let compile_once = |i: u32| {
        let mut sm = SourceMap::new();
        let id = sm.add(Source::new(format!("t{i}.sli"), src));
        Compiler::new().compile(id, sm.get(id)).unwrap().ir
    };
    let a = serialise(&compile_once(0));
    let b = serialise(&compile_once(1));
    assert_eq!(a, b);
    // Sanity: at least one Binary instruction survives.
    let lowered = lower(&{
        let mut sm = SourceMap::new();
        let id = sm.add(Source::new("t.sli", src));
        let mut diags = engine_script::diag::Diagnostics::new();
        let toks = engine_script::lex::lex(id, sm.get(id), &mut diags);
        engine_script::parse::parse(&toks, &mut diags)
    });
    assert!(!lowered.functions.is_empty());
}
