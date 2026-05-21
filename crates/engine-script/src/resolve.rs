//! Name resolution.
//!
//! Walks the AST once, recording the kind each name resolves to so the
//! type-checker can answer "is `foo` a local, a parameter, a function, a
//! struct, or a constant?" without a second pass. Scopes are nested
//! lexically; a `let` shadowing an outer name is permitted.

use crate::ast::{Decl, Expr, ExprKind, Module, Stmt, StmtKind};
use crate::diag::{Diagnostic, Diagnostics};
use engine_core::collections::HashMap;

/// What a name resolves to in source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sym {
    /// A local `let` binding or function parameter.
    Local,
    /// A top-level `fn` declaration.
    Fn,
    /// A top-level `struct` declaration.
    Struct,
    /// A top-level `const` declaration.
    Const,
}

/// Name-resolution result for one module.
#[derive(Debug, Default)]
pub struct Resolution {
    /// Top-level symbol table.
    pub globals: HashMap<String, Sym>,
}

impl Resolution {
    /// Looks up a top-level name.
    pub fn lookup(&self, name: &str) -> Option<Sym> {
        self.globals.get(name).copied()
    }
}

/// Resolves the module, returning the global table and emitting any
/// undefined-name diagnostics found inside function bodies.
pub fn resolve(module: &Module, diags: &mut Diagnostics) -> Resolution {
    let mut res = Resolution::default();
    // Pass 1: globals.
    for d in &module.decls {
        match d {
            Decl::Fn(f) => {
                res.globals.insert(f.name.clone(), Sym::Fn);
            }
            Decl::Struct(s) => {
                res.globals.insert(s.name.clone(), Sym::Struct);
            }
            Decl::Const(c) => {
                res.globals.insert(c.name.clone(), Sym::Const);
            }
        }
    }
    // Pass 2: check name references inside function bodies.
    for d in &module.decls {
        if let Decl::Fn(f) = d {
            let mut env = LocalEnv::new();
            env.push();
            for p in &f.params {
                env.insert(p.name.clone());
            }
            walk_block_for_resolution(&f.body, &mut env, &res, diags);
            env.pop();
        }
    }
    res
}

struct LocalEnv {
    scopes: Vec<HashMap<String, ()>>,
}

impl LocalEnv {
    fn new() -> Self {
        Self { scopes: Vec::new() }
    }

    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn insert(&mut self, name: String) {
        self.scopes.last_mut().unwrap().insert(name, ());
    }

    fn contains(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|s| s.contains_key(name))
    }
}

fn walk_block_for_resolution(
    b: &crate::ast::Block,
    env: &mut LocalEnv,
    res: &Resolution,
    diags: &mut Diagnostics,
) {
    env.push();
    for s in &b.stmts {
        walk_stmt(s, env, res, diags);
    }
    if let Some(t) = &b.tail {
        walk_expr(t, env, res, diags);
    }
    env.pop();
}

fn walk_stmt(s: &Stmt, env: &mut LocalEnv, res: &Resolution, diags: &mut Diagnostics) {
    match &s.kind {
        StmtKind::Let { name, init, .. } => {
            walk_expr(init, env, res, diags);
            env.insert(name.clone());
        }
        StmtKind::Assign(p, v) => {
            walk_expr(p, env, res, diags);
            walk_expr(v, env, res, diags);
        }
        StmtKind::Expr(e) => walk_expr(e, env, res, diags),
        StmtKind::Return(Some(e)) => walk_expr(e, env, res, diags),
        StmtKind::Return(None) => {}
        StmtKind::While(c, b) => {
            walk_expr(c, env, res, diags);
            walk_block_for_resolution(b, env, res, diags);
        }
        StmtKind::If(c, t, e) => {
            walk_expr(c, env, res, diags);
            walk_block_for_resolution(t, env, res, diags);
            if let Some(e) = e {
                walk_block_for_resolution(e, env, res, diags);
            }
        }
        StmtKind::Break | StmtKind::Continue => {}
    }
}

fn walk_expr(e: &Expr, env: &mut LocalEnv, res: &Resolution, diags: &mut Diagnostics) {
    match &e.kind {
        ExprKind::Lit(_) => {}
        ExprKind::Ident(name) => {
            if !env.contains(name) && res.lookup(name).is_none() {
                diags.emit(Diagnostic::error(
                    e.span,
                    format!("undefined name `{name}`"),
                ));
            }
        }
        ExprKind::Binary(_, l, r) => {
            walk_expr(l, env, res, diags);
            walk_expr(r, env, res, diags);
        }
        ExprKind::Unary(_, x) => walk_expr(x, env, res, diags),
        ExprKind::Call(c, args) => {
            walk_expr(c, env, res, diags);
            for a in args {
                walk_expr(a, env, res, diags);
            }
        }
        ExprKind::Field(x, _) => walk_expr(x, env, res, diags),
        ExprKind::Index(x, i) => {
            walk_expr(x, env, res, diags);
            walk_expr(i, env, res, diags);
        }
        ExprKind::StructLit(name, fs) => {
            if res.lookup(name) != Some(Sym::Struct) {
                diags.emit(Diagnostic::error(
                    e.span,
                    format!("undefined struct `{name}`"),
                ));
            }
            for (_, v) in fs {
                walk_expr(v, env, res, diags);
            }
        }
        ExprKind::Closure(ps, body) => {
            env.push();
            for p in ps {
                env.insert(p.name.clone());
            }
            walk_expr(body, env, res, diags);
            env.pop();
        }
        ExprKind::Block(b) => walk_block_for_resolution(b, env, res, diags),
        ExprKind::If(c, t, el) => {
            walk_expr(c, env, res, diags);
            walk_block_for_resolution(t, env, res, diags);
            if let Some(el) = el {
                walk_block_for_resolution(el, env, res, diags);
            }
        }
    }
}
