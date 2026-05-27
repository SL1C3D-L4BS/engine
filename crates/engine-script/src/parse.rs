//! Pratt parser for sli.
//!
//! Top-down operator precedence: every prefix construct has a `nud` and
//! every infix operator has a `led`. The parser is recursive-descent
//! against the [`crate::lex::Token`] stream produced by the lexer.

use crate::ast::{
    BinOp, Block, ConstDecl, Decl, Expr, ExprKind, FnDecl, Lit, Module, Param, Stmt, StmtKind,
    StructDecl, Type, UnOp,
};
use crate::diag::{Diagnostic, Diagnostics};
use crate::lex::{Token, TokenKind};

/// Parses `tokens` into a [`Module`], emitting diagnostics into `diags`.
pub fn parse(tokens: &[Token], diags: &mut Diagnostics) -> Module {
    let mut p = Parser {
        toks: tokens,
        pos: 0,
        diags,
    };
    let mut decls = Vec::new();
    while !p.at_end() {
        match p.parse_decl() {
            Some(d) => decls.push(d),
            None => {
                // Skip until next top-level keyword to avoid runaway errors.
                while !p.at_end()
                    && !matches!(
                        p.peek().kind,
                        TokenKind::KwFn | TokenKind::KwStruct | TokenKind::KwConst
                    )
                {
                    p.bump();
                }
            }
        }
    }
    Module { decls }
}

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
    diags: &'a mut Diagnostics,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    fn peek_kind(&self) -> TokenKind {
        self.toks[self.pos].kind
    }

    fn peek_kind_at(&self, offset: usize) -> TokenKind {
        self.toks
            .get(self.pos + offset)
            .map(|t| t.kind)
            .unwrap_or(TokenKind::Eof)
    }

    fn bump(&mut self) -> &Token {
        let t = &self.toks[self.pos];
        if !matches!(t.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        t
    }

    fn at_end(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Eof)
    }

    fn expect(&mut self, k: TokenKind, what: &str) -> Option<Token> {
        if self.peek_kind() == k {
            Some(self.bump().clone())
        } else {
            let tok = self.peek().clone();
            self.diags.emit(Diagnostic::error(
                tok.span,
                format!("expected {what}, found `{}`", token_label(&tok)),
            ));
            None
        }
    }

    fn eat(&mut self, k: TokenKind) -> bool {
        if self.peek_kind() == k {
            self.bump();
            true
        } else {
            false
        }
    }

    fn parse_decl(&mut self) -> Option<Decl> {
        match self.peek_kind() {
            TokenKind::KwFn => self.parse_fn().map(Decl::Fn),
            TokenKind::KwStruct => self.parse_struct().map(Decl::Struct),
            TokenKind::KwConst => self.parse_const().map(Decl::Const),
            _ => {
                let tok = self.peek().clone();
                self.diags.emit(Diagnostic::error(
                    tok.span,
                    format!(
                        "expected `fn`, `struct`, or `const`, found `{}`",
                        token_label(&tok)
                    ),
                ));
                None
            }
        }
    }

    fn parse_fn(&mut self) -> Option<FnDecl> {
        let fn_tok = self.bump().clone(); // `fn`
        let name_tok = self.expect(TokenKind::Ident, "function name")?;
        self.expect(TokenKind::LParen, "`(`")?;
        let mut params = Vec::new();
        if !matches!(self.peek_kind(), TokenKind::RParen) {
            loop {
                let p = self.parse_param()?;
                params.push(p);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen, "`)`")?;
        let ret = if self.eat(TokenKind::Arrow) {
            self.parse_type()?
        } else {
            Type::Nil
        };
        let body = self.parse_block()?;
        let span = fn_tok.span.join(body.span);
        Some(FnDecl {
            name: name_tok.ident,
            params,
            ret,
            body,
            span,
        })
    }

    fn parse_param(&mut self) -> Option<Param> {
        let name_tok = self.expect(TokenKind::Ident, "parameter name")?;
        let ty = if self.eat(TokenKind::Colon) {
            self.parse_type()?
        } else {
            Type::Unknown
        };
        Some(Param {
            name: name_tok.ident,
            ty,
            span: name_tok.span,
        })
    }

    fn parse_struct(&mut self) -> Option<StructDecl> {
        let kw = self.bump().clone();
        let name_tok = self.expect(TokenKind::Ident, "struct name")?;
        self.expect(TokenKind::LBrace, "`{`")?;
        let mut fields = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
            let n = self.expect(TokenKind::Ident, "field name")?;
            self.expect(TokenKind::Colon, "`:`")?;
            let ty = self.parse_type()?;
            fields.push((n.ident.clone(), ty, n.span));
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RBrace, "`}`")?;
        Some(StructDecl {
            name: name_tok.ident,
            fields,
            span: kw.span.join(close.span),
        })
    }

    fn parse_const(&mut self) -> Option<ConstDecl> {
        let kw = self.bump().clone();
        let name_tok = self.expect(TokenKind::Ident, "constant name")?;
        self.expect(TokenKind::Colon, "`:`")?;
        let ty = self.parse_type()?;
        self.expect(TokenKind::Assign, "`=`")?;
        let init = self.parse_expr(0)?;
        let semi = self.expect(TokenKind::Semicolon, "`;`")?;
        Some(ConstDecl {
            name: name_tok.ident,
            ty,
            init,
            span: kw.span.join(semi.span),
        })
    }

    fn parse_type(&mut self) -> Option<Type> {
        let tok = self.bump().clone();
        let ty = match tok.kind {
            TokenKind::KwFn => {
                // `fn(P1, P2, ...) -> R`
                self.expect(TokenKind::LParen, "`(`")?;
                let mut params = Vec::new();
                if !matches!(self.peek_kind(), TokenKind::RParen) {
                    loop {
                        params.push(self.parse_type()?);
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RParen, "`)`")?;
                let ret = if self.eat(TokenKind::Arrow) {
                    self.parse_type()?
                } else {
                    Type::Nil
                };
                return Some(Type::Fn(params, Box::new(ret)));
            }
            TokenKind::Ident => match tok.ident.as_str() {
                "i32" => Type::I32,
                "i64" => Type::I64,
                "f32" => Type::F32,
                "f64" => Type::F64,
                "bool" => Type::Bool,
                "str" => Type::Str,
                "nil" => Type::Nil,
                "Entity" => Type::Entity,
                "Query" => {
                    self.expect(TokenKind::Lt, "`<`")?;
                    let inner = self.parse_type()?;
                    self.expect(TokenKind::Gt, "`>`")?;
                    Type::Query(Box::new(inner))
                }
                "Res" => {
                    self.expect(TokenKind::Lt, "`<`")?;
                    let inner = self.parse_type()?;
                    self.expect(TokenKind::Gt, "`>`")?;
                    Type::Res(Box::new(inner))
                }
                "ResMut" => {
                    self.expect(TokenKind::Lt, "`<`")?;
                    let inner = self.parse_type()?;
                    self.expect(TokenKind::Gt, "`>`")?;
                    Type::ResMut(Box::new(inner))
                }
                "Array" => {
                    self.expect(TokenKind::Lt, "`<`")?;
                    let inner = self.parse_type()?;
                    self.expect(TokenKind::Gt, "`>`")?;
                    Type::Array(Box::new(inner))
                }
                "Map" => {
                    self.expect(TokenKind::Lt, "`<`")?;
                    let k = self.parse_type()?;
                    self.expect(TokenKind::Comma, "`,`")?;
                    let v = self.parse_type()?;
                    self.expect(TokenKind::Gt, "`>`")?;
                    Type::Map(Box::new(k), Box::new(v))
                }
                _ => Type::Struct(tok.ident.clone()),
            },
            TokenKind::Nil => Type::Nil,
            _ => {
                self.diags.emit(Diagnostic::error(
                    tok.span,
                    format!("expected type, found `{}`", token_label(&tok)),
                ));
                Type::Error
            }
        };
        Some(ty)
    }

    fn parse_block(&mut self) -> Option<Block> {
        let open = self.expect(TokenKind::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        let mut tail: Option<Expr> = None;
        while !matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
            // Statements that start with a keyword don't compete with
            // tail-expression position.
            match self.peek_kind() {
                TokenKind::KwLet => stmts.push(self.parse_let()?),
                TokenKind::KwReturn => stmts.push(self.parse_return()?),
                TokenKind::KwWhile => stmts.push(self.parse_while()?),
                TokenKind::KwBreak => {
                    let t = self.bump().clone();
                    let semi = self.expect(TokenKind::Semicolon, "`;`")?;
                    stmts.push(Stmt {
                        kind: StmtKind::Break,
                        span: t.span.join(semi.span),
                    });
                }
                TokenKind::KwContinue => {
                    let t = self.bump().clone();
                    let semi = self.expect(TokenKind::Semicolon, "`;`")?;
                    stmts.push(Stmt {
                        kind: StmtKind::Continue,
                        span: t.span.join(semi.span),
                    });
                }
                _ => {
                    let expr = self.parse_expr(0)?;
                    // Assignment?
                    if matches!(self.peek_kind(), TokenKind::Assign) {
                        self.bump();
                        let value = self.parse_expr(0)?;
                        let semi = self.expect(TokenKind::Semicolon, "`;`")?;
                        let span = expr.span.join(semi.span);
                        stmts.push(Stmt {
                            kind: StmtKind::Assign(expr, value),
                            span,
                        });
                    } else if matches!(self.peek_kind(), TokenKind::Semicolon) {
                        let semi = self.bump().clone();
                        let span = expr.span.join(semi.span);
                        stmts.push(Stmt {
                            kind: StmtKind::Expr(expr),
                            span,
                        });
                    } else if matches!(self.peek_kind(), TokenKind::RBrace) {
                        tail = Some(expr);
                        break;
                    } else if is_block_expr(&expr) {
                        // Block-style expressions (`if`, block, while-as-expr)
                        // are valid statements without a trailing `;`.
                        let span = expr.span;
                        stmts.push(Stmt {
                            kind: StmtKind::Expr(expr),
                            span,
                        });
                    } else {
                        let tok = self.peek().clone();
                        self.diags.emit(Diagnostic::error(
                            tok.span,
                            format!("expected `;` or `}}`, found `{}`", token_label(&tok)),
                        ));
                        return None;
                    }
                }
            }
        }
        let close = self.expect(TokenKind::RBrace, "`}`")?;
        Some(Block {
            stmts,
            tail,
            span: open.span.join(close.span),
        })
    }

    fn parse_let(&mut self) -> Option<Stmt> {
        let kw = self.bump().clone();
        let mutable = self.eat(TokenKind::KwMut);
        let name_tok = self.expect(TokenKind::Ident, "binding name")?;
        let ty = if self.eat(TokenKind::Colon) {
            self.parse_type()?
        } else {
            Type::Unknown
        };
        self.expect(TokenKind::Assign, "`=`")?;
        let init = self.parse_expr(0)?;
        let semi = self.expect(TokenKind::Semicolon, "`;`")?;
        Some(Stmt {
            kind: StmtKind::Let {
                name: name_tok.ident,
                ty,
                mutable,
                init,
            },
            span: kw.span.join(semi.span),
        })
    }

    fn parse_return(&mut self) -> Option<Stmt> {
        let kw = self.bump().clone();
        let value = if matches!(self.peek_kind(), TokenKind::Semicolon) {
            None
        } else {
            Some(self.parse_expr(0)?)
        };
        let semi = self.expect(TokenKind::Semicolon, "`;`")?;
        Some(Stmt {
            kind: StmtKind::Return(value),
            span: kw.span.join(semi.span),
        })
    }

    fn parse_while(&mut self) -> Option<Stmt> {
        let kw = self.bump().clone();
        let cond = self.parse_expr(0)?;
        let body = self.parse_block()?;
        let span = kw.span.join(body.span);
        Some(Stmt {
            kind: StmtKind::While(cond, Box::new(body)),
            span,
        })
    }

    fn parse_expr(&mut self, min_bp: u8) -> Option<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op_tok = self.peek().clone();
            let (op, l_bp, r_bp) = match infix_bp(op_tok.kind) {
                Some(t) => t,
                None => break,
            };
            if l_bp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_expr(r_bp)?;
            let span = lhs.span.join(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
                span,
                ty: Type::Unknown,
            };
        }
        Some(lhs)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        match self.peek_kind() {
            TokenKind::Minus => {
                let t = self.bump().clone();
                let e = self.parse_unary()?;
                let span = t.span.join(e.span);
                Some(Expr {
                    kind: ExprKind::Unary(UnOp::Neg, Box::new(e)),
                    span,
                    ty: Type::Unknown,
                })
            }
            TokenKind::Bang => {
                let t = self.bump().clone();
                let e = self.parse_unary()?;
                let span = t.span.join(e.span);
                Some(Expr {
                    kind: ExprKind::Unary(UnOp::Not, Box::new(e)),
                    span,
                    ty: Type::Unknown,
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Option<Expr> {
        let mut expr = self.parse_atom()?;
        loop {
            match self.peek_kind() {
                TokenKind::LParen => {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek_kind(), TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if !self.eat(TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    let close = self.expect(TokenKind::RParen, "`)`")?;
                    let span = expr.span.join(close.span);
                    expr = Expr {
                        kind: ExprKind::Call(Box::new(expr), args),
                        span,
                        ty: Type::Unknown,
                    };
                }
                TokenKind::Dot => {
                    self.bump();
                    let name = self.expect(TokenKind::Ident, "field name")?;
                    let span = expr.span.join(name.span);
                    expr = Expr {
                        kind: ExprKind::Field(Box::new(expr), name.ident),
                        span,
                        ty: Type::Unknown,
                    };
                }
                TokenKind::LBracket => {
                    self.bump();
                    let idx = self.parse_expr(0)?;
                    let close = self.expect(TokenKind::RBracket, "`]`")?;
                    let span = expr.span.join(close.span);
                    expr = Expr {
                        kind: ExprKind::Index(Box::new(expr), Box::new(idx)),
                        span,
                        ty: Type::Unknown,
                    };
                }
                _ => break,
            }
        }
        Some(expr)
    }

    fn parse_atom(&mut self) -> Option<Expr> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Int(v) => {
                self.bump();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Int(v)),
                    span: tok.span,
                    ty: Type::Unknown,
                })
            }
            TokenKind::Float(bits) => {
                self.bump();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Float(bits)),
                    span: tok.span,
                    ty: Type::Unknown,
                })
            }
            TokenKind::True => {
                self.bump();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Bool(true)),
                    span: tok.span,
                    ty: Type::Unknown,
                })
            }
            TokenKind::False => {
                self.bump();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Bool(false)),
                    span: tok.span,
                    ty: Type::Unknown,
                })
            }
            TokenKind::Nil => {
                self.bump();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Nil),
                    span: tok.span,
                    ty: Type::Unknown,
                })
            }
            TokenKind::Str => {
                self.bump();
                Some(Expr {
                    kind: ExprKind::Lit(Lit::Str(tok.str_value)),
                    span: tok.span,
                    ty: Type::Unknown,
                })
            }
            TokenKind::LParen => {
                self.bump();
                let e = self.parse_expr(0)?;
                self.expect(TokenKind::RParen, "`)`")?;
                Some(e)
            }
            TokenKind::LBracket => {
                // Primary `[` — array or map literal (ADR-060).
                // The postfix `[` for indexing is consumed inside
                // `parse_postfix`, which is layered on top of
                // `parse_primary`, so this arm never shadows it.
                self.bump();
                // Empty `[]` is an empty array literal; `[:]` is the
                // empty map literal. Both flow through here.
                if matches!(self.peek_kind(), TokenKind::RBracket) {
                    let close = self.bump().clone();
                    let span = tok.span.join(close.span);
                    return Some(Expr {
                        kind: ExprKind::ArrayLit(Vec::new()),
                        span,
                        ty: Type::Unknown,
                    });
                }
                if matches!(self.peek_kind(), TokenKind::Colon)
                    && matches!(self.peek_kind_at(1), TokenKind::RBracket)
                {
                    self.bump(); // `:`
                    let close = self.bump().clone(); // `]`
                    let span = tok.span.join(close.span);
                    return Some(Expr {
                        kind: ExprKind::MapLit(Vec::new()),
                        span,
                        ty: Type::Unknown,
                    });
                }
                let first = self.parse_expr(0)?;
                // Disambiguator: a `=>` after the first element means
                // map literal; otherwise the first element is an array
                // element.
                if matches!(self.peek_kind(), TokenKind::FatArrow) {
                    self.bump(); // `=>`
                    let v = self.parse_expr(0)?;
                    let mut pairs: Vec<(Expr, Expr)> = vec![(first, v)];
                    while self.eat(TokenKind::Comma) {
                        if matches!(self.peek_kind(), TokenKind::RBracket) {
                            break; // trailing comma
                        }
                        let k = self.parse_expr(0)?;
                        self.expect(TokenKind::FatArrow, "`=>`")?;
                        let v = self.parse_expr(0)?;
                        pairs.push((k, v));
                    }
                    let close = self.expect(TokenKind::RBracket, "`]`")?;
                    let span = tok.span.join(close.span);
                    Some(Expr {
                        kind: ExprKind::MapLit(pairs),
                        span,
                        ty: Type::Unknown,
                    })
                } else {
                    let mut elems: Vec<Expr> = vec![first];
                    while self.eat(TokenKind::Comma) {
                        if matches!(self.peek_kind(), TokenKind::RBracket) {
                            break; // trailing comma
                        }
                        elems.push(self.parse_expr(0)?);
                    }
                    let close = self.expect(TokenKind::RBracket, "`]`")?;
                    let span = tok.span.join(close.span);
                    Some(Expr {
                        kind: ExprKind::ArrayLit(elems),
                        span,
                        ty: Type::Unknown,
                    })
                }
            }
            TokenKind::LBrace => {
                let b = self.parse_block()?;
                let span = b.span;
                Some(Expr {
                    kind: ExprKind::Block(Box::new(b)),
                    span,
                    ty: Type::Unknown,
                })
            }
            TokenKind::KwIf => self.parse_if_expr(),
            TokenKind::Pipe => self.parse_closure(),
            TokenKind::Ident => {
                self.bump();
                // Struct literal?
                if matches!(self.peek_kind(), TokenKind::LBrace) && looks_like_type(&tok.ident) {
                    self.bump();
                    let mut fields = Vec::new();
                    while !matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
                        let n = self.expect(TokenKind::Ident, "field name")?;
                        self.expect(TokenKind::Colon, "`:`")?;
                        let v = self.parse_expr(0)?;
                        fields.push((n.ident, v));
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                    let close = self.expect(TokenKind::RBrace, "`}`")?;
                    let span = tok.span.join(close.span);
                    Some(Expr {
                        kind: ExprKind::StructLit(tok.ident, fields),
                        span,
                        ty: Type::Unknown,
                    })
                } else {
                    Some(Expr {
                        kind: ExprKind::Ident(tok.ident),
                        span: tok.span,
                        ty: Type::Unknown,
                    })
                }
            }
            _ => {
                self.diags.emit(Diagnostic::error(
                    tok.span,
                    format!("expected expression, found `{}`", token_label(&tok)),
                ));
                None
            }
        }
    }

    fn parse_if_expr(&mut self) -> Option<Expr> {
        let kw = self.bump().clone();
        let cond = self.parse_expr(0)?;
        let then = self.parse_block()?;
        let mut span = kw.span.join(then.span);
        let else_ = if self.eat(TokenKind::KwElse) {
            // `else if` or `else { ... }`
            if matches!(self.peek_kind(), TokenKind::KwIf) {
                let inner = self.parse_if_expr()?;
                let inner_span = inner.span;
                span = span.join(inner_span);
                Some(Box::new(Block {
                    stmts: Vec::new(),
                    tail: Some(inner),
                    span: inner_span,
                }))
            } else {
                let b = self.parse_block()?;
                span = span.join(b.span);
                Some(Box::new(b))
            }
        } else {
            None
        };
        Some(Expr {
            kind: ExprKind::If(Box::new(cond), Box::new(then), else_),
            span,
            ty: Type::Unknown,
        })
    }

    fn parse_closure(&mut self) -> Option<Expr> {
        let open = self.bump().clone();
        let mut params = Vec::new();
        if !matches!(self.peek_kind(), TokenKind::Pipe) {
            loop {
                let p = self.parse_param()?;
                params.push(p);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::Pipe, "`|`")?;
        let body = self.parse_expr(0)?;
        let span = open.span.join(body.span);
        Some(Expr {
            kind: ExprKind::Closure(params, Box::new(body)),
            span,
            ty: Type::Unknown,
        })
    }
}

fn is_block_expr(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::If(..) | ExprKind::Block(..))
}

fn infix_bp(k: TokenKind) -> Option<(BinOp, u8, u8)> {
    use TokenKind::*;
    Some(match k {
        PipePipe => (BinOp::Or, 1, 2),
        AmpAmp => (BinOp::And, 3, 4),
        EqEq => (BinOp::Eq, 5, 6),
        BangEq => (BinOp::Ne, 5, 6),
        Lt => (BinOp::Lt, 7, 8),
        LtEq => (BinOp::Le, 7, 8),
        Gt => (BinOp::Gt, 7, 8),
        GtEq => (BinOp::Ge, 7, 8),
        Plus => (BinOp::Add, 9, 10),
        Minus => (BinOp::Sub, 9, 10),
        Star => (BinOp::Mul, 11, 12),
        Slash => (BinOp::Div, 11, 12),
        Percent => (BinOp::Mod, 11, 12),
        _ => return None,
    })
}

fn looks_like_type(name: &str) -> bool {
    // Struct literals are written `Foo { ... }`; types start with an
    // upper-case ASCII letter. This avoids ambiguity with the block
    // expression `expr { ... }` form (which doesn't exist in sli — but
    // the rule keeps the parser locally unambiguous).
    name.chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
}

fn token_label(t: &Token) -> String {
    match t.kind {
        TokenKind::Eof => "<eof>".into(),
        TokenKind::Ident => t.ident.clone(),
        TokenKind::Str => format!("\"{}\"", t.str_value),
        TokenKind::Int(v) => v.to_string(),
        TokenKind::Float(b) => format!("{}", f64::from_bits(b)),
        _ => format!("{:?}", t.kind),
    }
}

/// Reprints `module` as canonical sli source. Used by `tests/parser.rs` for
/// the round-trip oracle (`parse -> print -> parse` must reach a fixed
/// point). The exact formatting is not load-bearing; what matters is that
/// the reprinted text re-parses to a structurally-equal AST.
pub fn print(module: &Module) -> String {
    let mut out = String::new();
    for d in &module.decls {
        match d {
            Decl::Fn(f) => print_fn(&mut out, f),
            Decl::Struct(s) => print_struct(&mut out, s),
            Decl::Const(c) => print_const(&mut out, c),
        }
        out.push('\n');
    }
    out
}

fn print_fn(out: &mut String, f: &FnDecl) {
    out.push_str("fn ");
    out.push_str(&f.name);
    out.push('(');
    for (i, p) in f.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&p.name);
        if !matches!(p.ty, Type::Unknown) {
            out.push_str(": ");
            print_type(out, &p.ty);
        }
    }
    out.push(')');
    if !matches!(f.ret, Type::Nil) {
        out.push_str(" -> ");
        print_type(out, &f.ret);
    }
    out.push(' ');
    print_block(out, &f.body);
}

fn print_struct(out: &mut String, s: &StructDecl) {
    out.push_str("struct ");
    out.push_str(&s.name);
    out.push_str(" {\n");
    for (n, t, _) in &s.fields {
        out.push_str("    ");
        out.push_str(n);
        out.push_str(": ");
        print_type(out, t);
        out.push_str(",\n");
    }
    out.push('}');
}

fn print_const(out: &mut String, c: &ConstDecl) {
    out.push_str("const ");
    out.push_str(&c.name);
    out.push_str(": ");
    print_type(out, &c.ty);
    out.push_str(" = ");
    print_expr(out, &c.init);
    out.push(';');
}

fn print_block(out: &mut String, b: &Block) {
    out.push_str("{\n");
    for s in &b.stmts {
        out.push_str("    ");
        print_stmt(out, s);
        out.push('\n');
    }
    if let Some(t) = &b.tail {
        out.push_str("    ");
        print_expr(out, t);
        out.push('\n');
    }
    out.push('}');
}

fn print_stmt(out: &mut String, s: &Stmt) {
    match &s.kind {
        StmtKind::Let {
            name,
            ty,
            mutable,
            init,
        } => {
            out.push_str("let ");
            if *mutable {
                out.push_str("mut ");
            }
            out.push_str(name);
            if !matches!(ty, Type::Unknown) {
                out.push_str(": ");
                print_type(out, ty);
            }
            out.push_str(" = ");
            print_expr(out, init);
            out.push(';');
        }
        StmtKind::Assign(p, v) => {
            print_expr(out, p);
            out.push_str(" = ");
            print_expr(out, v);
            out.push(';');
        }
        StmtKind::Expr(e) => {
            print_expr(out, e);
            out.push(';');
        }
        StmtKind::Return(v) => {
            out.push_str("return");
            if let Some(v) = v {
                out.push(' ');
                print_expr(out, v);
            }
            out.push(';');
        }
        StmtKind::While(c, b) => {
            out.push_str("while ");
            print_expr(out, c);
            out.push(' ');
            print_block(out, b);
        }
        StmtKind::If(c, t, e) => {
            out.push_str("if ");
            print_expr(out, c);
            out.push(' ');
            print_block(out, t);
            if let Some(e) = e {
                out.push_str(" else ");
                print_block(out, e);
            }
        }
        StmtKind::Break => out.push_str("break;"),
        StmtKind::Continue => out.push_str("continue;"),
    }
}

fn print_expr(out: &mut String, e: &Expr) {
    match &e.kind {
        ExprKind::Lit(l) => print_lit(out, l),
        ExprKind::Ident(n) => out.push_str(n),
        ExprKind::Binary(op, l, r) => {
            out.push('(');
            print_expr(out, l);
            out.push(' ');
            out.push_str(binop_str(*op));
            out.push(' ');
            print_expr(out, r);
            out.push(')');
        }
        ExprKind::Unary(op, e) => {
            out.push(match op {
                UnOp::Neg => '-',
                UnOp::Not => '!',
            });
            print_expr(out, e);
        }
        ExprKind::Call(c, args) => {
            print_expr(out, c);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                print_expr(out, a);
            }
            out.push(')');
        }
        ExprKind::Field(e, n) => {
            print_expr(out, e);
            out.push('.');
            out.push_str(n);
        }
        ExprKind::Index(e, i) => {
            print_expr(out, e);
            out.push('[');
            print_expr(out, i);
            out.push(']');
        }
        ExprKind::StructLit(n, fs) => {
            out.push_str(n);
            out.push_str(" { ");
            for (i, (fname, fv)) in fs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(fname);
                out.push_str(": ");
                print_expr(out, fv);
            }
            out.push_str(" }");
        }
        ExprKind::ArrayLit(elems) => {
            out.push('[');
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                print_expr(out, e);
            }
            out.push(']');
        }
        ExprKind::MapLit(pairs) => {
            out.push('[');
            if pairs.is_empty() {
                out.push(':');
            }
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                print_expr(out, k);
                out.push_str(" => ");
                print_expr(out, v);
            }
            out.push(']');
        }
        ExprKind::Closure(ps, body) => {
            out.push('|');
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&p.name);
                if !matches!(p.ty, Type::Unknown) {
                    out.push_str(": ");
                    print_type(out, &p.ty);
                }
            }
            out.push_str("| ");
            print_expr(out, body);
        }
        ExprKind::Block(b) => print_block(out, b),
        ExprKind::If(c, t, e) => {
            out.push_str("if ");
            print_expr(out, c);
            out.push(' ');
            print_block(out, t);
            if let Some(e) = e {
                out.push_str(" else ");
                print_block(out, e);
            }
        }
    }
}

fn print_lit(out: &mut String, l: &Lit) {
    match l {
        Lit::Nil => out.push_str("nil"),
        Lit::Bool(true) => out.push_str("true"),
        Lit::Bool(false) => out.push_str("false"),
        Lit::Int(v) => out.push_str(&v.to_string()),
        Lit::Float(b) => out.push_str(&format!("{}", f64::from_bits(*b))),
        Lit::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(c),
                }
            }
            out.push('"');
        }
    }
}

fn print_type(out: &mut String, t: &Type) {
    match t {
        Type::Unknown => out.push('_'),
        Type::Error => out.push_str("<error>"),
        Type::Nil => out.push_str("nil"),
        Type::Bool => out.push_str("bool"),
        Type::I32 => out.push_str("i32"),
        Type::I64 => out.push_str("i64"),
        Type::F32 => out.push_str("f32"),
        Type::F64 => out.push_str("f64"),
        Type::Str => out.push_str("str"),
        Type::Entity => out.push_str("Entity"),
        Type::Query(t) => {
            out.push_str("Query<");
            print_type(out, t);
            out.push('>');
        }
        Type::Res(t) => {
            out.push_str("Res<");
            print_type(out, t);
            out.push('>');
        }
        Type::ResMut(t) => {
            out.push_str("ResMut<");
            print_type(out, t);
            out.push('>');
        }
        Type::Array(t) => {
            out.push_str("Array<");
            print_type(out, t);
            out.push('>');
        }
        Type::Map(k, v) => {
            out.push_str("Map<");
            print_type(out, k);
            out.push_str(", ");
            print_type(out, v);
            out.push('>');
        }
        Type::Fn(ps, r) => {
            out.push_str("fn(");
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                print_type(out, p);
            }
            out.push_str(") -> ");
            print_type(out, r);
        }
        Type::Struct(n) => out.push_str(n),
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}
