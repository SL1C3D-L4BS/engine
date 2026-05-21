//! Diagnostics: structured error reports with source rendering.
//!
//! The compiler accumulates [`Diagnostic`]s instead of bailing on the first
//! error; the front-end's `compile()` returns both partial output and the
//! full diagnostic list so the editor can underline every problem in one
//! pass (spec III.2).

use crate::source::{SourceMap, Span};

/// Severity of a [`Diagnostic`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Compilation cannot continue past this diagnostic's stage.
    Error,
    /// Compilation continues; the diagnostic flags suspicious code.
    Warning,
    /// Informational note attached to another diagnostic.
    Note,
}

/// One diagnostic: severity, primary span, message, and optional notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity level.
    pub severity: Severity,
    /// Primary source location.
    pub span: Span,
    /// Short, single-line message describing the diagnostic.
    pub message: String,
    /// Optional sub-notes pointing at related spans.
    pub notes: Vec<(Span, String)>,
}

impl Diagnostic {
    /// Constructs an error-level diagnostic.
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            span,
            message: message.into(),
            notes: Vec::new(),
        }
    }

    /// Constructs a warning-level diagnostic.
    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            span,
            message: message.into(),
            notes: Vec::new(),
        }
    }

    /// Adds a note pointing at `span`.
    pub fn with_note(mut self, span: Span, message: impl Into<String>) -> Self {
        self.notes.push((span, message.into()));
        self
    }

    /// Renders the diagnostic with one line of source context.
    pub fn render(&self, sm: &SourceMap) -> String {
        let src = sm.get(self.span.file);
        let (line, col) = src.line_col(self.span.lo);
        let kind = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        let mut out = format!(
            "{kind}: {msg}\n --> {file}:{line}:{col}\n",
            msg = self.message,
            file = src.name,
        );
        if let Some(text) = src.line_text(line) {
            out.push_str(&format!("  | {text}\n"));
        }
        for (s, m) in &self.notes {
            let s_src = sm.get(s.file);
            let (l, c) = s_src.line_col(s.lo);
            out.push_str(&format!("note: {m}\n --> {f}:{l}:{c}\n", f = s_src.name));
        }
        out
    }
}

/// Sink for diagnostics produced during one compilation pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// An empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `diag`.
    pub fn emit(&mut self, diag: Diagnostic) {
        self.items.push(diag);
    }

    /// Borrows the accumulated diagnostics.
    pub fn all(&self) -> &[Diagnostic] {
        &self.items
    }

    /// Number of recorded diagnostics.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the sink is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// `true` if any diagnostic is [`Severity::Error`].
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Error)
    }

    /// Consumes the sink, returning the diagnostic vector.
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }

    /// Appends every diagnostic from `other` into `self`.
    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }
}
