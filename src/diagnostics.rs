// diagnostics.rs — Diagnostic engine for compiler errors, warnings, and notes.
//
// All error paths go through DiagEngine. No panics in production paths.
// Format: "file:line:col: severity: message"

use std::cell::RefCell;

use crate::source::{SourceMap, Span};

/// Severity levels for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Note,
    Warning,
    Error,
    Fatal,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Note => "note",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Fatal => "fatal error",
        }
    }

    pub fn ansi_color(self) -> &'static str {
        match self {
            Severity::Note => "\x1b[1;36m",    // bold cyan
            Severity::Warning => "\x1b[1;35m", // bold magenta
            Severity::Error => "\x1b[1;31m",   // bold red
            Severity::Fatal => "\x1b[1;31m",   // bold red
        }
    }
}

/// A single diagnostic message with location and optional secondary spans.
#[derive(Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub span: Span,
    pub message: String,
    /// Secondary notes (e.g., "previous definition was here").
    pub notes: Vec<(Span, String)>,
}

/// Thread-local diagnostic engine. Collects diagnostics for later emission.
///
/// Uses interior mutability (RefCell) so that it can be shared across
/// compiler phases without requiring &mut everywhere.
pub struct DiagEngine {
    diags: RefCell<Vec<Diagnostic>>,
    error_count: RefCell<u32>,
    warning_count: RefCell<u32>,
}

impl DiagEngine {
    pub fn new() -> Self {
        Self {
            diags: RefCell::new(Vec::new()),
            error_count: RefCell::new(0),
            warning_count: RefCell::new(0),
        }
    }

    /// Report an error at the given span.
    pub fn error(&self, span: Span, msg: impl Into<String>) {
        self.push(Severity::Error, span, msg.into());
    }

    /// Report an error with a secondary "note" span.
    pub fn error_with_note(
        &self,
        span: Span,
        msg: impl Into<String>,
        note_span: Span,
        note_msg: impl Into<String>,
    ) {
        let diag = Diagnostic {
            severity: Severity::Error,
            span,
            message: msg.into(),
            notes: vec![(note_span, note_msg.into())],
        };
        *self.error_count.borrow_mut() += 1;
        self.diags.borrow_mut().push(diag);
    }

    /// Report a warning at the given span.
    pub fn warning(&self, span: Span, msg: impl Into<String>) {
        self.push(Severity::Warning, span, msg.into());
    }

    /// Report a note at the given span.
    pub fn note(&self, span: Span, msg: impl Into<String>) {
        self.push(Severity::Note, span, msg.into());
    }

    /// Report a fatal error and return an error value.
    /// Does NOT terminate — the caller decides whether to abort.
    pub fn fatal(&self, span: Span, msg: impl Into<String>) {
        self.push(Severity::Fatal, span, msg.into());
    }

    /// Report an error without a source location.
    pub fn error_no_span(&self, msg: impl Into<String>) {
        self.error(Span::dummy(), msg);
    }

    fn push(&self, severity: Severity, span: Span, message: String) {
        match severity {
            Severity::Error | Severity::Fatal => *self.error_count.borrow_mut() += 1,
            Severity::Warning => *self.warning_count.borrow_mut() += 1,
            Severity::Note => {}
        }
        self.diags.borrow_mut().push(Diagnostic {
            severity,
            span,
            message,
            notes: Vec::new(),
        });
    }

    /// True if any errors have been reported.
    pub fn has_errors(&self) -> bool {
        *self.error_count.borrow() > 0
    }

    /// True if any warnings have been reported.
    pub fn has_warnings(&self) -> bool {
        *self.warning_count.borrow() > 0
    }

    /// Number of errors reported.
    pub fn error_count(&self) -> u32 {
        *self.error_count.borrow()
    }

    /// Emit all collected diagnostics to stderr.
    pub fn emit_all(&self, source_map: &SourceMap) {
        let diags = self.diags.borrow();
        let use_color = atty_stderr();

        for diag in diags.iter() {
            emit_one(diag, source_map, use_color);
        }

        let errors = *self.error_count.borrow();
        let warnings = *self.warning_count.borrow();
        if errors > 0 {
            eprintln!(
                "{} error{} generated{}",
                errors,
                if errors == 1 { "" } else { "s" },
                if warnings > 0 {
                    format!(
                        "; {} warning{}",
                        warnings,
                        if warnings == 1 { "" } else { "s" }
                    )
                } else {
                    String::new()
                }
            );
        }
    }

    /// Drain and return all diagnostics (for testing).
    pub fn drain(&self) -> Vec<Diagnostic> {
        self.diags.borrow_mut().drain(..).collect()
    }
}

fn emit_one(diag: &Diagnostic, source_map: &SourceMap, color: bool) {
    let reset = if color { "\x1b[0m" } else { "" };
    let bold = if color { "\x1b[1m" } else { "" };
    let sev_color = if color {
        diag.severity.ansi_color()
    } else {
        ""
    };

    if diag.span.is_dummy() {
        eprintln!(
            "{}cc1: {}{}{}: {}{}",
            bold,
            sev_color,
            diag.severity.label(),
            reset,
            bold,
            diag.message
        );
    } else {
        let (file, line, col) = source_map.span_to_location(diag.span);
        eprintln!(
            "{}{}:{}:{}: {}{}{}: {}{}{}",
            bold, file, line, col, sev_color, diag.severity.label(), reset, bold, diag.message,
            reset
        );

        // Show the source line with a caret
        let src_line = source_map.line_at_offset(diag.span.file, diag.span.lo);
        eprintln!(" {} | {}", line, src_line);
        let caret_offset = col as usize;
        let span_len = (diag.span.hi - diag.span.lo).max(1) as usize;
        let line_num_width = format!("{}", line).len();
        eprintln!(
            " {} | {}{}{}",
            " ".repeat(line_num_width),
            " ".repeat(caret_offset.saturating_sub(1)),
            if color { "\x1b[1;32m" } else { "" },
            "^".to_string() + &"~".repeat(span_len.saturating_sub(1))
        );
        if color {
            eprint!("{}", reset);
        }
    }

    // Emit notes
    for (note_span, note_msg) in &diag.notes {
        let note_diag = Diagnostic {
            severity: Severity::Note,
            span: *note_span,
            message: note_msg.clone(),
            notes: Vec::new(),
        };
        emit_one(&note_diag, source_map, color);
    }
}

/// Quick heuristic: is stderr a terminal?
fn atty_stderr() -> bool {
    // Use a simple heuristic — check if TERM is set
    std::env::var("TERM").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::FileId;

    #[test]
    fn test_diag_error_count() {
        let diag = DiagEngine::new();
        assert!(!diag.has_errors());
        diag.error(Span::dummy(), "test error");
        assert!(diag.has_errors());
        assert_eq!(diag.error_count(), 1);
    }

    #[test]
    fn test_diag_warning() {
        let diag = DiagEngine::new();
        assert!(!diag.has_warnings());
        diag.warning(Span::dummy(), "test warning");
        assert!(diag.has_warnings());
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_diag_with_note() {
        let diag = DiagEngine::new();
        let sp1 = Span::new(FileId(0), 0, 5);
        let sp2 = Span::new(FileId(0), 10, 15);
        diag.error_with_note(sp1, "redefinition of 'var'", sp2, "previous definition was here");
        assert_eq!(diag.error_count(), 1);
        let drained = diag.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].notes.len(), 1);
    }

    #[test]
    fn test_severity_label() {
        assert_eq!(Severity::Error.label(), "error");
        assert_eq!(Severity::Warning.label(), "warning");
        assert_eq!(Severity::Note.label(), "note");
        assert_eq!(Severity::Fatal.label(), "fatal error");
    }

    #[test]
    fn test_diag_emit_no_crash() {
        let diag = DiagEngine::new();
        let mut sm = SourceMap::new();
        let fid = sm.add_file("test.c".into(), "int x = 1;\nint x = 2;\n".into());
        diag.error(Span::new(fid, 11, 16), "redefinition of 'x'");
        diag.emit_all(&sm);
        // Just verify no panic
    }

    #[test]
    fn test_diag_drain() {
        let diag = DiagEngine::new();
        diag.error(Span::dummy(), "err1");
        diag.error(Span::dummy(), "err2");
        let drained = diag.drain();
        assert_eq!(drained.len(), 2);
    }
}
