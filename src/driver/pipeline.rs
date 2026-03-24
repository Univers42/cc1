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

    // ── Target info ────────────────────────────────────────────────

    /// Target triple string for build system probes.
    pub fn target_triple(&self) -> &'static str {
        match self.target {
            Target::X86_64 => "x86_64-linux-gnu",
            Target::I386 => "i686-linux-gnu",
        }
    }

    // ── Run modes ──────────────────────────────────────────────────

    /// -E: Preprocess only → stdout / file.
    fn run_preprocess_only(&self) -> Result<(), String> {
        for input in &self.input_files {
            // For now, without a full preprocessor, we just read the source
            // and output it. Assembly source with -x assembler-with-cpp is
            // handled similarly.
            let source = self.read_source_file(input)?;

            let output = if self.suppress_line_markers {
                super::file_types::strip_line_markers(&source)
            } else {
                source
            };

            if self.output_path_set {
                std::fs::write(&self.output_path, &output)
                    .map_err(|e| format!("cannot write to '{}': {}", self.output_path, e))?;
            } else {
                print!("{}", output);
            }
        }
        Ok(())
    }

    /// -S: Compile to assembly → .s file.
    fn run_assembly_only(&self) -> Result<(), String> {
        for input in &self.input_files {
            if !super::file_types::is_c_source(input)
                && !matches!(self.explicit_language.as_deref(), Some("c"))
            {
                return Err(format!("cannot compile '{}' with -S (not a C source file)", input));
            }

            let asm = self.compile_to_assembly(input)?;

            let out_path = if self.output_path_set {
                self.output_path.clone()
            } else {
                derive_output_path(input, ".s")
            };

            std::fs::write(&out_path, &asm)
                .map_err(|e| format!("cannot write to '{}': {}", out_path, e))?;

            if self.verbose {
                eprintln!("cc1: wrote assembly to '{}'", out_path);
            }
        }
        Ok(())
    }

    /// -c: Compile + assemble → .o file.
    fn run_object_only(&self) -> Result<(), String> {
        for input in &self.input_files {
            let obj_bytes = if super::file_types::is_assembly_source(input)
                || super::file_types::is_explicit_assembly(self.explicit_language.as_deref())
            {
                // Assembly source → assemble directly.
                self.assemble_source(input)?
            } else if super::file_types::is_c_source(input)
                || matches!(self.explicit_language.as_deref(), Some("c"))
            {
                // C source → compile to asm → assemble.
                let asm = self.compile_to_assembly(input)?;
                self.assemble_text(&asm)?
            } else {
                return Err(format!(
                    "don't know what to do with '{}' (use -x to specify language)",
                    input
                ));
            };

            let out_path = if self.output_path_set {
                self.output_path.clone()
            } else {
                derive_output_path(input, ".o")
            };

            std::fs::write(&out_path, &obj_bytes)
                .map_err(|e| format!("cannot write to '{}': {}", out_path, e))?;

            if self.verbose {
                eprintln!("cc1: wrote object to '{}'", out_path);
            }
        }
        Ok(())
    }

    /// Default: Compile + assemble + link → executable.
    fn run_full(&self) -> Result<(), String> {
        let mut temp_objects: Vec<TempFile> = Vec::new();
        let mut link_objects: Vec<Vec<u8>> = Vec::new();
        let mut passthrough_paths: Vec<String> = Vec::new();
        let mut need_start_stub = true;

        for input in &self.input_files {
            if super::file_types::is_c_source(input)
                || matches!(self.explicit_language.as_deref(), Some("c"))
            {
                // C source → compile → assemble → temp .o
                let mut asm = self.compile_to_assembly(input)?;

                // Inject _start stub into the first compiled object so that
                // cross-object symbol resolution isn't needed for `call main`.
                if need_start_stub && !self.nostdlib && !self.shared_lib && !self.relocatable {
                    let stub = self.generate_start_stub();
                    asm = format!("{}{}", stub, asm);
                    need_start_stub = false;
                }

                let obj_bytes = self.assemble_text(&asm)?;

                let temp_path = derive_temp_path(input, ".o");
                std::fs::write(&temp_path, &obj_bytes)
                    .map_err(|e| format!("cannot write temp object '{}': {}", temp_path, e))?;

                let tf = TempFile::new(temp_path);
                link_objects.push(obj_bytes);
                temp_objects.push(tf);
            } else if super::file_types::is_assembly_source(input)
                || super::file_types::is_explicit_assembly(self.explicit_language.as_deref())
            {
                // Assembly source → assemble → temp .o
                let obj_bytes = self.assemble_source(input)?;

                let temp_path = derive_temp_path(input, ".o");
                std::fs::write(&temp_path, &obj_bytes)
                    .map_err(|e| format!("cannot write temp object '{}': {}", temp_path, e))?;

                let tf = TempFile::new(temp_path);
                link_objects.push(obj_bytes);
                temp_objects.push(tf);
            } else if super::file_types::is_object_or_archive(input) {
                // Object/archive → pass through to linker.
                passthrough_paths.push(input.clone());
            } else if super::file_types::looks_like_binary_object(input) {
                // Unknown extension but ELF/ar magic → pass through.
                passthrough_paths.push(input.clone());
            } else {
                return Err(format!(
                    "don't know what to do with '{}' (use -x to specify language)",
                    input
                ));
            }
        }

        // Also collect ordered linker items from -l, -Wl,, etc.
        for item in &self.linker_ordered_items {
            if item.starts_with("-l") || item.starts_with("-Wl,") || item.starts_with("-") {
                // Linker flag — passed through
                passthrough_paths.push(item.clone());
            } else if super::file_types::is_object_or_archive(item)
                || super::file_types::looks_like_binary_object(item)
            {
                passthrough_paths.push(item.clone());
            }
        }

        // Read passthrough objects into memory for the builtin linker.
        for path in &passthrough_paths {
            // Skip linker flags (they start with -)
            if path.starts_with('-') {
                continue;
            }
            match std::fs::read(path) {
                Ok(data) => link_objects.push(data),
                Err(e) => return Err(format!("cannot read '{}': {}", path, e)),
            }
        }

        if link_objects.is_empty() {
            return Err("no input files to link".into());
        }

        // Verbose mode: print synthetic link line for CMake compatibility.
        if self.verbose {
            let mut link_line = String::from("/usr/bin/ld");
            for p in &self.linker_paths {
                link_line.push_str(&format!(" -L{}", p));
            }
            for p in &passthrough_paths {
                link_line.push_str(&format!(" {}", p));
            }
            eprintln!("{}", link_line);
        }

        // If no C source was compiled, the start stub hasn't been injected yet.
        // Assemble it as a standalone object.
        if need_start_stub && !self.nostdlib && !self.shared_lib && !self.relocatable {
            let stub = self.generate_start_stub();
            let stub_obj = self.assemble_text(&stub)?;
            // Prepend so _start is found before main.
            link_objects.insert(0, stub_obj);
        }

        // Link into executable.
        let exe_bytes = crate::backend::native::linker::link(&link_objects, self.target);

        std::fs::write(&self.output_path, &exe_bytes)
            .map_err(|e| format!("cannot write to '{}': {}", self.output_path, e))?;

        // Make the output executable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            let _ = std::fs::set_permissions(&self.output_path, perms);
        }

        if self.verbose {
            eprintln!("cc1: wrote executable to '{}'", self.output_path);
        }

        // temp_objects are cleaned up when they drop here
        drop(temp_objects);
        Ok(())
    }

    // ── Core compilation pipeline ──────────────────────────────────

    /// Compile a C source file to target-specific assembly text.
    ///
    /// This is the heart of the driver. The pipeline has 9 phases:
    ///   1. Preprocessor (TODO: not yet implemented, source read directly)
    ///   2. Lexer
    ///   3. Parser
    ///   4. Sema
    ///   5. Lowerer (AST → alloca-based IR)
    ///   6. mem2reg (TODO: promote allocas to SSA)
    ///   7. Optimization (TODO: constant fold, DCE, GVN, etc.)
    ///   8. Phi elimination (TODO: SSA phi nodes → Copy instructions)
    ///   9. Codegen (IR → target-specific assembly text)
    fn compile_to_assembly(&self, input_path: &str) -> Result<String, String> {
        let time_phases = std::env::var("CCC_TIME_PHASES").is_ok();
        let total_start = Instant::now();

        // ── Phase 1: Preprocessor ──────────────────────────────────
        // TODO: When the preprocessor is implemented, this will:
        //   - configure_preprocessor() with target macros, -D/-U, include paths
        //   - process force-include files
        //   - expand macros, process #include/#ifdef/#pragma
        // For now, we read the source directly.
        let phase_start = Instant::now();
        let source = self.read_source_file(input_path)?;
        if time_phases {
            eprintln!("[TIME] preprocess: {:.3}s", phase_start.elapsed().as_secs_f64());
        }

        // Create the diagnostic engine.
        let diag = DiagEngine::new();
        let mut source_map = SourceMap::new();
        let file_id = source_map.add_file(input_path.to_string(), source.clone());

        // ── Phase 2: Lexer ─────────────────────────────────────────
        let phase_start = Instant::now();
        let tokens = crate::frontend::lexer::lex(&source_map, file_id, &diag);
        if time_phases {
            eprintln!(
                "[TIME] lex: {:.3}s ({} tokens)",
                phase_start.elapsed().as_secs_f64(),
                tokens.len()
            );
        }
        if diag.has_errors() {
            diag.emit_all(&source_map);
            return Err(format!("lexer errors in '{}'", input_path));
        }

        // ── Phase 3: Parser ────────────────────────────────────────
        let phase_start = Instant::now();
        let mut ctx = Ctx::new(self.target);
        let translation_unit = crate::frontend::parser::parse(&tokens, &mut ctx, &diag);
        if time_phases {
            eprintln!("[TIME] parse: {:.3}s", phase_start.elapsed().as_secs_f64());
        }
        if diag.has_errors() {
            diag.emit_all(&source_map);
            return Err(format!("parse errors in '{}'", input_path));
        }

        // ── Phase 4: Sema ──────────────────────────────────────────
        let phase_start = Instant::now();
        crate::frontend::sema::analyze(&mut ctx, translation_unit, &diag);
        if time_phases {
            eprintln!("[TIME] sema: {:.3}s", phase_start.elapsed().as_secs_f64());
        }
        if diag.has_errors() {
            diag.emit_all(&source_map);
            return Err(format!("semantic errors in '{}'", input_path));
        }
        // Check for warnings promoted to errors by -Werror.
        if self.warning_config.error && diag.has_warnings() {
            diag.emit_all(&source_map);
            return Err("warnings treated as errors".into());
        }

        // ── Phase 5: Lowerer (AST → alloca-based IR) ──────────────
        let phase_start = Instant::now();
        let ir_module = crate::ir::lower::Lowering::lower(&ctx, translation_unit, input_path);
        if time_phases {
            let func_count = ir_module.functions.len();
            eprintln!(
                "[TIME] lowering: {:.3}s ({} functions)",
                phase_start.elapsed().as_secs_f64(),
                func_count
            );
        }

        // ── Post-lowering transformations ──────────────────────────
        // TODO: #pragma weak, #pragma redefine_extname, -fcommon

        // ── Phase 6: mem2reg ───────────────────────────────────────
        // TODO: Promote allocas to SSA phi nodes (currently handled as no-op in optimizer).
        if time_phases {
            eprintln!("[TIME] mem2reg: 0.000s (integrated into optimizer)");
        }

        // ── Phase 7: Optimization passes ───────────────────────────
        let phase_start_opt = Instant::now();
        let mut ir_module = ir_module;
        crate::ir::optimize::optimize(&mut ir_module, self.target);
        if time_phases {
            eprintln!(
                "[TIME] opt passes: {:.3}s",
                phase_start_opt.elapsed().as_secs_f64()
            );
        }

        // ── Phase 8: Phi elimination ───────────────────────────────
        // TODO: SSA phi nodes → Copy instructions.
        if time_phases {
            eprintln!("[TIME] phi elimination: 0.000s (not yet implemented)");
        }

        // ── Phase 9: Codegen (IR → assembly) ───────────────────────
        let phase_start = Instant::now();

        use crate::backend::native::generation::generate_asm;
        use crate::backend::native::x86_64::codegen::X86_64Codegen;

        let arch_codegen: Box<dyn crate::backend::native::traits::ArchCodegen> = match self.target {
            Target::X86_64 => Box::new(X86_64Codegen::new()),
            Target::I386 => {
                return Err("i386 native backend not yet implemented".into());
            }
        };

        let asm = generate_asm(arch_codegen.as_ref(), &ir_module, self.target);

        if time_phases {
            eprintln!(
                "[TIME] codegen: {:.3}s ({} bytes asm)",
                phase_start.elapsed().as_secs_f64(),
                asm.len()
            );
        }

        // Emit warnings if any.
        if diag.has_warnings() {
            diag.emit_all(&source_map);
        }

        if time_phases {
            eprintln!(
                "[TIME] total compile {}: {:.3}s",
                input_path,
                total_start.elapsed().as_secs_f64()
            );
        }

        Ok(asm)
    }

    // ── Assembly helpers ───────────────────────────────────────────

    /// Assemble an assembly source file (.s/.S) to ELF .o bytes.
    fn assemble_source(&self, path: &str) -> Result<Vec<u8>, String> {
        let source = if super::file_types::is_assembly_with_cpp(path) {
            // .S files need C preprocessing first.
            // TODO: Run the built-in C preprocessor with __ASSEMBLER__ defined.
            // For now, we just read the file directly.
            self.read_source_file(path)?
        } else {
            std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read '{}': {}", path, e))?
        };
        self.assemble_text(&source)
    }

    /// Assemble AT&T syntax assembly text to ELF .o bytes using the builtin assembler.
    fn assemble_text(&self, asm_text: &str) -> Result<Vec<u8>, String> {
        match self.target {
            Target::X86_64 => {
                use crate::backend::native::x86_64::assembler::X86_64Assembler;
                let mut assembler = X86_64Assembler::new();
                Ok(assembler.assemble(asm_text))
            }
            Target::I386 => {
                Err("i386 builtin assembler not yet implemented".into())
            }
        }
    }

    /// Generate the _start stub that calls main and exits via syscall.
    fn generate_start_stub(&self) -> String {
        match self.target {
            Target::X86_64 => {
                concat!(
                    "        .text\n",
                    "        .globl  _start\n",
                    "        .type   _start, @function\n",
                    "_start:\n",
                    "        xorl    %ebp, %ebp\n",
                    "        call    main\n",
                    "        movl    %eax, %edi\n",
                    "        movl    $60, %eax\n",
                    "        syscall\n",
                    "        .size   _start, .-_start\n",
                ).to_string()
            }
            Target::I386 => {
                concat!(
                    "        .text\n",
                    "        .globl  _start\n",
                    "        .type   _start, @function\n",
                    "_start:\n",
                    "        xorl    %ebp, %ebp\n",
                    "        call    main\n",
                    "        movl    %eax, %ebx\n",
                    "        movl    $1, %eax\n",
                    "        int     $0x80\n",
                    "        .size   _start, .-_start\n",
                ).to_string()
            }
        }
    }

    // ── Source file reading ────────────────────────────────────────

    /// Read a source file, handling non-UTF-8 content by encoding raw bytes
    /// as Private Use Area code points (U+E080-U+E0FF).
    fn read_source_file(&self, path: &str) -> Result<String, String> {
        let raw = std::fs::read(path)
            .map_err(|e| format!("cannot read '{}': {}", path, e))?;

        // Try UTF-8 first.
        match String::from_utf8(raw.clone()) {
            Ok(s) => Ok(s),
            Err(_) => {
                // Encode non-UTF-8 bytes as PUA code points.
                let mut result = String::with_capacity(raw.len());
                for &byte in &raw {
                    if byte < 0x80 {
                        result.push(byte as char);
                    } else {
                        // Map 0x80-0xFF → U+E080-U+E0FF
                        let cp = 0xE000 + (byte as u32);
                        if let Some(c) = char::from_u32(cp) {
                            result.push(c);
                        }
                    }
                }
                Ok(result)
            }
        }
    }

    // ── Dependency file generation ─────────────────────────────────

    /// Write a Make-compatible dependency file.
    #[allow(dead_code)]
    fn write_dep_file(&self, source: &str) -> Result<(), String> {
        let dep_path = match &self.dep_file {
            Some(p) => p.clone(),
            None => {
                // Derive from output path by replacing extension.
                let base = if self.output_path_set {
                    &self.output_path
                } else {
                    source
                };
                replace_extension(base, ".d")
            }
        };

        let target = match &self.dep_target {
            Some(t) => t.clone(),
            None => {
                let ext = match self.mode {
                    CompileMode::AssemblyOnly => ".s",
                    CompileMode::ObjectOnly => ".o",
                    _ => ".o",
                };
                if self.output_path_set {
                    self.output_path.clone()
                } else {
                    derive_output_path(source, ext)
                }
            }
        };

        let content = format!("{}: {}\n", target, source);
        std::fs::write(&dep_path, &content)
            .map_err(|e| format!("cannot write dependency file '{}': {}", dep_path, e))
    }
}

// ── Utility functions ──────────────────────────────────────────────────

/// Derive an output path by replacing the extension of the input path.
fn derive_output_path(input: &str, new_ext: &str) -> String {
    if let Some(dot) = input.rfind('.') {
        format!("{}{}", &input[..dot], new_ext)
    } else {
        format!("{}{}", input, new_ext)
    }
}

/// Derive a temporary object path from an input file.
fn derive_temp_path(input: &str, ext: &str) -> String {
    use std::process;
    let base = std::path::Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tmp");
    format!("/tmp/cc1_{}_{}{}", base, process::id(), ext)
}

/// Replace the extension of a path.
fn replace_extension(path: &str, new_ext: &str) -> String {
    if let Some(dot) = path.rfind('.') {
        format!("{}{}", &path[..dot], new_ext)
    } else {
        format!("{}{}", path, new_ext)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_new_defaults() {
        let d = Driver::new();
        assert_eq!(d.mode, CompileMode::Full);
        assert_eq!(d.output_path, "a.out");
        assert!(!d.output_path_set);
        assert!(d.input_files.is_empty());
        assert!(!d.optimize);
        assert!(d.gnu_extensions);
        assert!(!d.debug_info);
        assert!(!d.pic);
        assert_eq!(d.opt_level, 2);
    }

    #[test]
    fn test_compile_mode_variants() {
        assert_ne!(CompileMode::PreprocessOnly, CompileMode::Full);
        assert_ne!(CompileMode::AssemblyOnly, CompileMode::ObjectOnly);
    }

    #[test]
    fn test_warning_config_defaults() {
        let w = WarningConfig::new();
        assert!(w.all);
        assert!(!w.extra);
        assert!(!w.error);
        assert!(!w.suppress_all);
    }

    #[test]
    fn test_derive_output_path() {
        assert_eq!(derive_output_path("foo.c", ".s"), "foo.s");
        assert_eq!(derive_output_path("path/to/bar.c", ".o"), "path/to/bar.o");
        assert_eq!(derive_output_path("noext", ".o"), "noext.o");
    }

    #[test]
    fn test_codegen_options_pic_forced_with_shared() {
        let mut d = Driver::new();
        d.shared_lib = true;
        d.pic = false;
        let opts = d.codegen_options();
        assert!(opts.pic);
    }

    #[test]
    fn test_codegen_options_cfi_from_unwind() {
        let mut d = Driver::new();
        assert!(d.codegen_options().emit_cfi);
        d.no_unwind_tables = true;
        assert!(!d.codegen_options().emit_cfi);
    }

    #[test]
    fn test_target_triple() {
        let mut d = Driver::new();
        d.target = Target::X86_64;
        assert_eq!(d.target_triple(), "x86_64-linux-gnu");
        d.target = Target::I386;
        assert_eq!(d.target_triple(), "i686-linux-gnu");
    }

    #[test]
    fn test_replace_extension() {
        assert_eq!(replace_extension("foo.c", ".d"), "foo.d");
        assert_eq!(replace_extension("a/b/c.o", ".d"), "a/b/c.d");
    }
}
