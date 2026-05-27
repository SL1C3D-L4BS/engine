//! AST → register bytecode lowering (PR 2 of Phase 4, ADR-035).
//!
//! The PR-1 IR in `ir.rs` is the optimised-scalar form the cross-arch
//! golden keys on; PR 2 emits bytecode directly from the type-checked
//! AST so explicit control-flow opcodes (`Jmp`, `JmpIfFalse`) and
//! function-id resolution are first-class. Both pipelines coexist —
//! later PRs may unify them, but the golden discipline of PR 1 keeps
//! the IR as the source of truth for *what was compiled*, while
//! `codegen` is the source of truth for *what runs*.
//!
//! Register allocation is a simple bump allocator: every expression
//! consumes a fresh destination register and we never reuse one within
//! a function (the verifier enforces `r < max_register`). The PR-3
//! optimiser will compress this — but the simple form lands first so
//! the test corpus runs end-to-end.

use crate::ast::{
    BinOp, Block, Decl, Expr, ExprKind, FnDecl, Lit, Module as AstModule, Param, Stmt, StmtKind,
    Type, UnOp,
};
use crate::bytecode::{Const, FunctionBytecode, Module, ModuleBuilder, Opcode};
use crate::source::{Source, Span};
use engine_core::collections::HashMap;

/// Lowers a type-checked AST module into bytecode.
///
/// `source` is the underlying `Source` the module was parsed from —
/// codegen converts AST spans into 1-based line numbers for the
/// `line_for_pc` side-table the debugger walks (ADR-036).
pub fn lower(ast: &AstModule, source: &Source) -> Module {
    let mut builder = ModuleBuilder::new();
    // Pass 1: register every function name so callers can resolve ids
    // before bodies are compiled (recursive calls). Ids are indices into
    // the bytecode module's `functions` table — not AST decl positions
    // (struct/const decls don't take a slot).
    let mut name_to_id: HashMap<String, u16> = HashMap::new();
    let mut next_id: u16 = 0;
    for d in &ast.decls {
        if let Decl::Fn(f) = d {
            name_to_id.insert(f.name.clone(), next_id);
            next_id += 1;
        }
    }
    // Pass 2: compile each function.
    for d in &ast.decls {
        if let Decl::Fn(f) = d {
            let bc = compile_fn(f, &name_to_id, &mut builder, source);
            builder.push_function(bc);
        }
    }
    builder.build()
}

struct Compiler<'a> {
    builder: &'a mut ModuleBuilder,
    fn_index: &'a HashMap<String, u16>,
    source: &'a Source,
    code: Vec<u8>,
    line_for_pc: Vec<u32>,
    next_reg: u16,
    scopes: Vec<HashMap<String, u8>>,
}

impl<'a> Compiler<'a> {
    fn new(
        builder: &'a mut ModuleBuilder,
        fn_index: &'a HashMap<String, u16>,
        source: &'a Source,
    ) -> Self {
        Self {
            builder,
            fn_index,
            source,
            code: Vec::new(),
            line_for_pc: Vec::new(),
            next_reg: 0,
            scopes: vec![HashMap::new()],
        }
    }

    /// 1-based source line for a span. Codegen calls this at every
    /// opcode emit so the debugger's `line_for_pc` table is dense
    /// (ADR-036). A zero-`lo` span (synthesised opcode) falls back to
    /// line 0, which the debugger treats as "no source line."
    fn line_for(&self, span: Span) -> u32 {
        if span.lo == 0 && span.hi == 0 {
            return 0;
        }
        self.source.line_col(span.lo).0
    }

    fn emit_at(&mut self, op: Opcode, span: Span) {
        let line = self.line_for(span);
        self.emit(op, line);
    }

    fn alloc_reg(&mut self) -> u8 {
        let r = self.next_reg;
        self.next_reg += 1;
        if r > 255 {
            // 256 regs is the verifier limit. Emit a wrap-around but
            // the verifier will reject the function — PR 3's optimiser
            // will compress before this matters in practice.
            return 255;
        }
        r as u8
    }

    fn bind(&mut self, name: String, reg: u8) {
        self.scopes.last_mut().unwrap().insert(name, reg);
    }

    fn lookup(&self, name: &str) -> Option<u8> {
        for s in self.scopes.iter().rev() {
            if let Some(r) = s.get(name) {
                return Some(*r);
            }
        }
        None
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn emit(&mut self, op: Opcode, line: u32) {
        self.code.push(op as u8);
        self.line_for_pc.push(line);
    }

    fn emit_byte(&mut self, b: u8) {
        self.code.push(b);
        // Pad the line table so indices line up with `code`. Only the
        // opcode-byte indices are read by the debugger.
        self.line_for_pc.push(0);
    }

    fn emit_u16(&mut self, v: u16) {
        let bytes = v.to_le_bytes();
        self.emit_byte(bytes[0]);
        self.emit_byte(bytes[1]);
    }

    fn emit_i16(&mut self, v: i16) {
        let bytes = v.to_le_bytes();
        self.emit_byte(bytes[0]);
        self.emit_byte(bytes[1]);
    }

    fn patch_i16(&mut self, at: usize, v: i16) {
        let bytes = v.to_le_bytes();
        self.code[at] = bytes[0];
        self.code[at + 1] = bytes[1];
    }
}

fn compile_fn(
    f: &FnDecl,
    fn_index: &HashMap<String, u16>,
    builder: &mut ModuleBuilder,
    source: &Source,
) -> FunctionBytecode {
    let mut c = Compiler::new(builder, fn_index, source);
    for p in &f.params {
        let r = c.alloc_reg();
        c.bind(p.name.clone(), r);
    }
    let body_value = compile_block(&mut c, &f.body);
    // If the body produced a tail value, emit a return for it; else nil.
    let tail_span = f.body.tail.as_ref().map(|e| e.span).unwrap_or(f.body.span);
    match body_value {
        Some(reg) => {
            c.emit_at(Opcode::ReturnVal, tail_span);
            c.emit_byte(reg);
        }
        None => {
            // The block ended with a `return ...` already, OR has no tail.
            // Emit an implicit `ReturnNil` as a backstop — verifier
            // requires it.
            c.emit_at(Opcode::ReturnNil, tail_span);
        }
    }
    FunctionBytecode {
        name: f.name.clone(),
        arity: f.params.len() as u8,
        max_register: c.next_reg.min(256) as u8,
        code: c.code,
        line_for_pc: c.line_for_pc,
    }
}

fn compile_block(c: &mut Compiler, b: &Block) -> Option<u8> {
    c.push_scope();
    for s in &b.stmts {
        compile_stmt(c, s);
    }
    let tail_reg = b.tail.as_ref().map(|e| compile_expr(c, e));
    c.pop_scope();
    tail_reg
}

fn compile_stmt(c: &mut Compiler, s: &Stmt) {
    match &s.kind {
        StmtKind::Let { name, init, .. } => {
            let r = compile_expr(c, init);
            c.bind(name.clone(), r);
        }
        StmtKind::Assign(target, value) => {
            let v = compile_expr(c, value);
            if let ExprKind::Ident(name) = &target.kind
                && let Some(r) = c.lookup(name)
            {
                c.emit_at(Opcode::Move, s.span);
                c.emit_byte(r);
                c.emit_byte(v);
            }
            // Field / index assignment lowering is a PR 3 task.
        }
        StmtKind::Expr(e) => {
            let _ = compile_expr(c, e);
        }
        StmtKind::Return(None) => {
            c.emit_at(Opcode::ReturnNil, s.span);
        }
        StmtKind::Return(Some(e)) => {
            let r = compile_expr(c, e);
            c.emit_at(Opcode::ReturnVal, s.span);
            c.emit_byte(r);
        }
        StmtKind::While(cond, body) => {
            let loop_start = c.code.len();
            let cond_reg = compile_expr(c, cond);
            c.emit_at(Opcode::JmpIfFalse, cond.span);
            c.emit_byte(cond_reg);
            let exit_patch = c.code.len();
            c.emit_i16(0); // placeholder
            compile_block(c, body);
            // Jump back to loop start.
            c.emit_at(Opcode::Jmp, s.span);
            let here = c.code.len() as isize + 2;
            let back_offset = loop_start as isize - here;
            c.emit_i16(back_offset as i16);
            // Patch the exit jump.
            let after = c.code.len() as isize;
            let exit_offset = after - (exit_patch as isize + 2);
            c.patch_i16(exit_patch, exit_offset as i16);
        }
        StmtKind::If(cond, then, else_) => {
            compile_if(c, cond, then, else_.as_deref(), None);
        }
        StmtKind::Break | StmtKind::Continue => {}
    }
}

fn compile_expr(c: &mut Compiler, e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Lit(l) => {
            let dst = c.alloc_reg();
            match l {
                Lit::Nil => c.emit_at(Opcode::ConstNil, e.span),
                Lit::Bool(true) => c.emit_at(Opcode::ConstTrue, e.span),
                Lit::Bool(false) => c.emit_at(Opcode::ConstFalse, e.span),
                Lit::Int(v) => {
                    c.emit_at(Opcode::ConstInt, e.span);
                    let idx = c.builder.intern(Const::Int(*v));
                    c.emit_byte(dst);
                    c.emit_u16(idx);
                    return dst;
                }
                Lit::Float(b) => {
                    c.emit_at(Opcode::ConstFloat, e.span);
                    let idx = c.builder.intern(Const::Float(*b));
                    c.emit_byte(dst);
                    c.emit_u16(idx);
                    return dst;
                }
                Lit::Str(s) => {
                    c.emit_at(Opcode::ConstStr, e.span);
                    let idx = c.builder.intern(Const::Str(s.clone()));
                    c.emit_byte(dst);
                    c.emit_u16(idx);
                    return dst;
                }
            }
            c.emit_byte(dst);
            dst
        }
        ExprKind::Ident(name) => {
            // Locals: just return their register. Functions: a closure
            // value would need allocation; PR 2 doesn't support
            // function-as-value usage outside direct `Call`.
            if let Some(r) = c.lookup(name) {
                r
            } else {
                // Synthesise a nil placeholder; the type checker would
                // already have flagged this.
                let dst = c.alloc_reg();
                c.emit_at(Opcode::ConstNil, e.span);
                c.emit_byte(dst);
                dst
            }
        }
        ExprKind::Binary(op, l, r) => {
            let lr = compile_expr(c, l);
            let rr = compile_expr(c, r);
            let dst = c.alloc_reg();
            let opc = binop_opcode(*op);
            c.emit_at(opc, e.span);
            c.emit_byte(dst);
            c.emit_byte(lr);
            c.emit_byte(rr);
            dst
        }
        ExprKind::Unary(op, x) => {
            let xr = compile_expr(c, x);
            let dst = c.alloc_reg();
            let opc = match op {
                UnOp::Neg => Opcode::Neg,
                UnOp::Not => Opcode::Not,
            };
            c.emit_at(opc, e.span);
            c.emit_byte(dst);
            c.emit_byte(xr);
            dst
        }
        ExprKind::Call(callee, args) => {
            let arg_regs: Vec<u8> = args.iter().map(|a| compile_expr(c, a)).collect();
            // Direct call: `Ident(name)` that resolves in the top-level
            // function index. Emits the cheap `Call` opcode.
            if let ExprKind::Ident(name) = &callee.kind
                && let Some(id) = c.fn_index.get(name)
            {
                let dst = c.alloc_reg();
                c.emit_at(Opcode::Call, e.span);
                c.emit_byte(dst);
                c.emit_u16(*id);
                c.emit_byte(arg_regs.len() as u8);
                for r in &arg_regs {
                    c.emit_byte(*r);
                }
                return dst;
            }
            // Local-bound callee: `Ident(name)` that resolves to a
            // closure value in a register (e.g. `let f = |x| ...; f(3)`),
            // or any non-Ident callee expression. Emits `CallClosure`.
            let cls_reg = match &callee.kind {
                ExprKind::Ident(name) => match c.lookup(name) {
                    Some(r) => r,
                    None => {
                        // Unknown name — emit nil placeholder; PR 3 will
                        // route this through the FFI table.
                        let dst = c.alloc_reg();
                        c.emit_at(Opcode::ConstNil, e.span);
                        c.emit_byte(dst);
                        return dst;
                    }
                },
                _ => compile_expr(c, callee),
            };
            let dst = c.alloc_reg();
            c.emit_at(Opcode::CallClosure, e.span);
            c.emit_byte(dst);
            c.emit_byte(cls_reg);
            c.emit_byte(arg_regs.len() as u8);
            for r in &arg_regs {
                c.emit_byte(*r);
            }
            dst
        }
        ExprKind::ArrayLit(elems) => {
            // ADR-060 ArrayNew packs all elements in one instruction:
            // `dst:u8 n:u8 arg0..argN-1:u8`. Compile each element first
            // (each gets its own register), then allocate the dst and
            // emit. Empty array literal `[]` lowers to `ArrayNew dst 0`.
            let arg_regs: Vec<u8> = elems.iter().map(|el| compile_expr(c, el)).collect();
            let dst = c.alloc_reg();
            c.emit_at(Opcode::ArrayNew, e.span);
            c.emit_byte(dst);
            c.emit_byte(arg_regs.len() as u8);
            for r in &arg_regs {
                c.emit_byte(*r);
            }
            dst
        }
        ExprKind::MapLit(pairs) => {
            // ADR-060 MapNew + per-pair MapSet. The dispatcher's MapSet
            // (dispatch.rs) fires `heap.write_barrier(map, value)` when
            // crossing generations, so codegen does not insert barriers
            // itself.
            let pair_regs: Vec<(u8, u8)> = pairs
                .iter()
                .map(|(k, v)| (compile_expr(c, k), compile_expr(c, v)))
                .collect();
            let dst = c.alloc_reg();
            c.emit_at(Opcode::MapNew, e.span);
            c.emit_byte(dst);
            for (kreg, vreg) in &pair_regs {
                c.emit_at(Opcode::MapSet, e.span);
                c.emit_byte(dst);
                c.emit_byte(*kreg);
                c.emit_byte(*vreg);
            }
            dst
        }
        ExprKind::StructLit(_, fields) => {
            // `StructNew dst` then per field `StructSet dst name_ki src`.
            // Field names intern into the const pool's `Str` table; the
            // dispatcher resolves them at runtime against the heap
            // object's field map.
            let field_regs: Vec<(String, u8)> = fields
                .iter()
                .map(|(fname, fexpr)| (fname.clone(), compile_expr(c, fexpr)))
                .collect();
            let dst = c.alloc_reg();
            c.emit_at(Opcode::StructNew, e.span);
            c.emit_byte(dst);
            for (fname, vreg) in &field_regs {
                let ki = c.builder.intern(Const::Str(fname.clone()));
                c.emit_at(Opcode::StructSet, e.span);
                c.emit_byte(dst);
                c.emit_u16(ki);
                c.emit_byte(*vreg);
            }
            dst
        }
        ExprKind::Field(rcv, name) => {
            // `dst = rcv.<name>` — StructGet dst rreg name_ki:u16le.
            let rreg = compile_expr(c, rcv);
            let dst = c.alloc_reg();
            let ki = c.builder.intern(Const::Str(name.clone()));
            c.emit_at(Opcode::StructGet, e.span);
            c.emit_byte(dst);
            c.emit_byte(rreg);
            c.emit_u16(ki);
            dst
        }
        ExprKind::Index(rcv, idx) => {
            // Branch on the receiver's type recorded by typeck. Array
            // → ArrayGet; Map → MapGet; Error → nil fallback.
            let rreg = compile_expr(c, rcv);
            let ireg = compile_expr(c, idx);
            let dst = c.alloc_reg();
            let opcode = match &rcv.ty {
                Type::Array(_) => Opcode::ArrayGet,
                Type::Map(_, _) => Opcode::MapGet,
                Type::Error => {
                    // typeck flagged this; emit a nil so the verifier
                    // doesn't blow up downstream.
                    c.emit_at(Opcode::ConstNil, e.span);
                    c.emit_byte(dst);
                    return dst;
                }
                _ => {
                    // Defensive: typeck normally catches non-container
                    // indexing; emit nil.
                    c.emit_at(Opcode::ConstNil, e.span);
                    c.emit_byte(dst);
                    return dst;
                }
            };
            c.emit_at(opcode, e.span);
            c.emit_byte(dst);
            c.emit_byte(rreg);
            c.emit_byte(ireg);
            dst
        }
        ExprKind::Closure(params, body) => {
            // Discover free variables in the closure body. Free variables
            // are referenced by name; each maps to a register in the
            // *enclosing* compiler's scope. We capture by value at
            // ClosureMake time; the captured register byte is read into
            // register-0..k of the callee frame at runtime.
            let captures = collect_free_vars(body, params);
            // Resolve each free variable to its outer register. Free
            // variables that don't resolve (rare — typeck would have
            // flagged them) drop out of the capture list.
            let mut capture_regs: Vec<u8> = Vec::with_capacity(captures.len());
            let mut resolved_names: Vec<String> = Vec::with_capacity(captures.len());
            for name in &captures {
                if let Some(r) = c.lookup(name) {
                    capture_regs.push(r);
                    resolved_names.push(name.clone());
                }
            }
            // Compile the closure body as a fresh top-level function.
            // The callee frame is laid out as `[captures.., params..]`,
            // so we bind captures in registers 0..k and params in k..k+n.
            let fn_idx = compile_closure_body(c, &resolved_names, params, body);
            let dst = c.alloc_reg();
            c.emit_at(Opcode::ClosureMake, e.span);
            c.emit_byte(dst);
            c.emit_u16(fn_idx);
            c.emit_byte(capture_regs.len() as u8);
            for r in &capture_regs {
                c.emit_byte(*r);
            }
            dst
        }
        ExprKind::Block(b) => compile_block(c, b).unwrap_or_else(|| {
            let dst = c.alloc_reg();
            c.emit_at(Opcode::ConstNil, e.span);
            c.emit_byte(dst);
            dst
        }),
        ExprKind::If(cond, then, else_) => {
            let dst = c.alloc_reg();
            compile_if(c, cond, then, else_.as_deref(), Some(dst));
            dst
        }
    }
}

fn compile_if(c: &mut Compiler, cond: &Expr, then: &Block, else_: Option<&Block>, out: Option<u8>) {
    let cond_reg = compile_expr(c, cond);
    c.emit_at(Opcode::JmpIfFalse, cond.span);
    c.emit_byte(cond_reg);
    let to_else_patch = c.code.len();
    c.emit_i16(0);
    let then_tail = compile_block(c, then);
    if let (Some(dst), Some(tr)) = (out, then_tail)
        && dst != tr
    {
        c.emit_at(Opcode::Move, then.span);
        c.emit_byte(dst);
        c.emit_byte(tr);
    }
    // Unconditional jump past the else.
    c.emit_at(Opcode::Jmp, then.span);
    let to_end_patch = c.code.len();
    c.emit_i16(0);
    // else label.
    let else_pos = c.code.len() as isize;
    let to_else_off = else_pos - (to_else_patch as isize + 2);
    c.patch_i16(to_else_patch, to_else_off as i16);
    if let Some(eb) = else_ {
        let etail = compile_block(c, eb);
        if let (Some(dst), Some(er)) = (out, etail)
            && dst != er
        {
            c.emit_at(Opcode::Move, eb.span);
            c.emit_byte(dst);
            c.emit_byte(er);
        }
    } else if let Some(dst) = out {
        c.emit_at(Opcode::ConstNil, cond.span);
        c.emit_byte(dst);
    }
    // end label.
    let end_pos = c.code.len() as isize;
    let to_end_off = end_pos - (to_end_patch as isize + 2);
    c.patch_i16(to_end_patch, to_end_off as i16);
}

fn binop_opcode(op: BinOp) -> Opcode {
    match op {
        BinOp::Add => Opcode::Add,
        BinOp::Sub => Opcode::Sub,
        BinOp::Mul => Opcode::Mul,
        BinOp::Div => Opcode::Div,
        BinOp::Mod => Opcode::Mod,
        BinOp::Eq => Opcode::Eq,
        BinOp::Ne => Opcode::Ne,
        BinOp::Lt => Opcode::Lt,
        BinOp::Le => Opcode::Le,
        BinOp::Gt => Opcode::Gt,
        BinOp::Ge => Opcode::Ge,
        BinOp::And => Opcode::And,
        BinOp::Or => Opcode::Or,
    }
}

/// Discover free variables in a closure body, in source-position order.
///
/// A free variable is an `Ident` referenced inside the body that is not
/// bound by:
///   - the closure's own params, or
///   - an inner `let` (block-scoped), or
///   - an inner closure's params.
///
/// Returns each name once, in first-occurrence order. This drives the
/// capture list emitted by [`Opcode::ClosureMake`]. The walker treats
/// `ExprKind::Ident` as the only free-variable production — function
/// calls by name (`fn_index` lookups) and built-ins resolve at the
/// module level and are not captured.
fn collect_free_vars(body: &Expr, params: &[Param]) -> Vec<String> {
    let mut bound: Vec<Vec<String>> = vec![params.iter().map(|p| p.name.clone()).collect()];
    let mut out: Vec<String> = Vec::new();
    walk_free_vars(body, &mut bound, &mut out);
    out
}

fn walk_free_vars(e: &Expr, bound: &mut Vec<Vec<String>>, out: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Lit(_) => {}
        ExprKind::Ident(name) => {
            // Closure-bound? Function name? Then not free.
            if bound.iter().any(|scope| scope.iter().any(|n| n == name))
                || out.iter().any(|n| n == name)
            {
                return;
            }
            // Skip names registered as functions / constants at module
            // scope — these are resolved at codegen time, not captured.
            // We don't have access to fn_index here, so the conservative
            // rule is "any unbound name is a candidate capture." The
            // outer compiler's `lookup` filters: names that resolve to
            // a local register get captured; names that don't (function
            // / constant) get skipped silently in `compile_expr`.
            out.push(name.clone());
        }
        ExprKind::Binary(_, l, r) => {
            walk_free_vars(l, bound, out);
            walk_free_vars(r, bound, out);
        }
        ExprKind::Unary(_, x) => walk_free_vars(x, bound, out),
        ExprKind::Call(callee, args) => {
            walk_free_vars(callee, bound, out);
            for a in args {
                walk_free_vars(a, bound, out);
            }
        }
        ExprKind::Field(x, _) => walk_free_vars(x, bound, out),
        ExprKind::Index(x, i) => {
            walk_free_vars(x, bound, out);
            walk_free_vars(i, bound, out);
        }
        ExprKind::StructLit(_, fields) => {
            for (_, v) in fields {
                walk_free_vars(v, bound, out);
            }
        }
        ExprKind::ArrayLit(elems) => {
            for el in elems {
                walk_free_vars(el, bound, out);
            }
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                walk_free_vars(k, bound, out);
                walk_free_vars(v, bound, out);
            }
        }
        ExprKind::Closure(inner_params, inner_body) => {
            // Nested closure: its params shadow our scope; we continue
            // collecting free vars from the inner body, accumulating
            // through the existing `bound` stack.
            bound.push(inner_params.iter().map(|p| p.name.clone()).collect());
            walk_free_vars(inner_body, bound, out);
            bound.pop();
        }
        ExprKind::Block(b) => walk_free_vars_block(b, bound, out),
        ExprKind::If(c, t, el) => {
            walk_free_vars(c, bound, out);
            walk_free_vars_block(t, bound, out);
            if let Some(el) = el {
                walk_free_vars_block(el, bound, out);
            }
        }
    }
}

fn walk_free_vars_block(b: &Block, bound: &mut Vec<Vec<String>>, out: &mut Vec<String>) {
    // Each block opens a new lexical scope; inner `let` statements add
    // to the scope; the scope dies with the block.
    bound.push(Vec::new());
    for s in &b.stmts {
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                walk_free_vars(init, bound, out);
                bound.last_mut().unwrap().push(name.clone());
            }
            StmtKind::Assign(target, value) => {
                walk_free_vars(target, bound, out);
                walk_free_vars(value, bound, out);
            }
            StmtKind::Expr(e) => walk_free_vars(e, bound, out),
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    walk_free_vars(e, bound, out);
                }
            }
            StmtKind::While(cond, body) => {
                walk_free_vars(cond, bound, out);
                walk_free_vars_block(body, bound, out);
            }
            StmtKind::If(cond, t, el) => {
                walk_free_vars(cond, bound, out);
                walk_free_vars_block(t, bound, out);
                if let Some(el) = el {
                    walk_free_vars_block(el, bound, out);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
        }
    }
    if let Some(t) = &b.tail {
        walk_free_vars(t, bound, out);
    }
    bound.pop();
}

/// Compile a closure body as a fresh top-level function. The callee
/// frame's register layout is `[captures.., params..]` — registers
/// `0..k` hold the captured values supplied by `ClosureMake`, and
/// registers `k..k+n` hold the call arguments supplied by `CallClosure`.
/// Returns the index of the pushed function in the module builder.
fn compile_closure_body(
    outer: &Compiler<'_>,
    captures: &[String],
    params: &[Param],
    body: &Expr,
) -> u16 {
    // The closure-body compiler shares the outer module's builder so
    // function-id allocations stay coherent (any recursive call inside
    // the closure to a top-level fn resolves through the shared
    // `fn_index`).
    let fn_index = outer.fn_index;
    let source = outer.source;
    // Synthesise a unique name for the closure function; not user-
    // visible but used for diagnostics.
    let synth_name = format!("__closure_{}", outer.builder.function_count());
    // We need a mutable borrow of `outer.builder` and a fresh Compiler.
    // SAFETY: the outer Compiler does not concurrently use its builder
    // inside this function's lifetime — `compile_expr` returns control
    // once it has resolved the closure subtree, and `ClosureMake` is
    // emitted *after* this call returns.
    let builder: &mut ModuleBuilder = unsafe {
        // `outer` is &Compiler; `builder` is &mut inside it. We can't
        // hold two &mut references at once safely from a single Rust
        // type-system view, so this is the one carve-out. The pattern
        // mirrors the parent codegen's recursive `compile_fn`-on-
        // sub-AST style; revisit when the codegen acquires a proper
        // reborrowable arena.
        &mut *(outer.builder as *const ModuleBuilder as *mut ModuleBuilder)
    };
    let mut inner = Compiler::new(builder, fn_index, source);
    // Register layout: captures first, then params.
    for cap_name in captures {
        let r = inner.alloc_reg();
        inner.bind(cap_name.clone(), r);
    }
    for p in params {
        let r = inner.alloc_reg();
        inner.bind(p.name.clone(), r);
    }
    // Compile body and emit a return for the produced value.
    let body_reg = compile_expr(&mut inner, body);
    inner.emit_at(Opcode::ReturnVal, body.span);
    inner.emit_byte(body_reg);
    let bc = FunctionBytecode {
        name: synth_name,
        arity: (captures.len() + params.len()) as u8,
        max_register: inner.next_reg.min(256) as u8,
        code: inner.code,
        line_for_pc: inner.line_for_pc,
    };
    let fn_idx = builder.function_count();
    builder.push_function(bc);
    fn_idx as u16
}
