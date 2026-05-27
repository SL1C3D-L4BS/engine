//! SSA intermediate representation plus DCE, constant folding, and CSE.
//!
//! The IR is a low-level, register-based form between the type-checked AST
//! and the PR-2 bytecode. Each function is a single basic block of
//! [`IrInst`]s referring to virtual registers ([`IrReg`]); branching is
//! deferred to the bytecode lowering step. This is intentionally minimal —
//! the AST is small, the bytecode is small, and the IR mostly exists so
//! the three classical scalar optimisations (DCE / constant fold / CSE)
//! have somewhere to live.
//!
//! The passes are deterministic: same input AST yields the same instruction
//! sequence, which is what the cross-arch golden in
//! `tests/compile_parity.rs` keys on (ADR-034).

use crate::ast::{BinOp, Block, Decl, Expr, ExprKind, FnDecl, Lit, Module, Stmt, StmtKind, UnOp};

/// A virtual SSA register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IrReg(pub u32);

/// A constant value carried inline by [`IrInst::Const`].
#[derive(Clone, Debug, PartialEq)]
pub enum IrConst {
    /// `nil`
    Nil,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Float — bit pattern for deterministic equality.
    Float(u64),
    /// Interned string id.
    StrId(u32),
}

/// One SSA instruction.
#[derive(Clone, Debug, PartialEq)]
pub enum IrInst {
    /// `dst = const`
    Const(IrReg, IrConst),
    /// `dst = a OP b`
    Binary(IrReg, BinOp, IrReg, IrReg),
    /// `dst = OP a`
    Unary(IrReg, UnOp, IrReg),
    /// `dst = move(src)` — required when copying a value into a slot a
    /// later phi-like join refers to. SSA discipline is preserved at the
    /// AST-level via fresh registers per `let`; this opcode rebrands a
    /// register so dead-code elimination can prune unused variables
    /// without touching liveness analysis.
    Move(IrReg, IrReg),
    /// `dst = call name(args)`
    Call(IrReg, String, Vec<IrReg>),
    /// `return val?`
    Return(Option<IrReg>),
    /// Side-effecting drop — keeps a register alive past DCE because the
    /// statement that produced it must run.
    Eval(IrReg),
}

impl IrInst {
    /// Whether the instruction has a result register that is otherwise
    /// observably absent — DCE can drop it if the register is unused.
    pub fn dst(&self) -> Option<IrReg> {
        match self {
            Self::Const(d, _)
            | Self::Binary(d, _, _, _)
            | Self::Unary(d, _, _)
            | Self::Move(d, _)
            | Self::Call(d, _, _) => Some(*d),
            Self::Return(_) | Self::Eval(_) => None,
        }
    }

    /// Whether the instruction is observably pure (no side effects beyond
    /// computing a value). Calls are conservatively impure.
    pub fn is_pure(&self) -> bool {
        matches!(
            self,
            Self::Const(_, _) | Self::Binary(_, _, _, _) | Self::Unary(_, _, _) | Self::Move(_, _)
        )
    }
}

/// One IR function.
#[derive(Clone, Debug, PartialEq)]
pub struct IrFn {
    /// Function name.
    pub name: String,
    /// Parameter registers in declaration order.
    pub params: Vec<IrReg>,
    /// Instruction list.
    pub insts: Vec<IrInst>,
    /// Next-free register, used by the optimiser when synthesising
    /// replacement instructions.
    pub next_reg: u32,
}

/// A lowered module.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrModule {
    /// Functions in declaration order.
    pub functions: Vec<IrFn>,
}

/// Lowers the type-checked `module` to IR.
pub fn lower(module: &Module) -> IrModule {
    let mut out = IrModule::default();
    for d in &module.decls {
        if let Decl::Fn(f) = d {
            out.functions.push(lower_fn(f));
        }
    }
    out
}

fn lower_fn(f: &FnDecl) -> IrFn {
    let mut lo = Lowering::new();
    let mut env = LocalEnv::new();
    env.push();
    let mut params = Vec::new();
    for p in &f.params {
        let r = lo.fresh();
        params.push(r);
        env.insert(p.name.clone(), r);
    }
    let tail = lower_block(&f.body, &mut lo, &mut env);
    if let Some(r) = tail {
        lo.emit(IrInst::Return(Some(r)));
    } else {
        lo.emit(IrInst::Return(None));
    }
    env.pop();
    IrFn {
        name: f.name.clone(),
        params,
        insts: lo.insts,
        next_reg: lo.next,
    }
}

struct Lowering {
    insts: Vec<IrInst>,
    next: u32,
}

impl Lowering {
    fn new() -> Self {
        Self {
            insts: Vec::new(),
            next: 0,
        }
    }

    fn fresh(&mut self) -> IrReg {
        let r = IrReg(self.next);
        self.next += 1;
        r
    }

    fn emit(&mut self, inst: IrInst) {
        self.insts.push(inst);
    }
}

struct LocalEnv {
    scopes: Vec<Vec<(String, IrReg)>>,
}

impl LocalEnv {
    fn new() -> Self {
        Self { scopes: Vec::new() }
    }

    fn push(&mut self) {
        self.scopes.push(Vec::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn insert(&mut self, name: String, reg: IrReg) {
        self.scopes.last_mut().unwrap().push((name, reg));
    }

    fn lookup(&self, name: &str) -> Option<IrReg> {
        for s in self.scopes.iter().rev() {
            for (n, r) in s.iter().rev() {
                if n == name {
                    return Some(*r);
                }
            }
        }
        None
    }

    fn assign(&mut self, name: &str, reg: IrReg) {
        for s in self.scopes.iter_mut().rev() {
            for (n, r) in s.iter_mut().rev() {
                if n == name {
                    *r = reg;
                    return;
                }
            }
        }
        // Falls through silently — the resolver should already have
        // ensured `name` is in scope.
    }
}

fn lower_block(b: &Block, lo: &mut Lowering, env: &mut LocalEnv) -> Option<IrReg> {
    env.push();
    for s in &b.stmts {
        lower_stmt(s, lo, env);
    }
    let tail = b.tail.as_ref().map(|t| lower_expr(t, lo, env));
    env.pop();
    tail
}

fn lower_stmt(s: &Stmt, lo: &mut Lowering, env: &mut LocalEnv) {
    match &s.kind {
        StmtKind::Let { name, init, .. } => {
            let r = lower_expr(init, lo, env);
            env.insert(name.clone(), r);
        }
        StmtKind::Assign(target, value) => {
            let v = lower_expr(value, lo, env);
            if let ExprKind::Ident(n) = &target.kind {
                env.assign(n, v);
            } else {
                lo.emit(IrInst::Eval(v));
            }
        }
        StmtKind::Expr(e) => {
            let r = lower_expr(e, lo, env);
            lo.emit(IrInst::Eval(r));
        }
        StmtKind::Return(None) => lo.emit(IrInst::Return(None)),
        StmtKind::Return(Some(e)) => {
            let r = lower_expr(e, lo, env);
            lo.emit(IrInst::Return(Some(r)));
        }
        StmtKind::While(_, body) => {
            // PR 1 lowers loops to a linear unrolled body; the bytecode
            // back-end (PR 2) introduces explicit branches. This keeps
            // the IR optimiser simple — its purpose is scalar
            // simplification, not loop optimisation.
            lower_block(body, lo, env);
        }
        StmtKind::If(c, then, el) => {
            let _ = lower_expr(c, lo, env);
            lower_block(then, lo, env);
            if let Some(e) = el {
                lower_block(e, lo, env);
            }
        }
        StmtKind::Break | StmtKind::Continue => {}
    }
}

fn lower_expr(e: &Expr, lo: &mut Lowering, env: &mut LocalEnv) -> IrReg {
    match &e.kind {
        ExprKind::Lit(l) => {
            let r = lo.fresh();
            let c = match l {
                Lit::Nil => IrConst::Nil,
                Lit::Bool(v) => IrConst::Bool(*v),
                Lit::Int(v) => IrConst::Int(*v),
                Lit::Float(b) => IrConst::Float(*b),
                Lit::Str(_) => IrConst::StrId(0),
            };
            lo.emit(IrInst::Const(r, c));
            r
        }
        ExprKind::Ident(n) => env.lookup(n).unwrap_or_else(|| {
            let r = lo.fresh();
            lo.emit(IrInst::Const(r, IrConst::Nil));
            r
        }),
        ExprKind::Binary(op, l, r) => {
            let lr = lower_expr(l, lo, env);
            let rr = lower_expr(r, lo, env);
            let dst = lo.fresh();
            lo.emit(IrInst::Binary(dst, *op, lr, rr));
            dst
        }
        ExprKind::Unary(op, x) => {
            let xr = lower_expr(x, lo, env);
            let dst = lo.fresh();
            lo.emit(IrInst::Unary(dst, *op, xr));
            dst
        }
        ExprKind::Call(callee, args) => {
            let name = if let ExprKind::Ident(n) = &callee.kind {
                n.clone()
            } else {
                "<indirect>".to_string()
            };
            let arg_regs: Vec<IrReg> = args.iter().map(|a| lower_expr(a, lo, env)).collect();
            let dst = lo.fresh();
            lo.emit(IrInst::Call(dst, name, arg_regs));
            dst
        }
        ExprKind::Field(rcv, _) => {
            let r = lower_expr(rcv, lo, env);
            let dst = lo.fresh();
            lo.emit(IrInst::Move(dst, r));
            dst
        }
        ExprKind::Index(rcv, _) => {
            let r = lower_expr(rcv, lo, env);
            let dst = lo.fresh();
            lo.emit(IrInst::Move(dst, r));
            dst
        }
        ExprKind::StructLit(_, fields) => {
            for (_, fe) in fields {
                let _ = lower_expr(fe, lo, env);
            }
            let dst = lo.fresh();
            lo.emit(IrInst::Const(dst, IrConst::Nil));
            dst
        }
        ExprKind::ArrayLit(elems) => {
            // SSA IR is used for the const-fold / CSE / DCE pre-codegen
            // passes; aggregate types are opaque at this layer and the
            // result is a Nil-typed handle. Real bytecode emission for
            // ArrayLit lives in `codegen.rs` (ADR-060 wiring).
            for el in elems {
                let _ = lower_expr(el, lo, env);
            }
            let dst = lo.fresh();
            lo.emit(IrInst::Const(dst, IrConst::Nil));
            dst
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                let _ = lower_expr(k, lo, env);
                let _ = lower_expr(v, lo, env);
            }
            let dst = lo.fresh();
            lo.emit(IrInst::Const(dst, IrConst::Nil));
            dst
        }
        ExprKind::Closure(_, _) => {
            let dst = lo.fresh();
            lo.emit(IrInst::Const(dst, IrConst::Nil));
            dst
        }
        ExprKind::Block(b) => lower_block(b, lo, env).unwrap_or_else(|| {
            let dst = lo.fresh();
            lo.emit(IrInst::Const(dst, IrConst::Nil));
            dst
        }),
        ExprKind::If(c, then, el) => {
            let _ = lower_expr(c, lo, env);
            let _ = lower_block(then, lo, env);
            if let Some(e) = el {
                let _ = lower_block(e, lo, env);
            }
            let dst = lo.fresh();
            lo.emit(IrInst::Const(dst, IrConst::Nil));
            dst
        }
    }
}

// --- Optimisation passes -----------------------------------------------------

/// Runs all three scalar passes to a fixed point. Iteration stops when
/// no pass mutates the instruction list.
pub fn optimise(module: &mut IrModule) {
    for f in &mut module.functions {
        loop {
            let before = f.insts.clone();
            const_fold(f);
            cse(f);
            dce(f);
            if f.insts == before {
                break;
            }
        }
    }
}

/// Constant folding — `Binary`/`Unary` over two `Const`s is replaced with
/// the computed `Const`. Floats round-trip via `to_bits` to keep
/// determinism contract IV.2 (no FMA, strict IEEE-754).
pub fn const_fold(f: &mut IrFn) {
    // Snapshot `Const` definitions for quick lookup. SSA registers are
    // written exactly once, so an iterator pass with a flat table is
    // sufficient.
    let mut consts: Vec<Option<IrConst>> = vec![None; f.next_reg as usize];
    for inst in &f.insts {
        if let IrInst::Const(r, c) = inst {
            consts[r.0 as usize] = Some(c.clone());
        }
    }
    for inst in &mut f.insts {
        match inst {
            IrInst::Binary(dst, op, l, r) => {
                if let (Some(lc), Some(rc)) =
                    (consts[l.0 as usize].as_ref(), consts[r.0 as usize].as_ref())
                    && let Some(folded) = fold_binop(*op, lc, rc)
                {
                    consts[dst.0 as usize] = Some(folded.clone());
                    *inst = IrInst::Const(*dst, folded);
                }
            }
            IrInst::Unary(dst, op, x) => {
                if let Some(xc) = consts[x.0 as usize].as_ref()
                    && let Some(folded) = fold_unop(*op, xc)
                {
                    consts[dst.0 as usize] = Some(folded.clone());
                    *inst = IrInst::Const(*dst, folded);
                }
            }
            _ => {}
        }
    }
}

fn fold_binop(op: BinOp, l: &IrConst, r: &IrConst) -> Option<IrConst> {
    use IrConst::*;
    Some(match (l, r) {
        (Int(a), Int(b)) => match op {
            BinOp::Add => Int(a.wrapping_add(*b)),
            BinOp::Sub => Int(a.wrapping_sub(*b)),
            BinOp::Mul => Int(a.wrapping_mul(*b)),
            BinOp::Div => Int(a.checked_div(*b)?),
            BinOp::Mod => Int(a.checked_rem(*b)?),
            BinOp::Eq => Bool(a == b),
            BinOp::Ne => Bool(a != b),
            BinOp::Lt => Bool(a < b),
            BinOp::Le => Bool(a <= b),
            BinOp::Gt => Bool(a > b),
            BinOp::Ge => Bool(a >= b),
            _ => return None,
        },
        (Float(a), Float(b)) => {
            let af = f64::from_bits(*a);
            let bf = f64::from_bits(*b);
            match op {
                BinOp::Add => Float((af + bf).to_bits()),
                BinOp::Sub => Float((af - bf).to_bits()),
                BinOp::Mul => Float((af * bf).to_bits()),
                BinOp::Div => Float((af / bf).to_bits()),
                BinOp::Eq => Bool(af == bf),
                BinOp::Ne => Bool(af != bf),
                BinOp::Lt => Bool(af < bf),
                BinOp::Le => Bool(af <= bf),
                BinOp::Gt => Bool(af > bf),
                BinOp::Ge => Bool(af >= bf),
                _ => return None,
            }
        }
        (Bool(a), Bool(b)) => match op {
            BinOp::And => Bool(*a && *b),
            BinOp::Or => Bool(*a || *b),
            BinOp::Eq => Bool(a == b),
            BinOp::Ne => Bool(a != b),
            _ => return None,
        },
        _ => return None,
    })
}

fn fold_unop(op: UnOp, x: &IrConst) -> Option<IrConst> {
    use IrConst::*;
    Some(match (op, x) {
        (UnOp::Neg, Int(v)) => Int(v.wrapping_neg()),
        (UnOp::Neg, Float(b)) => Float((-f64::from_bits(*b)).to_bits()),
        (UnOp::Not, Bool(v)) => Bool(!v),
        _ => return None,
    })
}

/// Common-subexpression elimination: identical pure expressions over
/// the same operand registers reuse the first definition's destination.
/// Conservative — only handles `Const`, `Binary`, and `Unary` (the
/// instructions touched by const folding).
pub fn cse(f: &mut IrFn) {
    let mut seen: Vec<(InstKey, IrReg)> = Vec::new();
    let mut rewrite: Vec<(IrReg, IrReg)> = Vec::new();
    for inst in &f.insts {
        let key = inst_key(inst);
        if let Some(k) = key {
            if let Some((_, prior)) = seen.iter().find(|(prev, _)| prev == &k) {
                if let Some(dst) = inst.dst() {
                    rewrite.push((dst, *prior));
                }
            } else if let Some(dst) = inst.dst() {
                seen.push((k, dst));
            }
        }
    }
    if rewrite.is_empty() {
        return;
    }
    // Apply rewrites.
    let map = |r: &mut IrReg| {
        for (old, new) in &rewrite {
            if r == old {
                *r = *new;
                return;
            }
        }
    };
    for inst in &mut f.insts {
        match inst {
            IrInst::Binary(_, _, a, b) => {
                map(a);
                map(b);
            }
            IrInst::Unary(_, _, a) => map(a),
            IrInst::Move(_, a) => map(a),
            IrInst::Call(_, _, args) => {
                for a in args {
                    map(a);
                }
            }
            IrInst::Return(Some(a)) => map(a),
            IrInst::Eval(a) => map(a),
            _ => {}
        }
    }
}

/// Dead-code elimination: drop any pure instruction whose destination is
/// not read by a later instruction. Walks back-to-front so each removal
/// is observed by earlier passes.
pub fn dce(f: &mut IrFn) {
    let mut used = vec![false; f.next_reg as usize];
    // Parameter registers are always live — callers depend on them.
    for r in &f.params {
        if (r.0 as usize) < used.len() {
            used[r.0 as usize] = true;
        }
    }
    for inst in f.insts.iter().rev() {
        let mut keep = !inst.is_pure();
        if let Some(d) = inst.dst() {
            if used[d.0 as usize] {
                keep = true;
            }
        } else {
            keep = true;
        }
        if keep {
            match inst {
                IrInst::Binary(_, _, a, b) => {
                    used[a.0 as usize] = true;
                    used[b.0 as usize] = true;
                }
                IrInst::Unary(_, _, a) => {
                    used[a.0 as usize] = true;
                }
                IrInst::Move(_, a) => {
                    used[a.0 as usize] = true;
                }
                IrInst::Call(_, _, args) => {
                    for a in args {
                        used[a.0 as usize] = true;
                    }
                }
                IrInst::Return(Some(a)) => {
                    used[a.0 as usize] = true;
                }
                IrInst::Eval(a) => {
                    used[a.0 as usize] = true;
                }
                _ => {}
            }
        }
    }
    f.insts.retain(|inst| {
        if !inst.is_pure() {
            return true;
        }
        match inst.dst() {
            Some(d) => used[d.0 as usize],
            None => true,
        }
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum InstKey {
    Const(String),
    Binary(BinOp, IrReg, IrReg),
    Unary(UnOp, IrReg),
}

fn inst_key(inst: &IrInst) -> Option<InstKey> {
    Some(match inst {
        IrInst::Const(_, c) => InstKey::Const(format!("{:?}", c)),
        IrInst::Binary(_, op, a, b) => InstKey::Binary(*op, *a, *b),
        IrInst::Unary(_, op, a) => InstKey::Unary(*op, *a),
        _ => return None,
    })
}

/// Counts instructions in `module` — used by tests to assert that DCE
/// actually removed dead code.
pub fn count_insts(module: &IrModule) -> usize {
    module.functions.iter().map(|f| f.insts.len()).sum()
}

/// Serialises `module` into a deterministic, cross-arch-stable text form
/// — the same bytes on x86-64 and aarch64 given the same input. The
/// compile-parity golden in `tests/compile_parity.rs` takes a BLAKE3
/// digest over this text. Format is intentionally compact: one
/// instruction per line, `function/index` prefix on each.
pub fn serialise(module: &IrModule) -> String {
    let mut out = String::new();
    for f in &module.functions {
        out.push_str(&format!("fn {}#{}\n", f.name, f.params.len()));
        for (i, inst) in f.insts.iter().enumerate() {
            out.push_str(&format!("  {i:04}: {inst:?}\n"));
        }
    }
    out
}
