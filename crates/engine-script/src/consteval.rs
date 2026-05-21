//! Const-expression evaluator (spec IV.7).
//!
//! `const` initializers must reduce to a single value at compile time.
//! Phase 4 supports literals, references to previously-defined consts,
//! and arithmetic / comparison / logical operators on those — enough for
//! the editor Console's `CONSTEVAL` mode to evaluate user-typed
//! expressions against the module's symbol table (the UI for that mode
//! is Phase 10; the evaluator itself ships here).
//!
//! Anything that requires runtime state (function calls, FFI, mutable
//! access, struct construction) is rejected with a diagnostic.

use crate::ast::{BinOp, Decl, Expr, ExprKind, Lit, Module, UnOp};
use crate::diag::{Diagnostic, Diagnostics};
use engine_core::collections::HashMap;

/// A fully-reduced constant value.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    /// `nil`
    Nil,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Float — stored as a bit pattern for total equality.
    Float(u64),
    /// String literal.
    Str(String),
}

/// Const-eval environment: name -> value table built from the module's
/// `const` declarations.
#[derive(Debug, Default)]
pub struct ConstEnv {
    /// Already-evaluated const bindings.
    pub bindings: HashMap<String, ConstValue>,
}

/// Evaluates every `const` declaration in `module` and populates the
/// environment. Errors are emitted into `diags`.
pub fn eval_consts(module: &Module, diags: &mut Diagnostics) -> ConstEnv {
    let mut env = ConstEnv::default();
    for d in &module.decls {
        if let Decl::Const(c) = d {
            match eval(&c.init, &env) {
                Ok(v) => {
                    env.bindings.insert(c.name.clone(), v);
                }
                Err(msg) => {
                    diags.emit(Diagnostic::error(c.init.span, msg));
                }
            }
        }
    }
    env
}

/// Evaluates `expr` against `env`. Returns a value or a diagnostic message.
pub fn eval(expr: &Expr, env: &ConstEnv) -> Result<ConstValue, String> {
    match &expr.kind {
        ExprKind::Lit(l) => Ok(match l {
            Lit::Nil => ConstValue::Nil,
            Lit::Bool(v) => ConstValue::Bool(*v),
            Lit::Int(v) => ConstValue::Int(*v),
            Lit::Float(b) => ConstValue::Float(*b),
            Lit::Str(s) => ConstValue::Str(s.clone()),
        }),
        ExprKind::Ident(n) => env
            .bindings
            .get(n)
            .cloned()
            .ok_or_else(|| format!("name `{n}` is not a constant")),
        ExprKind::Binary(op, l, r) => {
            let lv = eval(l, env)?;
            let rv = eval(r, env)?;
            apply_binop(*op, &lv, &rv)
        }
        ExprKind::Unary(op, x) => {
            let xv = eval(x, env)?;
            apply_unop(*op, &xv)
        }
        _ => Err("not a constant expression".to_string()),
    }
}

fn apply_binop(op: BinOp, l: &ConstValue, r: &ConstValue) -> Result<ConstValue, String> {
    use ConstValue::*;
    Ok(match (l, r) {
        (Int(a), Int(b)) => match op {
            BinOp::Add => Int(a.wrapping_add(*b)),
            BinOp::Sub => Int(a.wrapping_sub(*b)),
            BinOp::Mul => Int(a.wrapping_mul(*b)),
            BinOp::Div => Int(a.checked_div(*b).ok_or("division by zero")?),
            BinOp::Mod => Int(a.checked_rem(*b).ok_or("modulo by zero")?),
            BinOp::Eq => Bool(a == b),
            BinOp::Ne => Bool(a != b),
            BinOp::Lt => Bool(a < b),
            BinOp::Le => Bool(a <= b),
            BinOp::Gt => Bool(a > b),
            BinOp::Ge => Bool(a >= b),
            _ => return Err("invalid operator on integers".into()),
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
                _ => return Err("invalid operator on floats".into()),
            }
        }
        (Bool(a), Bool(b)) => match op {
            BinOp::And => Bool(*a && *b),
            BinOp::Or => Bool(*a || *b),
            BinOp::Eq => Bool(a == b),
            BinOp::Ne => Bool(a != b),
            _ => return Err("invalid operator on booleans".into()),
        },
        _ => return Err("type mismatch in constant expression".into()),
    })
}

fn apply_unop(op: UnOp, x: &ConstValue) -> Result<ConstValue, String> {
    use ConstValue::*;
    Ok(match (op, x) {
        (UnOp::Neg, Int(v)) => Int(v.wrapping_neg()),
        (UnOp::Neg, Float(b)) => Float((-f64::from_bits(*b)).to_bits()),
        (UnOp::Not, Bool(v)) => Bool(!v),
        _ => return Err("invalid unary operator".into()),
    })
}
