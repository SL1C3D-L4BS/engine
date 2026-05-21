//! Hand-written lexer for sli source text.
//!
//! Produces a flat token stream. No parser-generator crate is involved
//! (ADR-034 owned-VM grep guard rejects lalrpop, pest, nom, combine,
//! chumsky); the lexer's only dependency is the source map.
//!
//! Lexical structure:
//!
//! - Line comments start with `//`.
//! - Block comments `/* ... */` nest.
//! - String literals are double-quoted; supported escapes are
//!   `\n`, `\r`, `\t`, `\0`, `\\`, `\"`.
//! - Integer literals: decimal digits with optional `_` separators.
//! - Float literals: decimal digits with a `.` and decimal digits, or
//!   trailing `f32` / `f64` suffix.
//! - Identifiers and keywords share ASCII alphanumerics + `_`; keywords
//!   are resolved after lexing the lexeme.

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{FileId, Source, Span};

/// One classified lexeme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    // --- literals -----------------------------------------------------
    /// Integer literal — value already parsed; the span is the lexeme.
    Int(i64),
    /// Float literal — value already parsed.
    Float(u64), // bit pattern; `f64::from_bits`
    /// String literal — payload is byte indices into a per-token table
    /// the parser owns. The lexer only checks well-formedness; the
    /// decoded text lives on the [`Token`] itself.
    Str,
    /// Boolean literal `true`.
    True,
    /// Boolean literal `false`.
    False,
    /// `nil` literal.
    Nil,

    // --- identifiers / keywords --------------------------------------
    /// Identifier or unknown keyword. Use [`Token::lexeme`] to read it.
    Ident,
    /// `fn` keyword.
    KwFn,
    /// `let` keyword.
    KwLet,
    /// `mut` keyword.
    KwMut,
    /// `const` keyword.
    KwConst,
    /// `return` keyword.
    KwReturn,
    /// `if` keyword.
    KwIf,
    /// `else` keyword.
    KwElse,
    /// `while` keyword.
    KwWhile,
    /// `for` keyword.
    KwFor,
    /// `in` keyword.
    KwIn,
    /// `break` keyword.
    KwBreak,
    /// `continue` keyword.
    KwContinue,
    /// `struct` keyword.
    KwStruct,
    /// `as` keyword.
    KwAs,

    // --- punctuation --------------------------------------------------
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `,`
    Comma,
    /// `;`
    Semicolon,
    /// `:`
    Colon,
    /// `::`
    ColonColon,
    /// `.`
    Dot,
    /// `->`
    Arrow,
    /// `=>`
    FatArrow,
    /// `|`  (used as closure parameter delimiter)
    Pipe,

    // --- operators ----------------------------------------------------
    /// `=`
    Assign,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `==`
    EqEq,
    /// `!=`
    BangEq,
    /// `<`
    Lt,
    /// `<=`
    LtEq,
    /// `>`
    Gt,
    /// `>=`
    GtEq,
    /// `&&`
    AmpAmp,
    /// `||`
    PipePipe,
    /// `!`
    Bang,

    // --- meta ---------------------------------------------------------
    /// End of input.
    Eof,
    /// Malformed lexeme — diagnostic already emitted.
    Error,
}

/// One lexed token: kind, source span, and the decoded text of any
/// payload-carrying literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Classified lexeme.
    pub kind: TokenKind,
    /// Source span.
    pub span: Span,
    /// Decoded string-literal text (without quotes). Empty for non-strings.
    pub str_value: String,
    /// Raw identifier text — only populated for [`TokenKind::Ident`] and
    /// keyword tokens to keep token equality lightweight elsewhere.
    pub ident: String,
}

impl Token {
    /// Constructs an `Eof` token at the very end of `file`.
    pub fn eof(file: FileId, at: u32) -> Self {
        Self {
            kind: TokenKind::Eof,
            span: Span::point(file, at),
            str_value: String::new(),
            ident: String::new(),
        }
    }
}

/// Lexes `source` into a `Vec<Token>`, emitting diagnostics into `diags`.
pub fn lex(file: FileId, source: &Source, diags: &mut Diagnostics) -> Vec<Token> {
    let bytes = source.text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        // Skip whitespace.
        if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
            i += 1;
            continue;
        }
        // Line comment.
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment (nesting).
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let mut depth = 1;
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && depth > 0 {
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth != 0 {
                diags.emit(Diagnostic::error(
                    Span::new(file, start as u32, bytes.len() as u32),
                    "unterminated block comment",
                ));
                out.push(Token {
                    kind: TokenKind::Error,
                    span: Span::new(file, start as u32, bytes.len() as u32),
                    str_value: String::new(),
                    ident: String::new(),
                });
                break;
            }
            continue;
        }
        let lo = i as u32;
        // Identifier / keyword.
        if c == b'_' || c.is_ascii_alphabetic() {
            let start = i;
            while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let lex = &source.text[start..i];
            let kind = match lex {
                "fn" => TokenKind::KwFn,
                "let" => TokenKind::KwLet,
                "mut" => TokenKind::KwMut,
                "const" => TokenKind::KwConst,
                "return" => TokenKind::KwReturn,
                "if" => TokenKind::KwIf,
                "else" => TokenKind::KwElse,
                "while" => TokenKind::KwWhile,
                "for" => TokenKind::KwFor,
                "in" => TokenKind::KwIn,
                "break" => TokenKind::KwBreak,
                "continue" => TokenKind::KwContinue,
                "struct" => TokenKind::KwStruct,
                "as" => TokenKind::KwAs,
                "true" => TokenKind::True,
                "false" => TokenKind::False,
                "nil" => TokenKind::Nil,
                _ => TokenKind::Ident,
            };
            out.push(Token {
                kind,
                span: Span::new(file, lo, i as u32),
                str_value: String::new(),
                ident: lex.to_string(),
            });
            continue;
        }
        // Number literal.
        if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                i += 1;
            }
            let mut is_float = false;
            if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                is_float = true;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            // Optional `f32` / `f64` suffix forces float interpretation.
            if i + 2 < bytes.len()
                && bytes[i] == b'f'
                && (&bytes[i + 1..i + 3] == b"32" || &bytes[i + 1..i + 3] == b"64")
            {
                is_float = true;
                i += 3;
            }
            let lex: String = source.text[start..i]
                .chars()
                .filter(|c| *c != '_' && *c != 'f')
                .collect::<String>()
                .replace("32", "")
                .replace("64", "");
            let kind = if is_float {
                match lex.parse::<f64>() {
                    Ok(v) => TokenKind::Float(v.to_bits()),
                    Err(_) => {
                        diags.emit(Diagnostic::error(
                            Span::new(file, lo, i as u32),
                            "malformed float literal",
                        ));
                        TokenKind::Error
                    }
                }
            } else {
                match lex.parse::<i64>() {
                    Ok(v) => TokenKind::Int(v),
                    Err(_) => {
                        diags.emit(Diagnostic::error(
                            Span::new(file, lo, i as u32),
                            "integer literal out of range",
                        ));
                        TokenKind::Error
                    }
                }
            };
            out.push(Token {
                kind,
                span: Span::new(file, lo, i as u32),
                str_value: String::new(),
                ident: String::new(),
            });
            continue;
        }
        // String literal.
        if c == b'"' {
            let start = i;
            i += 1;
            let mut decoded = String::new();
            let mut ok = true;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    let esc = bytes[i + 1];
                    let ch = match esc {
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'0' => '\0',
                        b'\\' => '\\',
                        b'"' => '"',
                        _ => {
                            diags.emit(Diagnostic::error(
                                Span::new(file, i as u32, (i + 2) as u32),
                                format!("unknown escape \\{}", esc as char),
                            ));
                            ok = false;
                            ' '
                        }
                    };
                    decoded.push(ch);
                    i += 2;
                } else {
                    decoded.push(bytes[i] as char);
                    i += 1;
                }
            }
            if i >= bytes.len() {
                diags.emit(Diagnostic::error(
                    Span::new(file, start as u32, bytes.len() as u32),
                    "unterminated string literal",
                ));
                out.push(Token {
                    kind: TokenKind::Error,
                    span: Span::new(file, start as u32, bytes.len() as u32),
                    str_value: String::new(),
                    ident: String::new(),
                });
                break;
            }
            i += 1; // closing quote
            out.push(Token {
                kind: if ok { TokenKind::Str } else { TokenKind::Error },
                span: Span::new(file, lo, i as u32),
                str_value: decoded,
                ident: String::new(),
            });
            continue;
        }
        // Punctuation / operators.
        let next = bytes.get(i + 1).copied();
        // `kind1` for a 1-byte punctuation token, `kind2` for a 2-byte one;
        // both share the push-and-advance bookkeeping.
        let (tok, len): (TokenKind, usize) = match c {
            b'(' => (TokenKind::LParen, 1),
            b')' => (TokenKind::RParen, 1),
            b'{' => (TokenKind::LBrace, 1),
            b'}' => (TokenKind::RBrace, 1),
            b'[' => (TokenKind::LBracket, 1),
            b']' => (TokenKind::RBracket, 1),
            b',' => (TokenKind::Comma, 1),
            b';' => (TokenKind::Semicolon, 1),
            b':' => match next {
                Some(b':') => (TokenKind::ColonColon, 2),
                _ => (TokenKind::Colon, 1),
            },
            b'.' => (TokenKind::Dot, 1),
            b'+' => (TokenKind::Plus, 1),
            b'-' => match next {
                Some(b'>') => (TokenKind::Arrow, 2),
                _ => (TokenKind::Minus, 1),
            },
            b'*' => (TokenKind::Star, 1),
            b'/' => (TokenKind::Slash, 1),
            b'%' => (TokenKind::Percent, 1),
            b'=' => match next {
                Some(b'=') => (TokenKind::EqEq, 2),
                Some(b'>') => (TokenKind::FatArrow, 2),
                _ => (TokenKind::Assign, 1),
            },
            b'!' => match next {
                Some(b'=') => (TokenKind::BangEq, 2),
                _ => (TokenKind::Bang, 1),
            },
            b'<' => match next {
                Some(b'=') => (TokenKind::LtEq, 2),
                _ => (TokenKind::Lt, 1),
            },
            b'>' => match next {
                Some(b'=') => (TokenKind::GtEq, 2),
                _ => (TokenKind::Gt, 1),
            },
            b'&' => match next {
                Some(b'&') => (TokenKind::AmpAmp, 2),
                _ => {
                    diags.emit(Diagnostic::error(
                        Span::new(file, lo, lo + 1),
                        "unexpected character `&`",
                    ));
                    i += 1;
                    continue;
                }
            },
            b'|' => match next {
                Some(b'|') => (TokenKind::PipePipe, 2),
                _ => (TokenKind::Pipe, 1),
            },
            _ => {
                diags.emit(Diagnostic::error(
                    Span::new(file, lo, lo + 1),
                    format!("unexpected character `{}`", c as char),
                ));
                i += 1;
                continue;
            }
        };
        out.push(Token {
            kind: tok,
            span: Span::new(file, lo, (lo as usize + len) as u32),
            str_value: String::new(),
            ident: String::new(),
        });
        i += len;
    }
    out.push(Token::eof(file, bytes.len() as u32));
    out
}
