// cc1 — C89 Compiler Front-End for LLVM
// Entry point: CLI parsing → compilation pipeline

use std::env;
use std::process;

use cc1::ctx::Ctx;
use cc1::diagnostics::DiagEngine;
use cc1::opts::{EmitMode, Opts};
use cc1::source::SourceMap;

fn run() -> i32 {
    let args: Vec<String> = env::args().skip(1).collect();
    let opts = match Opts::parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cc1: error: {}", e);
            return 1;
        }
    };

    let mut source_map = SourceMap::new();
    let diag = DiagEngine::new();

    // Load the input file
    let file_id = match source_map.load_file(&opts.input) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("cc1: error: {}: {}", opts.input, e);
            return 1;
        }
    };

    let mut ctx = Ctx::new(opts.target);

    // Phase 1-3: Lexing
    let tokens = cc1::frontend::lexer::lex(&source_map, file_id, &diag);
    if diag.has_errors() {
        diag.emit_all(&source_map);
        return 1;
    }

    if opts.dump_tokens {
        for tok in &tokens {
            eprintln!("{:?}", tok);
        }
        return 0;
    }

    // Phase 5: Parsing
    let translation_unit = cc1::frontend::parser::parse(&tokens, &mut ctx, &diag);
    if diag.has_errors() {
        diag.emit_all(&source_map);
        return 1;
    }

    if opts.dump_ast {
        cc1::frontend::parser::ast::dump(&ctx, translation_unit);
        return 0;
    }

    // Phase 6: Semantic analysis
    cc1::frontend::sema::analyze(&mut ctx, translation_unit, &diag);
    if diag.has_errors() {
        diag.emit_all(&source_map);
        return 1;
    }

    if opts.dump_types {
        cc1::frontend::sema::dump_types(&ctx);
        return 0;
    }

    // Phase 7: Code generation
    match opts.emit_mode {
        EmitMode::LlvmIr => {
            // Original LLVM IR text backend
            let ir = cc1::backend::codegen::generate(&ctx, translation_unit, &opts);
            if diag.has_errors() {
                diag.emit_all(&source_map);
                return 1;
            }
            if let Some(ref outfile) = opts.output {
                if let Err(e) = std::fs::write(outfile, &ir) {
                    eprintln!("cc1: error: cannot write to '{}': {}", outfile, e);
                    return 1;
                }
            } else {
                print!("{}", ir);
            }
        }
        EmitMode::Asm | EmitMode::Obj | EmitMode::Exe => {
            // Native backend: SSA IR → assembly → object → executable
            use cc1::ir::lower::Lowering;
            use cc1::backend::native::generation::generate_asm;
            use cc1::backend::native::x86_64::codegen::X86_64Codegen;

            // Lower AST → SSA IR
            let ir_module = Lowering::lower(&ctx, translation_unit, &opts.input);

            // Dump SSA IR if requested
            if opts.dump_ssa_ir {
                eprintln!("{}", ir_module);
            }

            // Select architecture backend
            let arch_codegen: Box<dyn cc1::backend::native::traits::ArchCodegen> = match opts.target {
                cc1::target::Target::X86_64 => Box::new(X86_64Codegen::new()),
                cc1::target::Target::I386 => {
                    eprintln!("cc1: error: i386 native backend not yet implemented");
                    return 1;
                }
            };

            // Generate assembly text
            let asm = generate_asm(arch_codegen.as_ref(), &ir_module, opts.target);

            match opts.emit_mode {
                EmitMode::Asm => {
                    if let Some(ref outfile) = opts.output {
                        if let Err(e) = std::fs::write(outfile, &asm) {
                            eprintln!("cc1: error: cannot write to '{}': {}", outfile, e);
                            return 1;
                        }
                    } else {
                        print!("{}", asm);
                    }
                }
                EmitMode::Obj => {
                    // Assembly text → ELF .o via builtin assembler
                    use cc1::backend::native::x86_64::assembler::X86_64Assembler;
                    let mut assembler = X86_64Assembler::new();
                    let elf_bytes = assembler.assemble(&asm);
                    let outfile = opts.output.as_deref().unwrap_or("a.o");
                    if let Err(e) = std::fs::write(outfile, &elf_bytes) {
                        eprintln!("cc1: error: cannot write to '{}': {}", outfile, e);
                        return 1;
                    }
                }
                EmitMode::Exe => {
                    // Assembly text → ELF .o → ELF exe via builtin linker
                    // Inject a _start stub that calls main and exits
                    use cc1::backend::native::x86_64::assembler::X86_64Assembler;
                    use cc1::backend::native::linker::link;

                    let start_stub = concat!(
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
                    );
                    let full_asm = format!("{}{}", start_stub, asm);

                    let mut assembler = X86_64Assembler::new();
                    let obj_bytes = assembler.assemble(&full_asm);
                    let exe_bytes = link(&[obj_bytes], opts.target);
                    let outfile = opts.output.as_deref().unwrap_or("a.out");
                    if let Err(e) = std::fs::write(outfile, &exe_bytes) {
                        eprintln!("cc1: error: cannot write to '{}': {}", outfile, e);
                        return 1;
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    if diag.has_warnings() {
        diag.emit_all(&source_map);
    }

    0
}

fn main() {
    process::exit(run());
}
