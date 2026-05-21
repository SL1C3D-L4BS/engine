//! REPL line evaluator (spec VIII.4, ADR-036).
//!
//! Lex / parse / typeck / IR / bytecode / `vm.exec` one input at a
//! time, keeping a persistent global module across inputs. The REPL
//! wraps each line in a synthetic `fn _repl() -> ...` so the
//! existing pipeline can be reused unchanged.
//!
//! Built-in dot-commands per spec VIII.4: `.help`, `.bytecode <expr>`,
//! `.type <expr>`, `.exit`, `.history`, `.clear`. `--attach` adds
//! `.ecs`, `.profile`, `.asset`; the attach surface lives in
//! `bin/engine-repl` (it needs a live engine process to talk to).

use crate::vm::{StopReason, Value, Vm};
use crate::{Compiler, Source, SourceMap};

/// Result of one REPL evaluation.
#[derive(Clone, Debug)]
pub enum Reply {
    /// The input produced a value.
    Value(Value),
    /// The input produced a value of side-effect-only kind.
    Nil,
    /// A diagnostic-class error.
    Error(String),
    /// A built-in command response, already formatted for the screen.
    Builtin(String),
    /// The user typed `.exit`.
    Exit,
}

/// One REPL session.
pub struct Repl {
    history: Vec<String>,
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

impl Repl {
    /// Constructs an empty REPL session.
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
        }
    }

    /// Borrows the history buffer.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Evaluates one input line/group. The caller is responsible for
    /// gathering multi-line input (bracket-balanced) before calling
    /// `eval` — `bin/engine-repl/src/lineedit.rs` does this. The REPL
    /// itself runs each input as a single program.
    pub fn eval(&mut self, input: &str) -> Reply {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Reply::Nil;
        }
        if let Some(cmd) = trimmed.strip_prefix('.') {
            return self.dot_command(cmd);
        }
        self.history.push(input.to_string());
        // Try the input as a standalone expression. Wrap in a function
        // and run.
        let wrapped = format!("fn _repl() -> i64 {{ return {trimmed}; }}");
        match self.run_program(&wrapped) {
            Ok(v) => Reply::Value(v),
            Err(_) => {
                // Fallback: try as a full program with an entry point
                // named `_repl`.
                let mut sm = SourceMap::new();
                let id = sm.add(Source::new("<repl>", trimmed));
                match Compiler::new().compile(id, sm.get(id)) {
                    Ok(c) if !c.diagnostics.has_errors() => {
                        let mut vm = Vm::new(c.bytecode);
                        if let Some(fn_id) = vm.module.function_id("_repl") {
                            let _ = fn_id;
                        }
                        if vm.module.function_id("_repl").is_some() {
                            match vm.call("_repl", vec![]) {
                                StopReason::Returned(v) => Reply::Value(v),
                                other => Reply::Error(format!("{other:?}")),
                            }
                        } else {
                            Reply::Nil
                        }
                    }
                    Ok(c) => Reply::Error(diag_summary(&c.diagnostics)),
                    Err((e, _)) => Reply::Error(format!("{e}")),
                }
            }
        }
    }

    fn dot_command(&mut self, cmd: &str) -> Reply {
        let (head, rest) = match cmd.split_once(' ') {
            Some((h, r)) => (h, r),
            None => (cmd, ""),
        };
        match head {
            "help" => Reply::Builtin(HELP_TEXT.to_string()),
            "exit" | "quit" => Reply::Exit,
            "history" => Reply::Builtin(self.history.join("\n")),
            "clear" => {
                self.history.clear();
                Reply::Builtin("history cleared".into())
            }
            "type" => {
                let wrapped = format!("fn _t() -> i64 {{ return {rest}; }}");
                let mut sm = SourceMap::new();
                let id = sm.add(Source::new("<repl-type>", wrapped));
                match Compiler::new().compile(id, sm.get(id)) {
                    Ok(c) if !c.diagnostics.has_errors() => Reply::Builtin(format!(
                        "{:?}",
                        c.types.functions.get("_t").map(|(_, r)| r.clone())
                    )),
                    Ok(c) => Reply::Error(diag_summary(&c.diagnostics)),
                    Err((e, _)) => Reply::Error(format!("{e}")),
                }
            }
            "bytecode" => {
                let wrapped = format!("fn _b() -> i64 {{ return {rest}; }}");
                let mut sm = SourceMap::new();
                let id = sm.add(Source::new("<repl-bc>", wrapped));
                match Compiler::new().compile(id, sm.get(id)) {
                    Ok(c) if !c.diagnostics.has_errors() => {
                        let f = c.bytecode.functions.first().expect("compiled fn");
                        Reply::Builtin(format!(
                            "code = {:?}\nmax_register = {}\nconsts = {:?}",
                            f.code, f.max_register, c.bytecode.constants
                        ))
                    }
                    Ok(c) => Reply::Error(diag_summary(&c.diagnostics)),
                    Err((e, _)) => Reply::Error(format!("{e}")),
                }
            }
            other => Reply::Error(format!("unknown command `.{other}`")),
        }
    }

    fn run_program(&mut self, source: &str) -> Result<Value, String> {
        let mut sm = SourceMap::new();
        let id = sm.add(Source::new("<repl>", source));
        let compiled = Compiler::new()
            .compile(id, sm.get(id))
            .map_err(|(e, _)| format!("{e}"))?;
        if compiled.diagnostics.has_errors() {
            return Err(diag_summary(&compiled.diagnostics));
        }
        let mut vm = Vm::new(compiled.bytecode);
        match vm.call("_repl", vec![]) {
            StopReason::Returned(v) => Ok(v),
            other => Err(format!("{other:?}")),
        }
    }
}

fn diag_summary(diags: &crate::Diagnostics) -> String {
    diags
        .all()
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

const HELP_TEXT: &str = "\
sli REPL commands:
  .help                Show this help
  .bytecode <expr>     Show compiled bytecode of <expr>
  .type <expr>         Show inferred type of <expr>
  .history             Print input history
  .clear               Clear input history
  .exit                Leave the REPL

Anything else is evaluated as an sli expression.
";

/// Splits `input` into bracket-balanced chunks, returning the number
/// of currently-unmatched openers. The REPL uses this to decide when
/// to keep reading more lines.
pub fn unmatched_brackets(input: &str) -> i32 {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for c in input.chars() {
        if in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth
}
