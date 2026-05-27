//! Typed AST for sli.
//!
//! The AST is the parser's output and the type-checker's input. Nodes carry
//! source spans for diagnostic rendering; type slots are filled in by
//! [`crate::typeck`] and consumed by [`crate::ir`].

use crate::source::Span;

/// A surface type written by the user or inferred by the checker.
#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    /// Inferred / not yet resolved.
    Unknown,
    /// The type checker has already flagged this position; suppress
    /// further errors.
    Error,
    /// `nil`
    Nil,
    /// `bool`
    Bool,
    /// `i32`
    I32,
    /// `i64`
    I64,
    /// `f32`
    F32,
    /// `f64`
    F64,
    /// `str`
    Str,
    /// `Entity`
    Entity,
    /// `Query<T>` — ECS query over a single component (spec V.3).
    Query(Box<Type>),
    /// `Res<T>` — read-only resource borrow.
    Res(Box<Type>),
    /// `ResMut<T>` — mutable resource borrow.
    ResMut(Box<Type>),
    /// `Array<T>`
    Array(Box<Type>),
    /// `Map<K, V>`
    Map(Box<Type>, Box<Type>),
    /// `fn(P1, P2, ...) -> R`
    Fn(Vec<Type>, Box<Type>),
    /// Reference to a user-declared `struct` by name.
    Struct(String),
}

impl Type {
    /// Whether `self` is one of the four built-in numeric types.
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::I32 | Self::I64 | Self::F32 | Self::F64)
    }

    /// Whether `self` is one of the two integer types.
    pub fn is_integer(&self) -> bool {
        matches!(self, Self::I32 | Self::I64)
    }

    /// Whether `self` is one of the two floating-point types.
    pub fn is_float(&self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }
}

/// Binary operator. Lowered straight to bytecode with no operator
/// overloading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Mod,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `&&`
    And,
    /// `||`
    Or,
}

/// Unary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `-`
    Neg,
    /// `!`
    Not,
}

/// A literal whose value is fully decoded at parse time.
#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    /// `nil`
    Nil,
    /// `true` / `false`
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Float — stored by bit pattern so [`Lit`] keeps `PartialEq` total.
    Float(u64),
    /// String literal (already escape-decoded).
    Str(String),
}

/// One expression node.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// Variant.
    pub kind: ExprKind,
    /// Source span of the whole expression.
    pub span: Span,
    /// Type slot, filled by the type checker.
    pub ty: Type,
}

/// Expression variants.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// Literal.
    Lit(Lit),
    /// Name reference.
    Ident(String),
    /// `lhs OP rhs`
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `OP expr`
    Unary(UnOp, Box<Expr>),
    /// `callee(args...)`
    Call(Box<Expr>, Vec<Expr>),
    /// `expr.name`
    Field(Box<Expr>, String),
    /// `expr[idx]`
    Index(Box<Expr>, Box<Expr>),
    /// `TyName { field: expr, ... }`
    StructLit(String, Vec<(String, Expr)>),
    /// `[e1, e2, ...]` — array literal (homogeneous, type inferred from
    /// the first element). Empty `[]` requires an outer type annotation
    /// (e.g. `let xs: Array<i32> = [];`).
    ArrayLit(Vec<Expr>),
    /// `[k1 => v1, k2 => v2, ...]` — map literal (homogeneous keys +
    /// values, types inferred from the first pair). Empty `[:]`
    /// requires an outer type annotation.
    MapLit(Vec<(Expr, Expr)>),
    /// `|p1, p2| body`
    Closure(Vec<Param>, Box<Expr>),
    /// A braced block as an expression. The block's tail value (if any)
    /// is the expression's value.
    Block(Box<Block>),
    /// `if c { ... } else { ... }`
    If(Box<Expr>, Box<Block>, Option<Box<Block>>),
}

/// Function or closure parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// Parameter name.
    pub name: String,
    /// Declared parameter type. May be [`Type::Unknown`] for closure params.
    pub ty: Type,
    /// Span of the parameter name.
    pub span: Span,
}

/// One statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    /// Variant.
    pub kind: StmtKind,
    /// Source span.
    pub span: Span,
}

/// Statement variants.
#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// `let name(: ty)? = init;`
    Let {
        /// Bound name.
        name: String,
        /// Optional declared type.
        ty: Type,
        /// Whether the binding is `let mut`.
        mutable: bool,
        /// Initializer.
        init: Expr,
    },
    /// `place = value;`
    Assign(Expr, Expr),
    /// Expression statement (`expr;`).
    Expr(Expr),
    /// `return expr?;`
    Return(Option<Expr>),
    /// `while cond { body }`
    While(Expr, Box<Block>),
    /// `if`/`else` used in statement position. The expression form is
    /// folded into the same node via [`ExprKind::If`]; this variant
    /// exists so a trailing `if` without a tail value parses cleanly.
    If(Expr, Box<Block>, Option<Box<Block>>),
    /// `break;`
    Break,
    /// `continue;`
    Continue,
}

/// Braced block — a list of statements and an optional tail expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Statements that execute in order.
    pub stmts: Vec<Stmt>,
    /// Optional tail expression — the block's value.
    pub tail: Option<Expr>,
    /// Source span of the whole block.
    pub span: Span,
}

/// A function declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    /// Function name.
    pub name: String,
    /// Parameters in declaration order.
    pub params: Vec<Param>,
    /// Return type.
    pub ret: Type,
    /// Body.
    pub body: Block,
    /// Source span of the whole declaration.
    pub span: Span,
}

/// A struct declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct StructDecl {
    /// Type name.
    pub name: String,
    /// `(field_name, field_type, field_span)` in declaration order.
    pub fields: Vec<(String, Type, Span)>,
    /// Source span of the whole declaration.
    pub span: Span,
}

/// A `const` declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstDecl {
    /// Bound name.
    pub name: String,
    /// Declared type.
    pub ty: Type,
    /// Initializer — must be a const-evaluable expression.
    pub init: Expr,
    /// Source span.
    pub span: Span,
}

/// One top-level declaration in a module.
#[derive(Clone, Debug, PartialEq)]
pub enum Decl {
    /// `fn ...`
    Fn(FnDecl),
    /// `struct ...`
    Struct(StructDecl),
    /// `const ...`
    Const(ConstDecl),
}

/// One parsed sli module.
#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    /// Declarations in source order.
    pub decls: Vec<Decl>,
}
