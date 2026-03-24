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
pub struct Driver {
    // ── Target and output ──────────────────────────────────────────
    /// Architecture (detected from binary name or -m32/-m16).
    pub(super) target: Target,
    /// Output file path (from -o).
    pub(super) output_path: String,
    /// Whether -o was explicitly given.
    pub(super) output_path_set: bool,
    /// Input source/object/archive paths.
    pub(super) input_files: Vec<String>,
    /// Pipeline stop point (from -E/-S/-c).
    pub(super) mode: CompileMode,

    // ── Optimization ───────────────────────────────────────────────
    /// Internal optimization level (always 2; all levels run the same passes).
    pub(super) opt_level: u32,
    /// Whether user passed -O1 or higher (defines __OPTIMIZE__).
    pub(super) optimize: bool,
    /// Whether \-Os/\-Oz (defines __OPTIMIZE_SIZE__).
    pub(super) optimize_size: bool,

    // ── Preprocessor ───────────────────────────────────────────────
    /// -D macro definitions.
    pub(super) defines: Vec<CliDefine>,
    /// -I include search paths.
    pub(super) include_paths: Vec<String>,
    /// -iquote paths (searched only for #include "file").
    pub(super) quote_include_paths: Vec<String>,
    /// -isystem system include paths.
    pub(super) isystem_include_paths: Vec<String>,
    /// -idirafter paths (searched last).
    pub(super) after_include_paths: Vec<String>,
    /// -include files (processed before main source).
    pub(super) force_includes: Vec<String>,
    /// -U macro undefinitions.
    pub(super) undef_macros: Vec<String>,
    /// -undef (suppress all predefined macros).
    pub(super) undef_all: bool,
    /// GNU C extensions enabled (false with -std=c99 etc.).
    pub(super) gnu_extensions: bool,
    /// GNU89 inline semantics (-fgnu89-inline or -std=gnu89).
    pub(super) gnu89_inline: bool,
    /// -nostdinc (no default system include paths).
    pub(super) nostdinc: bool,
    /// -P (strip # line markers from -E output).
    pub(super) suppress_line_markers: bool,
    /// -dM (dump all #defines instead of preprocessed text).
    pub(super) dump_defines: bool,
    /// -x language override (e.g., "c", "assembler-with-cpp").
    pub(super) explicit_language: Option<String>,

    // ── Code generation ────────────────────────────────────────────
    /// -g (emit DWARF debug info).
    pub(super) debug_info: bool,
    /// -fPIC/-fpic (position-independent code).
    pub(super) pic: bool,
    /// -mfunction-return=thunk-extern (Spectre v2 retpoline).
    pub(super) function_return_thunk: bool,
    /// -mindirect-branch=thunk-extern (retpoline for indirect calls).
    pub(super) indirect_branch_thunk: bool,
    /// -fpatchable-function-entry=N,M (NOP padding for ftrace).
    pub(super) patchable_function_entry: Option<(u32, u32)>,
    /// -fcf-protection=branch (Intel CET endbr64).
    pub(super) cf_protection_branch: bool,
    /// -mno-sse (avoid all SSE/XMM instructions).
    pub(super) no_sse: bool,
    /// -msse3 through -mavx2 SIMD feature flags.
    pub(super) enable_sse3: bool,
    pub(super) enable_ssse3: bool,
    pub(super) enable_sse4_1: bool,
    pub(super) enable_sse4_2: bool,
    pub(super) enable_avx: bool,
    pub(super) enable_avx2: bool,
    /// -mgeneral-regs-only (no FP/SIMD registers; AArch64 kernel).
    pub(super) general_regs_only: bool,
    /// -mcmodel=kernel (negative 2GB address space).
    pub(super) code_model_kernel: bool,
    /// -fno-jump-tables (compare-and-branch chains for switches).
    pub(super) no_jump_tables: bool,
    /// -ffunction-sections (each function in its own section).
    pub(super) function_sections: bool,
    /// -fdata-sections (each global in its own section).
    pub(super) data_sections: bool,
    /// -m16 (16-bit real mode boot code).
    pub(super) code16gcc: bool,
    /// -mregparm=N (i686: pass first N int args in registers).
    pub(super) regparm: u8,
    /// -fomit-frame-pointer (free EBP as GP register).
    pub(super) omit_frame_pointer: bool,
    /// -fno-asynchronous-unwind-tables (suppress .eh_frame).
    pub(super) no_unwind_tables: bool,
    /// -fcommon (COMMON linkage for tentative definitions).
    pub(super) fcommon: bool,

    // ── RISC-V specific ────────────────────────────────────────────
    /// -mabi= override (e.g., lp64, lp64d).
    pub(super) riscv_abi: Option<String>,
    /// -march= override (e.g., rv64imac_zicsr_zifencei).
    pub(super) riscv_march: Option<String>,
    /// -mno-relax (suppress linker relaxation).
    pub(super) riscv_no_relax: bool,

    // ── Linker ─────────────────────────────────────────────────────
    /// -L library search paths.
    pub(super) linker_paths: Vec<String>,
    /// Ordered list of -l, -Wl,, and object/archive paths.
    pub(super) linker_ordered_items: Vec<String>,
    /// -static.
    pub(super) static_link: bool,
    /// -shared.
    pub(super) shared_lib: bool,
    /// -nostdlib.
    pub(super) nostdlib: bool,
    /// -r (relocatable link; merge .o files).
    pub(super) relocatable: bool,

    // ── Diagnostics ────────────────────────────────────────────────
    /// Warning enable/disable/error state (from -W flags).
    pub(super) warning_config: WarningConfig,
    /// -fdiagnostics-color={auto,always,never}.
    pub(super) color_mode: ColorMode,
    /// -v / --verbose.
    pub(super) verbose: bool,

    // ── Dependency generation ──────────────────────────────────────
    /// Dependency file path (from -MF or -Wp,-MMD,path).
    pub(super) dep_file: Option<String>,
    /// -M/-MM (output dependency rules, no compilation).
    pub(super) dep_only: bool,
    /// -MT (override target name in dependency rule).
    pub(super) dep_target: Option<String>,

    // ── Other ──────────────────────────────────────────────────────
    /// -pthread (defines _REENTRANT).
    pub(super) pthread: bool,
    /// -Wa, assembler passthrough flags.
    pub(super) assembler_extra_args: Vec<String>,
    /// Raw CLI args for GCC -m16 passthrough.
    pub(super) raw_args: Vec<String>,
}

impl Driver {
    /// Create a new Driver with sensible defaults.
    pub fn new() -> Self {
        Self {
            target: Target::X86_64,
            output_path: "a.out".into(),
            output_path_set: false,
            input_files: Vec::new(),
            mode: CompileMode::Full,

            opt_level: 2,
            optimize: false,
            optimize_size: false,

            defines: Vec::new(),
            include_paths: Vec::new(),
            quote_include_paths: Vec::new(),
            isystem_include_paths: Vec::new(),
            after_include_paths: Vec::new(),
            force_includes: Vec::new(),
            undef_macros: Vec::new(),
            undef_all: false,
            gnu_extensions: true,
            gnu89_inline: false,
            nostdinc: false,
            suppress_line_markers: false,
            dump_defines: false,
            explicit_language: None,

            debug_info: false,
            pic: false,
            function_return_thunk: false,
            indirect_branch_thunk: false,
            patchable_function_entry: None,
            cf_protection_branch: false,
            no_sse: false,
            enable_sse3: false,
            enable_ssse3: false,
            enable_sse4_1: false,
            enable_sse4_2: false,
            enable_avx: false,
            enable_avx2: false,
            general_regs_only: false,
            code_model_kernel: false,
            no_jump_tables: false,
            function_sections: false,
            data_sections: false,
            code16gcc: false,
            regparm: 0,
            omit_frame_pointer: false,
            no_unwind_tables: false,
            fcommon: false,

            riscv_abi: None,
            riscv_march: None,
            riscv_no_relax: false,

            linker_paths: Vec::new(),
            linker_ordered_items: Vec::new(),
            static_link: false,
            shared_lib: false,
            nostdlib: false,
            relocatable: false,

            warning_config: WarningConfig::new(),
            color_mode: ColorMode::Auto,
            verbose: false,

            dep_file: None,
            dep_only: false,
            dep_target: None,

            pthread: false,
            assembler_extra_args: Vec::new(),
            raw_args: Vec::new(),
        }
    }

    // ── Public API ─────────────────────────────────────────────────

    /// Check if there are any input files.
    pub fn has_input_files(&self) -> bool {
        !self.input_files.is_empty()
    }

    /// Build CodegenOptions from current driver configuration.
    pub fn codegen_options(&self) -> CodegenOptions {
        CodegenOptions {
            pic: self.pic || self.shared_lib,
            function_return_thunk: self.function_return_thunk,
            indirect_branch_thunk: self.indirect_branch_thunk,
            patchable_function_entry: self.patchable_function_entry,
            cf_protection_branch: self.cf_protection_branch,
            no_sse: self.no_sse,
            general_regs_only: self.general_regs_only,
            code_model_kernel: self.code_model_kernel,
            no_jump_tables: self.no_jump_tables,
            no_relax: self.riscv_no_relax,
            debug_info: self.debug_info,
            function_sections: self.function_sections,
            data_sections: self.data_sections,
            code16gcc: self.code16gcc,
            regparm: self.regparm,
            omit_frame_pointer: self.omit_frame_pointer,
            emit_cfi: !self.no_unwind_tables,
        }
    }

    /// Main entry point — dispatch to the appropriate pipeline.
    pub fn run(&self) -> Result<(), String> {
        match self.mode {
            CompileMode::PreprocessOnly => self.run_preprocess_only(),
            CompileMode::AssemblyOnly => self.run_assembly_only(),
            CompileMode::ObjectOnly => self.run_object_only(),
            CompileMode::Full => self.run_full(),
        }
    }

