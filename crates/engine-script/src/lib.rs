//! `engine-script` — sli compiler, register VM, and Slang toolchain bridge.
//!
//! Level 2 crate. See `ENGINE_SPECIFICATION_v2.0.md` Part IV.1.
//!
//! # The sli language
//!
//! sli (pronounced "slice") is the engine's owned scripting language —
//! Sliced Engine's surface for game code. The canonical file extension is
//! `.sli`; `.bp` is kept as a legacy alias for one major version (spec V.1).
//!
//! ## Phase 4 PR 1 — front-end
//!
//! This PR ships the compiler front-end as ten modules:
//!
//! - [`source`] — files, line tables, and byte-offset [`Span`](source::Span)s.
//! - [`diag`] — accumulating [`Diagnostic`](diag::Diagnostic) sink.
//! - [`lex`] — hand-written tokenizer.
//! - [`parse`] — Pratt parser to a typed [`ast::Module`].
//! - [`resolve`] — name resolution to a global symbol table.
//! - [`typeck`] — bottom-up type checker; mutates AST type slots in place.
//! - [`ir`] — SSA IR plus DCE / constant fold / CSE passes.
//! - [`consteval`] — `const` initializer evaluator.
//! - [`ext`] — file-extension routing (`.sli` canonical, `.bp` alias).
//!
//! The PR-2 register VM, PR-3 hot-reload / debugger / REPL, and PR-4
//! Slang toolchain land in their own modules on top of this surface.
//!
//! # Owned dependencies (ADR-034)
//!
//! No parser generator, regex engine, or vendored interpreter is linked.
//! The CI grep guard rejects lalrpop / pest / nom / combine / chumsky /
//! rlua / mlua / wasmtime / wasmer / cranelift / inkwell anywhere under
//! `crates/engine-script/`.

pub mod asset;
pub mod ast;
pub mod breakpoints_toml;
pub mod bytecode;
pub mod codegen;
pub mod consteval;
pub mod debug;
pub mod debug_proto;
pub mod diag;
pub mod ext;
pub mod ffi;
pub mod gc;
pub mod ir;
pub mod lex;
pub mod parse;
pub mod reload;
pub mod repl;
pub mod resolve;
pub mod source;
pub mod typeck;
pub mod verify;
pub mod vm;
pub mod watch_expr;

pub use asset::ScriptModule;
pub use ast::{Decl, Module, Type};
pub use bytecode::{Module as Bytecode, Opcode};
pub use debug::{Breakpoint, BreakpointId, Debugger};
pub use diag::{Diagnostic, Diagnostics, Severity};
pub use ext::{SourceKind, classify};
pub use ffi::{Binding, CallTable, FfiFn};
pub use gc::{GcConfig, GcHandle, GcStats, Heap};
pub use ir::IrModule;
pub use reload::{Event as ReloadEvent, Reloader};
pub use repl::Repl;
pub use source::{FileId, Source, SourceMap, Span};
pub use verify::{VerifyError, verify};
pub use vm::{StopReason, Value, Vm};
pub use watch_expr::{WatchError, validate as validate_watch};

/// Side-table emitted by [`Compiler::compile`] alongside the compiled artefact.
///
/// Carries the byte-offset spans the debugger (Phase 4 PR 3) attaches
/// breakpoints to. Phase 4 PR 1 ships a minimal version: one entry per
/// function declaration. The PR-2 bytecode emitter widens this to one
/// entry per opcode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebugInfo {
    /// `(function name, declaration span)` pairs.
    pub functions: Vec<(String, Span)>,
}

/// Hard compilation failure — distinct from accumulated [`Diagnostics`].
/// A `CompileError` means the compiler couldn't produce *any* IR; soft
/// errors (single bad expression, missing field) ride on the
/// [`Diagnostics`] sink returned by [`Compiler::compile`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// The source file ended before the parser saw a complete program.
    UnexpectedEof,
    /// One or more of the steps reported errors. The caller should
    /// surface the [`Diagnostics`] sink for details.
    HardErrors,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of source"),
            Self::HardErrors => write!(f, "compilation failed; see diagnostics"),
        }
    }
}

impl std::error::Error for CompileError {}

/// Result of [`Compiler::compile`].
#[derive(Debug)]
pub struct Compiled {
    /// Optimised IR module — the cross-arch golden in
    /// `tests/compile_parity.rs` digests this.
    pub ir: IrModule,
    /// Executable bytecode module — input to [`crate::vm::Vm`] (ADR-035).
    pub bytecode: bytecode::Module,
    /// Symbol-keyed type information from the checker.
    pub types: typeck::TypeTable,
    /// Evaluated `const` bindings (spec IV.7 const-eval).
    pub consts: consteval::ConstEnv,
    /// Side-table for debugger attachment.
    pub debug: DebugInfo,
    /// Accumulated diagnostics — empty on a clean build.
    pub diagnostics: Diagnostics,
}

/// The Phase 4 compiler driver.
///
/// One-shot per source; instantiate, [`compile`](Self::compile), drop.
/// Hot-reload (Phase 4 PR 3) constructs a fresh `Compiler` per swap so
/// no per-call mutable state survives across runs.
#[derive(Debug, Default)]
pub struct Compiler;

impl Compiler {
    /// Constructs a fresh compiler.
    pub fn new() -> Self {
        Self
    }

    /// Compiles `source` from `file` end-to-end: lex → parse → resolve
    /// → typeck → consteval → IR lower → optimise. Soft errors land in
    /// `Compiled::diagnostics`; if the source is so malformed nothing
    /// compiles, returns [`CompileError`].
    pub fn compile(
        &self,
        file: FileId,
        source: &Source,
    ) -> Result<Compiled, (CompileError, Diagnostics)> {
        let mut diags = Diagnostics::new();
        let tokens = lex::lex(file, source, &mut diags);
        let mut module = parse::parse(&tokens, &mut diags);
        let _resolution = resolve::resolve(&module, &mut diags);
        let types = typeck::check(&mut module, &mut diags);
        let consts = consteval::eval_consts(&module, &mut diags);
        let debug = DebugInfo {
            functions: module
                .decls
                .iter()
                .filter_map(|d| match d {
                    Decl::Fn(f) => Some((f.name.clone(), f.span)),
                    _ => None,
                })
                .collect(),
        };
        let mut ir = ir::lower(&module);
        ir::optimise(&mut ir);
        let bytecode = codegen::lower(&module, source);
        Ok(Compiled {
            ir,
            bytecode,
            types,
            consts,
            debug,
            diagnostics: diags,
        })
    }
}

/// Pretty-prints a [`Type`] into `out` in canonical sli syntax. The type
/// checker uses this to render mismatch diagnostics.
pub(crate) fn parse_print_type(out: &mut String, t: &Type) {
    fn helper(out: &mut String, t: &Type) {
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
                helper(out, t);
                out.push('>');
            }
            Type::Res(t) => {
                out.push_str("Res<");
                helper(out, t);
                out.push('>');
            }
            Type::ResMut(t) => {
                out.push_str("ResMut<");
                helper(out, t);
                out.push('>');
            }
            Type::Array(t) => {
                out.push_str("Array<");
                helper(out, t);
                out.push('>');
            }
            Type::Map(k, v) => {
                out.push_str("Map<");
                helper(out, k);
                out.push_str(", ");
                helper(out, v);
                out.push('>');
            }
            Type::Fn(ps, r) => {
                out.push_str("fn(");
                for (i, p) in ps.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    helper(out, p);
                }
                out.push_str(") -> ");
                helper(out, r);
            }
            Type::Struct(n) => out.push_str(n),
        }
    }
    helper(out, t);
}
