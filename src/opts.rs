// opts.rs — Command-line options for cc1.

use crate::target::Target;

/// Emission mode — what the compiler ultimately outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitMode {
    /// LLVM IR text (.ll) — the original backend.
    LlvmIr,
    /// Assembly text (.s) via the native backend.
    Asm,
    /// ELF object file (.o) via native backend + builtin assembler.
    Obj,
    /// ELF executable via native backend + builtin assembler + builtin linker.
    Exe,
}

/// Command-line options for cc1.
#[derive(Debug)]
pub struct Opts {
    pub input: String,
    pub output: Option<String>,
    pub target: Target,
    pub emit_mode: EmitMode,
    pub emit_debug: bool,
    pub dump_tokens: bool,
    pub dump_ast: bool,
    pub dump_types: bool,
    pub dump_ir: bool,
    pub dump_ssa_ir: bool,
    pub preprocess_only: bool,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        if args.is_empty() {
            return Err("no input file".into());
        }

        let mut input: Option<String> = None;
        let mut output: Option<String> = None;
        let mut target = Target::I386; // Default per subject
        let mut emit_mode = EmitMode::LlvmIr;
        let mut emit_debug = false;
        let mut dump_tokens = false;
        let mut dump_ast = false;
        let mut dump_types = false;
        let mut dump_ir = false;
        let mut dump_ssa_ir = false;
        let mut preprocess_only = false;
        let mut i = 0;

        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "-o" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("-o requires an argument".into());
                    }
                    output = Some(args[i].clone());
                }
                // -oFILE (no space)
                a if a.starts_with("-o") && a.len() > 2 => {
                    output = Some(a[2..].to_string());
                }
                "--target" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("--target requires an argument".into());
                    }
                    target = Target::from_str(&args[i])?;
                }
                "-m32" => target = Target::I386,
                "-m64" | "--target=x86_64" => target = Target::X86_64,
                "-g" => emit_debug = true,
                "--dump-tokens" => dump_tokens = true,
                "--dump-ast" => dump_ast = true,
                "--dump-types" => dump_types = true,
                "--dump-ir" => dump_ir = true,
                "--dump-ssa-ir" => dump_ssa_ir = true,
                "--emit-asm" | "-S" => emit_mode = EmitMode::Asm,
                "--emit-obj" | "-c" => emit_mode = EmitMode::Obj,
                "--emit-exe" => emit_mode = EmitMode::Exe,
                "--emit-llvm" => emit_mode = EmitMode::LlvmIr,
                "-E" => preprocess_only = true,
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                "--version" | "-v" => {
                    println!("cc1 0.1.0 — C89 front-end for LLVM");
                    std::process::exit(0);
                }
                a if a.starts_with('-') => {
                    return Err(format!("unknown option: {}", a));
                }
                _ => {
                    if input.is_some() {
                        return Err("multiple input files not supported".into());
                    }
                    input = Some(arg.clone());
                }
            }
            i += 1;
        }

        let input = input.ok_or("no input file")?;
        Ok(Opts {
            input,
            output,
            target,
            emit_mode,
            emit_debug,
            dump_tokens,
            dump_ast,
            dump_types,
            dump_ir,
            dump_ssa_ir,
            preprocess_only,
        })
    }
}

fn print_usage() {
    eprintln!(
        "Usage: cc1 infile [-o outfile] [options]

Options:
  -o <file>         Write output to <file> (default: stdout)
  --target <arch>   Set target architecture (i386, x86_64)
  -m32              Target i386 (default)
  -m64              Target x86_64
  -g                Emit LLVM debug metadata (DWARF)
  -E                Preprocess only
  --dump-tokens     Dump token stream and exit
  --dump-ast        Dump AST and exit
  --dump-types      Dump type information and exit
  --dump-ir         Dump LLVM IR to stderr
  --dump-ssa-ir     Dump SSA IR to stderr
  -S, --emit-asm    Emit assembly text via native backend
  -c, --emit-obj    Emit ELF object file via native backend
  --emit-exe        Emit ELF executable via native backend
  --emit-llvm       Emit LLVM IR text (default)
  --help, -h        Show this help
  --version, -v     Show version"
    );
}
