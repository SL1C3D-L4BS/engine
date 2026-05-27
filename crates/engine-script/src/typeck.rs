//! Type checker for sli.
//!
//! Bottom-up checking: every expression is given a type by walking the AST,
//! with limited numeric promotion (an integer literal unifies with any of
//! the four numeric types; otherwise types must match exactly). Struct
//! field access, function calls, and ECS generics (`Query<T>`, `Res<T>`,
//! `ResMut<T>`, `Entity`) are recognised.
//!
//! The checker mutates the type slots in [`crate::ast::Expr`]. Errors are
//! reported as diagnostics; the checker continues past errors so the editor
//! sees every problem in one round.

use crate::ast::{
    BinOp, Block, Decl, Expr, ExprKind, FnDecl, Lit, Module, Stmt, StmtKind, StructDecl, Type, UnOp,
};
use crate::diag::{Diagnostic, Diagnostics};
use engine_core::collections::HashMap;

/// Per-module symbol table built by the checker — function signatures,
/// struct layouts, and const types.
#[derive(Debug, Default)]
pub struct TypeTable {
    /// `name -> (param types, return type)`
    pub functions: HashMap<String, (Vec<Type>, Type)>,
    /// `name -> field list`
    pub structs: HashMap<String, Vec<(String, Type)>>,
    /// `name -> declared type`
    pub constants: HashMap<String, Type>,
}

/// Public entry point: typechecks `module` in place.
pub fn check(module: &mut Module, diags: &mut Diagnostics) -> TypeTable {
    let mut table = TypeTable::default();
    for d in &module.decls {
        match d {
            Decl::Fn(f) => {
                let params = f.params.iter().map(|p| p.ty.clone()).collect();
                table
                    .functions
                    .insert(f.name.clone(), (params, f.ret.clone()));
            }
            Decl::Struct(s) => {
                let fields = s
                    .fields
                    .iter()
                    .map(|(n, t, _)| (n.clone(), t.clone()))
                    .collect();
                table.structs.insert(s.name.clone(), fields);
            }
            Decl::Const(c) => {
                table.constants.insert(c.name.clone(), c.ty.clone());
            }
        }
    }
    for d in &module.decls {
        if let Decl::Struct(s) = d {
            check_struct_fields(s, &table, diags);
        }
    }
    for d in &mut module.decls {
        if let Decl::Fn(f) = d {
            let mut chk = Checker {
                table: &table,
                diags,
                locals: vec![HashMap::new()],
                ret_ty: f.ret.clone(),
            };
            chk.check_fn(f);
        }
    }
    table
}

fn check_struct_fields(s: &StructDecl, t: &TypeTable, diags: &mut Diagnostics) {
    for (_, ty, span) in &s.fields {
        validate_type(ty, t, *span, diags);
    }
}

fn validate_type(ty: &Type, t: &TypeTable, span: crate::source::Span, diags: &mut Diagnostics) {
    match ty {
        Type::Struct(n) if !t.structs.contains_key(n) => {
            diags.emit(Diagnostic::error(span, format!("unknown type `{n}`")));
        }
        Type::Query(inner) | Type::Res(inner) | Type::ResMut(inner) | Type::Array(inner) => {
            validate_type(inner, t, span, diags);
        }
        Type::Map(k, v) => {
            validate_type(k, t, span, diags);
            validate_type(v, t, span, diags);
        }
        Type::Fn(ps, r) => {
            for p in ps {
                validate_type(p, t, span, diags);
            }
            validate_type(r, t, span, diags);
        }
        _ => {}
    }
}

struct Checker<'a> {
    table: &'a TypeTable,
    diags: &'a mut Diagnostics,
    locals: Vec<HashMap<String, Type>>,
    ret_ty: Type,
}

impl<'a> Checker<'a> {
    fn check_fn(&mut self, f: &mut FnDecl) {
        for p in &f.params {
            validate_type(&p.ty, self.table, p.span, self.diags);
            self.locals
                .last_mut()
                .unwrap()
                .insert(p.name.clone(), p.ty.clone());
        }
        validate_type(&f.ret, self.table, f.span, self.diags);
        let body_ty = self.check_block(&mut f.body);
        if !matches!(f.ret, Type::Nil)
            && !matches!(body_ty, Type::Error)
            && !type_eq(&body_ty, &f.ret)
            && !matches!(body_ty, Type::Nil)
        {
            self.diags.emit(Diagnostic::error(
                f.body.span,
                format!(
                    "function `{}` returns `{}`, but body produces `{}`",
                    f.name,
                    fmt_type(&f.ret),
                    fmt_type(&body_ty)
                ),
            ));
        }
    }

    fn check_block(&mut self, b: &mut Block) -> Type {
        self.locals.push(HashMap::new());
        for s in &mut b.stmts {
            self.check_stmt(s);
        }
        let out = if let Some(tail) = &mut b.tail {
            self.check_expr(tail)
        } else {
            Type::Nil
        };
        self.locals.pop();
        out
    }

    fn check_stmt(&mut self, s: &mut Stmt) {
        match &mut s.kind {
            StmtKind::Let {
                name,
                ty,
                mutable: _,
                init,
            } => {
                let hint = if matches!(ty, Type::Unknown) {
                    None
                } else {
                    Some(ty.clone())
                };
                let init_ty = self.check_expr_hinted(init, hint.as_ref());
                let final_ty = if matches!(ty, Type::Unknown) {
                    if matches!(init_ty, Type::Error) {
                        Type::Error
                    } else {
                        init_ty.clone()
                    }
                } else {
                    if !unifies(&init_ty, ty) {
                        self.diags.emit(Diagnostic::error(
                            init.span,
                            format!(
                                "expected `{}`, found `{}`",
                                fmt_type(ty),
                                fmt_type(&init_ty)
                            ),
                        ));
                    }
                    ty.clone()
                };
                *ty = final_ty.clone();
                self.locals
                    .last_mut()
                    .unwrap()
                    .insert(name.clone(), final_ty);
            }
            StmtKind::Assign(target, value) => {
                let target_ty = self.check_expr(target);
                let value_ty = self.check_expr(value);
                if !unifies(&value_ty, &target_ty) {
                    self.diags.emit(Diagnostic::error(
                        s.span,
                        format!(
                            "cannot assign `{}` to `{}`",
                            fmt_type(&value_ty),
                            fmt_type(&target_ty)
                        ),
                    ));
                }
            }
            StmtKind::Expr(e) => {
                self.check_expr(e);
            }
            StmtKind::Return(None) => {
                if !matches!(self.ret_ty, Type::Nil) {
                    self.diags.emit(Diagnostic::error(
                        s.span,
                        format!("expected return value of type `{}`", fmt_type(&self.ret_ty)),
                    ));
                }
            }
            StmtKind::Return(Some(e)) => {
                let ret_ty = self.ret_ty.clone();
                let et = self.check_expr_hinted(e, Some(&ret_ty));
                if !unifies(&et, &ret_ty) {
                    self.diags.emit(Diagnostic::error(
                        s.span,
                        format!(
                            "return type mismatch: expected `{}`, found `{}`",
                            fmt_type(&ret_ty),
                            fmt_type(&et)
                        ),
                    ));
                }
            }
            StmtKind::While(c, body) => {
                let ct = self.check_expr(c);
                if !matches!(ct, Type::Bool | Type::Error) {
                    self.diags.emit(Diagnostic::error(
                        c.span,
                        format!(
                            "`while` condition must be `bool`, found `{}`",
                            fmt_type(&ct)
                        ),
                    ));
                }
                self.check_block(body);
            }
            StmtKind::If(c, then, el) => {
                let ct = self.check_expr(c);
                if !matches!(ct, Type::Bool | Type::Error) {
                    self.diags.emit(Diagnostic::error(
                        c.span,
                        format!("`if` condition must be `bool`, found `{}`", fmt_type(&ct)),
                    ));
                }
                self.check_block(then);
                if let Some(e) = el {
                    self.check_block(e);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
        }
    }

    fn check_expr(&mut self, e: &mut Expr) -> Type {
        self.check_expr_hinted(e, None)
    }

    fn check_expr_hinted(&mut self, e: &mut Expr, hint: Option<&Type>) -> Type {
        let ty = match &mut e.kind {
            ExprKind::Lit(l) => match l {
                Lit::Nil => Type::Nil,
                Lit::Bool(_) => Type::Bool,
                // Integer literals are polymorphic: they take whichever
                // of the four numeric types the context calls for.
                Lit::Int(_) => match hint {
                    Some(Type::I32) => Type::I32,
                    Some(Type::I64) => Type::I64,
                    Some(Type::F32) => Type::F32,
                    Some(Type::F64) => Type::F64,
                    _ => Type::I64,
                },
                Lit::Float(_) => match hint {
                    Some(Type::F32) => Type::F32,
                    Some(Type::F64) => Type::F64,
                    _ => Type::F64,
                },
                Lit::Str(_) => Type::Str,
            },
            ExprKind::Ident(name) => self
                .locals
                .iter()
                .rev()
                .find_map(|s| s.get(name).cloned())
                .or_else(|| self.table.constants.get(name).cloned())
                .or_else(|| {
                    self.table
                        .functions
                        .get(name)
                        .map(|(ps, r)| Type::Fn(ps.clone(), Box::new(r.clone())))
                })
                .unwrap_or_else(|| {
                    self.diags.emit(Diagnostic::error(
                        e.span,
                        format!("undefined name `{name}`"),
                    ));
                    Type::Error
                }),
            ExprKind::Binary(op, l, r) => {
                let lt = self.check_expr(l);
                let rt = self.check_expr(r);
                check_binop(*op, &lt, &rt, e.span, self.diags)
            }
            ExprKind::Unary(op, x) => {
                let xt = self.check_expr(x);
                match (op, &xt) {
                    (UnOp::Neg, ty) if ty.is_numeric() => ty.clone(),
                    (UnOp::Not, Type::Bool) => Type::Bool,
                    (_, Type::Error) => Type::Error,
                    _ => {
                        self.diags.emit(Diagnostic::error(
                            e.span,
                            format!("invalid unary operator for `{}`", fmt_type(&xt)),
                        ));
                        Type::Error
                    }
                }
            }
            ExprKind::Call(callee, args) => {
                let ct = self.check_expr(callee);
                let mut arg_types = Vec::new();
                let expected_params: Vec<Type> = match &ct {
                    Type::Fn(ps, _) => ps.clone(),
                    _ => Vec::new(),
                };
                for (i, a) in args.iter_mut().enumerate() {
                    let hint = expected_params.get(i);
                    arg_types.push(self.check_expr_hinted(a, hint));
                }
                match ct {
                    Type::Fn(ps, r) => {
                        if ps.len() != arg_types.len() {
                            self.diags.emit(Diagnostic::error(
                                e.span,
                                format!(
                                    "function expects {} argument(s), found {}",
                                    ps.len(),
                                    arg_types.len()
                                ),
                            ));
                        } else {
                            for (i, (p, a)) in ps.iter().zip(arg_types.iter()).enumerate() {
                                if !unifies(a, p) {
                                    self.diags.emit(Diagnostic::error(
                                        args[i].span,
                                        format!(
                                            "argument {} expected `{}`, found `{}`",
                                            i + 1,
                                            fmt_type(p),
                                            fmt_type(a)
                                        ),
                                    ));
                                }
                            }
                        }
                        *r
                    }
                    Type::Error => Type::Error,
                    other => {
                        self.diags.emit(Diagnostic::error(
                            e.span,
                            format!("cannot call value of type `{}`", fmt_type(&other)),
                        ));
                        Type::Error
                    }
                }
            }
            ExprKind::Field(rcv, name) => {
                let rt = self.check_expr(rcv);
                match &rt {
                    Type::Struct(sn) => match self.table.structs.get(sn) {
                        Some(fields) => match fields.iter().find(|(n, _)| n == name) {
                            Some((_, ft)) => ft.clone(),
                            None => {
                                self.diags.emit(Diagnostic::error(
                                    e.span,
                                    format!("struct `{sn}` has no field `{name}`"),
                                ));
                                Type::Error
                            }
                        },
                        None => Type::Error,
                    },
                    Type::Error => Type::Error,
                    _ => {
                        self.diags.emit(Diagnostic::error(
                            e.span,
                            format!("`{}` is not a struct", fmt_type(&rt)),
                        ));
                        Type::Error
                    }
                }
            }
            ExprKind::Index(rcv, idx) => {
                let rt = self.check_expr(rcv);
                let it = self.check_expr(idx);
                match &rt {
                    Type::Array(inner) => {
                        if !it.is_integer() && !matches!(it, Type::Error) {
                            self.diags.emit(Diagnostic::error(
                                idx.span,
                                format!("array index must be integer, found `{}`", fmt_type(&it)),
                            ));
                        }
                        (**inner).clone()
                    }
                    Type::Map(k, v) => {
                        if !unifies(&it, k) {
                            self.diags.emit(Diagnostic::error(
                                idx.span,
                                format!(
                                    "map index expected `{}`, found `{}`",
                                    fmt_type(k),
                                    fmt_type(&it)
                                ),
                            ));
                        }
                        (**v).clone()
                    }
                    Type::Error => Type::Error,
                    _ => {
                        self.diags.emit(Diagnostic::error(
                            e.span,
                            format!("cannot index value of type `{}`", fmt_type(&rt)),
                        ));
                        Type::Error
                    }
                }
            }
            ExprKind::StructLit(name, fields) => match self.table.structs.get(name).cloned() {
                Some(decl_clone) => {
                    let span = e.span;
                    let mut lit_types = Vec::new();
                    for (fname, fexpr) in fields.iter_mut() {
                        let expected = decl_clone
                            .iter()
                            .find(|(n, _)| n == fname)
                            .map(|(_, t)| t.clone());
                        let ft = self.check_expr_hinted(fexpr, expected.as_ref());
                        lit_types.push((fname.clone(), ft, fexpr.span));
                    }
                    for (decl_name, decl_ty) in &decl_clone {
                        match lit_types.iter().find(|(n, _, _)| n == decl_name) {
                            Some((_, lt, span)) => {
                                if !unifies(lt, decl_ty) {
                                    self.diags.emit(Diagnostic::error(
                                        *span,
                                        format!(
                                            "field `{}` expected `{}`, found `{}`",
                                            decl_name,
                                            fmt_type(decl_ty),
                                            fmt_type(lt)
                                        ),
                                    ));
                                }
                            }
                            None => {
                                self.diags.emit(Diagnostic::error(
                                    span,
                                    format!("missing field `{decl_name}` in struct literal"),
                                ));
                            }
                        }
                    }
                    Type::Struct(name.clone())
                }
                None => {
                    self.diags.emit(Diagnostic::error(
                        e.span,
                        format!("unknown struct `{name}`"),
                    ));
                    Type::Error
                }
            },
            ExprKind::ArrayLit(elems) => {
                // Inference: the first element's type pins the array's
                // element type; subsequent elements must unify. Empty
                // `[]` falls back to the hint (e.g. from `let xs: Array<i32> = [];`),
                // otherwise `Type::Error` with a diagnostic.
                let span = e.span;
                let elem_hint: Option<&Type> = match hint {
                    Some(Type::Array(inner)) => Some(inner.as_ref()),
                    _ => None,
                };
                if elems.is_empty() {
                    match elem_hint {
                        Some(t) => Type::Array(Box::new(t.clone())),
                        None => {
                            self.diags.emit(Diagnostic::error(
                                span,
                                "empty array literal `[]` requires a type annotation".to_string(),
                            ));
                            Type::Error
                        }
                    }
                } else {
                    let first_ty = self.check_expr_hinted(&mut elems[0], elem_hint);
                    let element_ty = first_ty.clone();
                    for el in elems.iter_mut().skip(1) {
                        let t = self.check_expr_hinted(el, Some(&element_ty));
                        if !unifies(&t, &element_ty) {
                            self.diags.emit(Diagnostic::error(
                                el.span,
                                format!(
                                    "array element expected `{}`, found `{}`",
                                    fmt_type(&element_ty),
                                    fmt_type(&t)
                                ),
                            ));
                        }
                    }
                    Type::Array(Box::new(element_ty))
                }
            }
            ExprKind::MapLit(pairs) => {
                let span = e.span;
                let (k_hint, v_hint): (Option<&Type>, Option<&Type>) = match hint {
                    Some(Type::Map(k, v)) => (Some(k.as_ref()), Some(v.as_ref())),
                    _ => (None, None),
                };
                if pairs.is_empty() {
                    match (k_hint, v_hint) {
                        (Some(k), Some(v)) => Type::Map(Box::new(k.clone()), Box::new(v.clone())),
                        _ => {
                            self.diags.emit(Diagnostic::error(
                                span,
                                "empty map literal `[:]` requires a type annotation".to_string(),
                            ));
                            Type::Error
                        }
                    }
                } else {
                    let (first_k, first_v) = pairs.split_first_mut().unwrap();
                    let key_ty = self.check_expr_hinted(&mut first_k.0, k_hint);
                    let val_ty = self.check_expr_hinted(&mut first_k.1, v_hint);
                    for (k, v) in first_v.iter_mut() {
                        let kt = self.check_expr_hinted(k, Some(&key_ty));
                        if !unifies(&kt, &key_ty) {
                            self.diags.emit(Diagnostic::error(
                                k.span,
                                format!(
                                    "map key expected `{}`, found `{}`",
                                    fmt_type(&key_ty),
                                    fmt_type(&kt)
                                ),
                            ));
                        }
                        let vt = self.check_expr_hinted(v, Some(&val_ty));
                        if !unifies(&vt, &val_ty) {
                            self.diags.emit(Diagnostic::error(
                                v.span,
                                format!(
                                    "map value expected `{}`, found `{}`",
                                    fmt_type(&val_ty),
                                    fmt_type(&vt)
                                ),
                            ));
                        }
                    }
                    Type::Map(Box::new(key_ty), Box::new(val_ty))
                }
            }
            ExprKind::Closure(ps, body) => {
                self.locals.push(HashMap::new());
                for p in ps.iter() {
                    self.locals
                        .last_mut()
                        .unwrap()
                        .insert(p.name.clone(), p.ty.clone());
                }
                let bt = self.check_expr(body);
                self.locals.pop();
                Type::Fn(ps.iter().map(|p| p.ty.clone()).collect(), Box::new(bt))
            }
            ExprKind::Block(b) => self.check_block(b),
            ExprKind::If(c, then, el) => {
                let ct = self.check_expr(c);
                if !matches!(ct, Type::Bool | Type::Error) {
                    self.diags.emit(Diagnostic::error(
                        c.span,
                        format!("`if` condition must be `bool`, found `{}`", fmt_type(&ct)),
                    ));
                }
                let tt = self.check_block(then);
                let et = if let Some(e) = el {
                    self.check_block(e)
                } else {
                    Type::Nil
                };
                if type_eq(&tt, &et) {
                    tt
                } else if matches!(et, Type::Nil) || matches!(tt, Type::Nil) {
                    Type::Nil
                } else {
                    Type::Error
                }
            }
        };
        e.ty = ty.clone();
        ty
    }
}

fn check_binop(
    op: BinOp,
    l: &Type,
    r: &Type,
    span: crate::source::Span,
    diags: &mut Diagnostics,
) -> Type {
    if matches!(l, Type::Error) || matches!(r, Type::Error) {
        return Type::Error;
    }
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            if l.is_numeric() && type_eq(l, r) {
                l.clone()
            } else {
                diags.emit(Diagnostic::error(
                    span,
                    format!(
                        "arithmetic operator requires matching numeric types, got `{}` and `{}`",
                        fmt_type(l),
                        fmt_type(r)
                    ),
                ));
                Type::Error
            }
        }
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            if type_eq(l, r) {
                Type::Bool
            } else {
                diags.emit(Diagnostic::error(
                    span,
                    format!(
                        "comparison requires matching types, got `{}` and `{}`",
                        fmt_type(l),
                        fmt_type(r)
                    ),
                ));
                Type::Bool
            }
        }
        BinOp::And | BinOp::Or => {
            if matches!(l, Type::Bool) && matches!(r, Type::Bool) {
                Type::Bool
            } else {
                diags.emit(Diagnostic::error(
                    span,
                    format!(
                        "logical operator requires `bool`, got `{}` and `{}`",
                        fmt_type(l),
                        fmt_type(r)
                    ),
                ));
                Type::Bool
            }
        }
    }
}

fn type_eq(a: &Type, b: &Type) -> bool {
    a == b
}

fn unifies(a: &Type, b: &Type) -> bool {
    if matches!(a, Type::Error) || matches!(b, Type::Error) {
        return true;
    }
    type_eq(a, b)
}

fn fmt_type(t: &Type) -> String {
    let mut s = String::new();
    crate::parse_print_type(&mut s, t);
    s
}
