//! Watch-expression purity verifier and evaluator (ADR-036).
//!
//! Watch expressions ride the same parser as the rest of sli but
//! must reduce to a pure value — no side effects, no FFI, no
//! function calls outside an allowlist of stdlib pure builtins.
//! The verifier walks the parsed [`Expr`] and rejects anything that
//! could mutate state; the evaluator then uses
//! [`crate::consteval::eval`] against the paused frame's locals.

use crate::ast::{Expr, ExprKind};
use crate::consteval::{ConstEnv, ConstValue};
use crate::diag::Diagnostics;
use crate::lex::lex;
use crate::parse::parse;
use crate::source::{FileId, Source};

/// Why a watch expression was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchError {
    /// The source failed to parse.
    ParseFailed,
    /// The source parsed but the verifier rejected it.
    Impure {
        /// Human-readable reason.
        reason: String,
    },
    /// Evaluation failed at runtime.
    EvalFailed {
        /// Human-readable reason.
        reason: String,
    },
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseFailed => write!(f, "watch expression failed to parse"),
            Self::Impure { reason } => write!(f, "watch expression is impure: {reason}"),
            Self::EvalFailed { reason } => write!(f, "watch evaluation failed: {reason}"),
        }
    }
}

impl std::error::Error for WatchError {}

/// Pure stdlib builtins watch expressions may invoke. PR 3 leaves this
/// empty — every call is rejected. The list grows as `Math.*`, `Str.*`
/// builtins land.
pub const PURE_BUILTINS: &[&str] = &[];

/// Compiles `source` into a parsed expression, then runs the purity
/// verifier. Returns the parsed expression on success.
pub fn validate(source: &str) -> Result<Expr, WatchError> {
    let src = Source::new("<watch>", source);
    let mut diags = Diagnostics::new();
    let toks = lex(FileId(0), &src, &mut diags);
    let module = parse(&toks, &mut diags);
    if diags.has_errors() || module.decls.is_empty() {
        // Fallback: treat the whole source as a single expression by
        // wrapping in `fn _w() -> nil { _; }`. This is only used as a
        // last resort if the user typed an isolated expression — the
        // primary form expects a parseable program.
        let wrapped = format!("fn _w() {{ {source}; }}");
        let src2 = Source::new("<watch>", &wrapped);
        let mut diags2 = Diagnostics::new();
        let toks2 = lex(FileId(0), &src2, &mut diags2);
        let module2 = parse(&toks2, &mut diags2);
        if diags2.has_errors() {
            return Err(WatchError::ParseFailed);
        }
        if let Some(crate::ast::Decl::Fn(f)) = module2.decls.into_iter().next()
            && let Some(crate::ast::Stmt {
                kind: crate::ast::StmtKind::Expr(e),
                ..
            }) = f.body.stmts.into_iter().next()
        {
            verify_pure(&e)?;
            return Ok(e);
        }
        return Err(WatchError::ParseFailed);
    }
    // If the user supplied a full module, pick the first Fn's body's
    // tail or first expression stmt.
    for d in module.decls {
        if let crate::ast::Decl::Fn(f) = d {
            if let Some(tail) = f.body.tail {
                verify_pure(&tail)?;
                return Ok(tail);
            }
            for s in f.body.stmts {
                if let crate::ast::StmtKind::Expr(e) = s.kind {
                    verify_pure(&e)?;
                    return Ok(e);
                }
            }
        }
    }
    Err(WatchError::ParseFailed)
}

/// Walks `expr`, rejecting any node that could observably mutate
/// state. This is the same rule the debugger uses to gate watches.
pub fn verify_pure(expr: &Expr) -> Result<(), WatchError> {
    match &expr.kind {
        ExprKind::Lit(_) | ExprKind::Ident(_) => Ok(()),
        ExprKind::Binary(_, l, r) => {
            verify_pure(l)?;
            verify_pure(r)
        }
        ExprKind::Unary(_, x) => verify_pure(x),
        ExprKind::Field(x, _) => verify_pure(x),
        ExprKind::Index(x, i) => {
            verify_pure(x)?;
            verify_pure(i)
        }
        ExprKind::Block(b) => {
            for s in &b.stmts {
                match &s.kind {
                    crate::ast::StmtKind::Let { init, .. } => verify_pure(init)?,
                    crate::ast::StmtKind::Expr(e) => verify_pure(e)?,
                    _ => {
                        return Err(WatchError::Impure {
                            reason: "statements other than `let`/expr are not allowed".into(),
                        });
                    }
                }
            }
            if let Some(t) = &b.tail {
                verify_pure(t)?;
            }
            Ok(())
        }
        ExprKind::If(c, t, e) => {
            verify_pure(c)?;
            // Reuse the block-purity rule.
            verify_pure(&Expr {
                kind: ExprKind::Block(t.clone()),
                span: expr.span,
                ty: expr.ty.clone(),
            })?;
            if let Some(e) = e {
                verify_pure(&Expr {
                    kind: ExprKind::Block(e.clone()),
                    span: expr.span,
                    ty: expr.ty.clone(),
                })?;
            }
            Ok(())
        }
        ExprKind::StructLit(_, fs) => {
            for (_, v) in fs {
                verify_pure(v)?;
            }
            Ok(())
        }
        ExprKind::ArrayLit(elems) => {
            for el in elems {
                verify_pure(el)?;
            }
            Ok(())
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                verify_pure(k)?;
                verify_pure(v)?;
            }
            Ok(())
        }
        ExprKind::Closure(_, _) => Err(WatchError::Impure {
            reason: "closures are not allowed in watch expressions".into(),
        }),
        ExprKind::Call(callee, _) => {
            // Allow only calls to PURE_BUILTINS, identified by name.
            if let ExprKind::Ident(name) = &callee.kind
                && PURE_BUILTINS.contains(&name.as_str())
            {
                return Ok(());
            }
            Err(WatchError::Impure {
                reason: "function calls (including FFI) are not allowed".into(),
            })
        }
    }
}

/// Evaluates a previously-validated expression against `env`. Errors
/// surface as [`WatchError::EvalFailed`].
pub fn evaluate(expr: &Expr, env: &ConstEnv) -> Result<ConstValue, WatchError> {
    crate::consteval::eval(expr, env).map_err(|reason| WatchError::EvalFailed { reason })
}
