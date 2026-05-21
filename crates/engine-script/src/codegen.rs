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
    BinOp, Block, Decl, Expr, ExprKind, FnDecl, Lit, Module as AstModule, Stmt, StmtKind, UnOp,
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
            let dst = c.alloc_reg();
            if let ExprKind::Ident(name) = &callee.kind
                && let Some(id) = c.fn_index.get(name)
            {
                c.emit_at(Opcode::Call, e.span);
                c.emit_byte(dst);
                c.emit_u16(*id);
                c.emit_byte(arg_regs.len() as u8);
                for r in &arg_regs {
                    c.emit_byte(*r);
                }
                return dst;
            }
            // Unknown name — emit nil for now. PR 3 will route this
            // through the FFI table.
            c.emit_at(Opcode::ConstNil, e.span);
            c.emit_byte(dst);
            dst
        }
        ExprKind::Field(_, _) | ExprKind::Index(_, _) | ExprKind::StructLit(_, _) => {
            // Struct field access and literal construction are
            // PR-3 work (need heap allocation through `Heap::alloc`).
            // Lower to a nil placeholder; the type checker has already
            // validated the shape statically.
            let dst = c.alloc_reg();
            c.emit_at(Opcode::ConstNil, e.span);
            c.emit_byte(dst);
            dst
        }
        ExprKind::Closure(_, _) => {
            let dst = c.alloc_reg();
            c.emit_at(Opcode::ConstNil, e.span);
            c.emit_byte(dst);
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
