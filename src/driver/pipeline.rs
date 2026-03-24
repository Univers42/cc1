// driver/pipeline.rs — Driver struct, compilation pipeline, and run modes.
//
// The Driver struct holds every piece of configuration parsed from the command
// line. All fields are pub(super) (visible within the driver module) and
// populated exclusively by parse_cli_args(). The run() method dispatches
// through the correct pipeline based on CompileMode.

use std::time::Instant;

use crate::ctx::Ctx;
use crate::diagnostics::DiagEngine;
use crate::source::SourceMap;
use crate::target::Target;

// ── Types ──────────────────────────────────────────────────────────────

/// Compilation pipeline stop-point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileMode {
    /// -E: preprocess only → stdout / file.
    PreprocessOnly,
    /// -S: compile to assembly → .s file.
    AssemblyOnly,
    /// -c: compile + assemble → .o file.
    ObjectOnly,
    /// Default: compile + assemble + link → executable.
    Full,
}

/// A -D macro definition from the command line.
#[derive(Debug, Clone)]
pub struct CliDefine {
    pub name: String,
    pub value: Option<String>,
}

/// Warning configuration from -W flags.
#[derive(Debug, Clone)]
pub struct WarningConfig {
    /// -Wall
    pub all: bool,
    /// -Wextra
    pub extra: bool,
    /// -Werror (all warnings → errors)
    pub error: bool,
    /// -Wpedantic
    pub pedantic: bool,
    /// -w (suppress all warnings)
    pub suppress_all: bool,
    /// Individual -Werror=<flag> entries.
    pub error_flags: Vec<String>,
    /// Individual -Wno-<flag> entries.
    pub disabled_flags: Vec<String>,
    /// Individual -W<flag> entries (enabled).
    pub enabled_flags: Vec<String>,
}

impl WarningConfig {
    pub fn new() -> Self {
        Self {
            all: true,
            extra: false,
            error: false,
            pedantic: false,
            suppress_all: false,
            error_flags: Vec::new(),
            disabled_flags: Vec::new(),
            enabled_flags: Vec::new(),
        }
    }
}

/// Diagnostic color mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

/// Code-generation options threaded to the backend.
#[derive(Debug, Clone)]
pub struct CodegenOptions {
    pub pic: bool,
    pub function_return_thunk: bool,
    pub indirect_branch_thunk: bool,
    pub patchable_function_entry: Option<(u32, u32)>,
    pub cf_protection_branch: bool,
    pub no_sse: bool,
    pub general_regs_only: bool,
    pub code_model_kernel: bool,
    pub no_jump_tables: bool,
    pub no_relax: bool,
    pub debug_info: bool,
    pub function_sections: bool,
    pub data_sections: bool,
    pub code16gcc: bool,
    pub regparm: u8,
    pub omit_frame_pointer: bool,
    pub emit_cfi: bool,
}

// ── TempFile RAII guard ────────────────────────────────────────────────

/// RAII guard for temporary files. Deletes the file on drop unless kept.
#[allow(dead_code)]
struct TempFile {
    path: String,
    keep: bool,
}

impl TempFile {
    fn new(path: String) -> Self {
        Self { path, keep: false }
    }

    #[allow(dead_code)]
    fn keep(&mut self) {
        self.keep = true;
    }

    #[allow(dead_code)]
    fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// ── Driver Struct ──────────────────────────────────────────────────────

/// The Driver holds every piece of configuration parsed from the CLI.
///
/// Fields are pub(super) — visible only within the driver module.
/// Populated exclusively by `parse_cli_args()`. Created with `Driver::new()`.
