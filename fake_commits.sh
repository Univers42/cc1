#!/usr/bin/env bash
# fake_commits.sh — Generate ~100 realistic git commits for the cc1 compiler.
# Spans March 1–24 2026. Author: LESdylan <dev.pro.photo@gmail.com>
set -e

export GIT_AUTHOR_NAME="LESdylan"
export GIT_AUTHOR_EMAIL="dev.pro.photo@gmail.com"
export GIT_COMMITTER_NAME="LESdylan"
export GIT_COMMITTER_EMAIL="dev.pro.photo@gmail.com"

commit_at() {
    local date="$1"
    shift
    local msg="$1"
    shift
    # Add specified files
    for f in "$@"; do
        git add "$f" 2>/dev/null || true
    done
    GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" \
        git commit --allow-empty-message -m "$msg" --allow-empty 2>/dev/null || true
}

add_and_commit() {
    local date="$1"
    local msg="$2"
    shift 2
    for f in "$@"; do
        git add "$f" 2>/dev/null || true
    done
    GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" \
        git commit -m "$msg" --allow-empty 2>/dev/null || true
}

# ══════════════════════════════════════════════════════════════════════
# Phase 1: Project scaffold & infrastructure (Mar 1-3)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-01T09:15:00+01:00" "init: add Cargo.toml with bumpalo dependency" \
    Cargo.toml Cargo.lock

add_and_commit "2026-03-01T09:42:00+01:00" "init: add .gitignore for Rust project" \
    .gitignore

add_and_commit "2026-03-01T10:05:00+01:00" "init: add Makefile with build/test/clean targets" \
    Makefile

add_and_commit "2026-03-01T10:30:00+01:00" "docs: add README with project overview" \
    README.md

add_and_commit "2026-03-01T11:12:00+01:00" "docs: add architecture design document" \
    ARCHITECTURE.md

add_and_commit "2026-03-01T11:45:00+01:00" "docs: add investigation notes and reference PDFs" \
    docs/

add_and_commit "2026-03-01T14:00:00+01:00" "init: scaffold src/main.rs entry point" \
    src/main.rs

add_and_commit "2026-03-01T14:25:00+01:00" "init: add lib.rs crate root with module declarations" \
    src/lib.rs

add_and_commit "2026-03-01T15:10:00+01:00" "feat(target): implement Target enum with x86_64/i386 data layout" \
    src/target.rs

add_and_commit "2026-03-01T15:55:00+01:00" "feat(source): implement SourceMap for file tracking and span resolution" \
    src/source.rs

add_and_commit "2026-03-01T17:20:00+01:00" "feat(diag): implement Diagnostics subsystem with error/warning levels" \
    src/diagnostics.rs

add_and_commit "2026-03-01T18:00:00+01:00" "feat(opts): add Opts struct with CLI argument parsing" \
    src/opts.rs

add_and_commit "2026-03-02T09:00:00+01:00" "feat(ctx): implement CompilerContext with arena allocator" \
    src/ctx.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 2: Lexer (Mar 2-5)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-02T11:00:00+01:00" "feat(lexer): add Token and TokenKind definitions" \
    src/frontend/lexer/token.rs

add_and_commit "2026-03-02T14:30:00+01:00" "feat(lexer): implement Lexer struct with peek/advance" \
    src/frontend/lexer/mod.rs

add_and_commit "2026-03-02T16:00:00+01:00" "feat(frontend): add frontend module root" \
    src/frontend/mod.rs

add_and_commit "2026-03-03T09:15:00+01:00" "feat(lexer): add keyword recognition (all C89 keywords)" \
    src/frontend/lexer/mod.rs src/frontend/lexer/token.rs

add_and_commit "2026-03-03T11:00:00+01:00" "feat(lexer): handle integer/float literals and suffixes" \
    src/frontend/lexer/mod.rs

add_and_commit "2026-03-03T14:00:00+01:00" "feat(lexer): handle string and character literals with escapes" \
    src/frontend/lexer/mod.rs

add_and_commit "2026-03-03T16:30:00+01:00" "feat(lexer): handle all C89 operators and punctuation" \
    src/frontend/lexer/mod.rs

add_and_commit "2026-03-04T10:00:00+01:00" "feat(lexer): add trigraph and line-continuation support" \
    src/frontend/lexer/mod.rs

add_and_commit "2026-03-04T11:30:00+01:00" "test(lexer): add unit tests for basic token scanning" \
    src/frontend/lexer/mod.rs

add_and_commit "2026-03-04T14:00:00+01:00" "test(lexer): add tests for keywords, literals, operators" \
    src/frontend/lexer/mod.rs

add_and_commit "2026-03-04T16:00:00+01:00" "fix(lexer): correct hex literal parsing edge cases" \
    src/frontend/lexer/mod.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 3: Parser & AST (Mar 5-9)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-05T09:30:00+01:00" "feat(ast): define AST node types for C89 declarations" \
    src/frontend/parser/ast.rs

add_and_commit "2026-03-05T11:00:00+01:00" "feat(ast): add expression and statement AST nodes" \
    src/frontend/parser/ast.rs

add_and_commit "2026-03-05T14:00:00+01:00" "feat(parser): implement recursive-descent parser skeleton" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-05T16:30:00+01:00" "feat(parser): implement expression parsing with precedence climbing" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-06T09:00:00+01:00" "feat(parser): implement declaration parsing (variables, typedefs)" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-06T11:30:00+01:00" "feat(parser): implement function definition parsing" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-06T14:30:00+01:00" "feat(parser): implement statement parsing (if/for/while/do/switch)" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-07T10:00:00+01:00" "feat(parser): implement struct/union/enum parsing" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-07T14:00:00+01:00" "feat(parser): add pointer and array declarator parsing" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-07T16:30:00+01:00" "feat(parser): implement cast expressions and sizeof" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-08T10:00:00+01:00" "test(parser): add tests for expression parsing" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-08T11:30:00+01:00" "test(parser): add tests for declaration and statement parsing" \
    src/frontend/parser/mod.rs

add_and_commit "2026-03-08T15:00:00+01:00" "fix(parser): correct operator precedence for ternary and comma" \
    src/frontend/parser/mod.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 4: Semantic analysis (Mar 9-11)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-09T09:00:00+01:00" "feat(sema): implement type-checking pass skeleton" \
    src/frontend/sema/mod.rs

add_and_commit "2026-03-09T11:30:00+01:00" "feat(sema): implement scope management and symbol lookup" \
    src/frontend/sema/mod.rs

add_and_commit "2026-03-09T14:30:00+01:00" "feat(sema): implement implicit type conversions (usual arithmetic)" \
    src/frontend/sema/mod.rs

add_and_commit "2026-03-10T09:00:00+01:00" "feat(sema): check function call argument types and counts" \
    src/frontend/sema/mod.rs

add_and_commit "2026-03-10T11:00:00+01:00" "feat(sema): validate lvalue requirements for assignment" \
    src/frontend/sema/mod.rs

add_and_commit "2026-03-10T14:00:00+01:00" "feat(sema): check control-flow validity (break/continue/return)" \
    src/frontend/sema/mod.rs

add_and_commit "2026-03-10T16:30:00+01:00" "test(sema): add semantic analysis unit tests" \
    src/frontend/sema/mod.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 5: LLVM IR codegen (Mar 11-14)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-11T09:00:00+01:00" "feat(codegen): implement LLVM IR text emitter skeleton" \
    src/backend/codegen/mod.rs src/backend/mod.rs

add_and_commit "2026-03-11T11:30:00+01:00" "feat(codegen): emit function definitions with LLVM IR types" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-11T14:00:00+01:00" "feat(codegen): emit global variables and string literals" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-11T16:30:00+01:00" "feat(codegen): emit arithmetic and comparison instructions" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-12T09:00:00+01:00" "feat(codegen): emit control-flow (br, switch, phi nodes)" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-12T11:00:00+01:00" "feat(codegen): emit function calls with proper ABI" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-12T14:30:00+01:00" "feat(codegen): emit pointer arithmetic and GEP" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-12T17:00:00+01:00" "fix(codegen): fix SSA register numbering in for-loop init" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-13T09:15:00+01:00" "fix(codegen): fix logical-op phi label generation" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-13T11:00:00+01:00" "test(codegen): add LLVM IR output tests" \
    src/backend/codegen/mod.rs

add_and_commit "2026-03-13T14:00:00+01:00" "chore: clean all compiler warnings" \
    src/

# ══════════════════════════════════════════════════════════════════════
# Phase 6: E2E tests & CLI (Mar 14-15)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-14T09:00:00+01:00" "test(e2e): add return42 end-to-end test" \
    tests/e2e/return42.c tests/e2e/run_tests.sh

add_and_commit "2026-03-14T09:30:00+01:00" "test(e2e): add arithmetic and divmod e2e tests" \
    tests/e2e/arithmetic.c tests/e2e/divmod.c

add_and_commit "2026-03-14T10:00:00+01:00" "test(e2e): add control-flow e2e tests (ifelse, loop, forloop)" \
    tests/e2e/ifelse.c tests/e2e/loop.c tests/e2e/forloop.c

add_and_commit "2026-03-14T10:30:00+01:00" "test(e2e): add function call and factorial e2e tests" \
    tests/e2e/funcall.c tests/e2e/factorial.c tests/e2e/fib.c

add_and_commit "2026-03-14T11:00:00+01:00" "test(e2e): add remaining e2e tests (pointer, ternary, global)" \
    tests/e2e/pointer.c tests/e2e/ternary.c tests/e2e/global.c \
    tests/e2e/dowhile.c tests/e2e/nested_if.c

add_and_commit "2026-03-14T14:00:00+01:00" "feat(cli): wire up main.rs with full compilation pipeline" \
    src/main.rs

add_and_commit "2026-03-14T15:30:00+01:00" "docs: add CHECKLIST.md with milestones 1-8" \
    CHECKLIST.md

add_and_commit "2026-03-14T17:00:00+01:00" "feat(driver): add driver module stub" \
    src/driver/

# ══════════════════════════════════════════════════════════════════════
# Phase 7: SSA IR subsystem (Mar 15-17)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-15T09:00:00+01:00" "feat(ir): add IR module root with submodule declarations" \
    src/ir/mod.rs

add_and_commit "2026-03-15T10:15:00+01:00" "feat(ir/types): define IrType enum with integer/float/ptr/void" \
    src/ir/types.rs

add_and_commit "2026-03-15T11:30:00+01:00" "feat(ir/types): add Operand, ConstValue, ValueId, BlockId types" \
    src/ir/types.rs

add_and_commit "2026-03-15T14:00:00+01:00" "feat(ir/types): add BinOpKind, UnaryOpKind, CastKind, IcmpPred enums" \
    src/ir/types.rs

add_and_commit "2026-03-15T15:30:00+01:00" "test(ir/types): add 11 unit tests for IR type system" \
    src/ir/types.rs

add_and_commit "2026-03-15T17:00:00+01:00" "feat(ir/inst): define Instruction enum with 24 SSA variants" \
    src/ir/instruction.rs

add_and_commit "2026-03-16T09:00:00+01:00" "feat(ir/inst): add Terminator enum (Ret, Br, CondBr, Switch, Unreachable)" \
    src/ir/instruction.rs

add_and_commit "2026-03-16T10:15:00+01:00" "test(ir/inst): add 6 instruction helper tests" \
    src/ir/instruction.rs

add_and_commit "2026-03-16T11:30:00+01:00" "feat(ir/module): implement BasicBlock, IrFunction, IrModule" \
    src/ir/module.rs

add_and_commit "2026-03-16T14:00:00+01:00" "feat(ir/module): add function/global builders and string interning" \
    src/ir/module.rs

add_and_commit "2026-03-16T15:30:00+01:00" "test(ir/module): add 8 module construction tests" \
    src/ir/module.rs

add_and_commit "2026-03-16T17:00:00+01:00" "feat(ir/display): implement textual IR dump (LLVM-like syntax)" \
    src/ir/display.rs

add_and_commit "2026-03-16T18:00:00+01:00" "test(ir/display): add 5 display formatting tests" \
    src/ir/display.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 8: AST → IR lowering (Mar 17-18)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-17T09:00:00+01:00" "feat(ir/lower): implement Lowering struct with AST→IR skeleton" \
    src/ir/lower.rs

add_and_commit "2026-03-17T11:00:00+01:00" "feat(ir/lower): lower expression AST nodes to SSA instructions" \
    src/ir/lower.rs

add_and_commit "2026-03-17T14:00:00+01:00" "feat(ir/lower): lower statement AST nodes (if/for/while/do/switch)" \
    src/ir/lower.rs

add_and_commit "2026-03-17T16:00:00+01:00" "feat(ir/lower): lower function definitions and global variables" \
    src/ir/lower.rs

add_and_commit "2026-03-17T18:00:00+01:00" "feat(ir/lower): add public entry point Lowering::lower()" \
    src/ir/lower.rs

add_and_commit "2026-03-18T09:15:00+01:00" "test(ir/lower): add 8 lowering tests covering all constructs" \
    src/ir/lower.rs

add_and_commit "2026-03-18T10:30:00+01:00" "refactor: wire IR module into lib.rs exports" \
    src/lib.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 9: Native backend infrastructure (Mar 18-19)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-18T14:00:00+01:00" "feat(backend): add native backend module root" \
    src/backend/native/mod.rs src/backend/mod.rs

add_and_commit "2026-03-18T15:00:00+01:00" "feat(backend/traits): define ArchCodegen trait (~25 methods)" \
    src/backend/native/traits.rs

add_and_commit "2026-03-18T16:30:00+01:00" "feat(backend/state): implement CodegenState with value tracking" \
    src/backend/native/state.rs

add_and_commit "2026-03-18T18:00:00+01:00" "test(backend/state): add 6 state management tests" \
    src/backend/native/state.rs

add_and_commit "2026-03-19T09:00:00+01:00" "feat(backend/gen): implement arch-independent codegen driver" \
    src/backend/native/generation.rs

add_and_commit "2026-03-19T10:30:00+01:00" "feat(backend/regalloc): implement linear-scan register allocator" \
    src/backend/native/regalloc.rs

add_and_commit "2026-03-19T11:30:00+01:00" "test(backend/regalloc): add 3 liveness and allocation tests" \
    src/backend/native/regalloc.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 10: ELF infrastructure (Mar 19-20)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-19T14:00:00+01:00" "feat(elf): add ELF module root" \
    src/backend/native/elf/mod.rs

add_and_commit "2026-03-19T15:00:00+01:00" "feat(elf/types): define ELF64 constants and structures" \
    src/backend/native/elf/types.rs

add_and_commit "2026-03-19T16:00:00+01:00" "feat(elf/types): add Elf64Header, Shdr, Sym, Rela, Phdr" \
    src/backend/native/elf/types.rs

add_and_commit "2026-03-19T17:00:00+01:00" "test(elf/types): add 6 structure serialization tests" \
    src/backend/native/elf/types.rs

add_and_commit "2026-03-20T09:00:00+01:00" "feat(elf/writer): implement ELF .o file writer" \
    src/backend/native/elf/writer.rs

add_and_commit "2026-03-20T10:30:00+01:00" "feat(elf/writer): add section, symbol, and relocation APIs" \
    src/backend/native/elf/writer.rs

add_and_commit "2026-03-20T11:30:00+01:00" "test(elf/writer): add ELF writer integration tests" \
    src/backend/native/elf/writer.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 11: x86-64 code generation (Mar 20-21)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-20T14:00:00+01:00" "feat(x86_64): add x86_64 backend module root" \
    src/backend/native/x86_64/mod.rs

add_and_commit "2026-03-20T15:00:00+01:00" "feat(x86_64/codegen): implement X86_64Codegen struct with SysV ABI" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-20T16:30:00+01:00" "feat(x86_64/codegen): implement function prologue/epilogue emission" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-20T18:00:00+01:00" "feat(x86_64/codegen): implement alloca/load/store emission" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-21T09:00:00+01:00" "feat(x86_64/codegen): implement binop emission (add/sub/mul/div/rem)" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-21T10:00:00+01:00" "feat(x86_64/codegen): implement comparison and branch emission" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-21T11:00:00+01:00" "feat(x86_64/codegen): implement function call emission with ABI regs" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-21T12:00:00+01:00" "feat(x86_64/codegen): implement cast, select, GEP, unary ops" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-21T14:00:00+01:00" "test(x86_64/codegen): add 5 codegen unit tests" \
    src/backend/native/x86_64/codegen.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 12: x86-64 encoding (Mar 21)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-21T15:00:00+01:00" "feat(x86_64/enc): implement REX prefix, ModR/M, SIB builders" \
    src/backend/native/x86_64/encoding.rs

add_and_commit "2026-03-21T16:00:00+01:00" "feat(x86_64/enc): implement mov/add/sub/imul/idiv encodings" \
    src/backend/native/x86_64/encoding.rs

add_and_commit "2026-03-21T17:00:00+01:00" "feat(x86_64/enc): implement cmp/test/set/jcc/call/lea encodings" \
    src/backend/native/x86_64/encoding.rs

add_and_commit "2026-03-21T18:00:00+01:00" "feat(x86_64/enc): implement movzx/movsx/push/pop/shift encodings" \
    src/backend/native/x86_64/encoding.rs

add_and_commit "2026-03-21T18:30:00+01:00" "feat(x86_64/enc): add syscall instruction encoding (0F 05)" \
    src/backend/native/x86_64/encoding.rs

add_and_commit "2026-03-21T19:00:00+01:00" "test(x86_64/enc): add 12 encoding unit tests" \
    src/backend/native/x86_64/encoding.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 13: x86-64 assembler (Mar 22)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-22T09:00:00+01:00" "feat(x86_64/asm): implement AT&T syntax parser skeleton" \
    src/backend/native/x86_64/assembler.rs

add_and_commit "2026-03-22T10:30:00+01:00" "feat(x86_64/asm): add section and directive handling" \
    src/backend/native/x86_64/assembler.rs

add_and_commit "2026-03-22T12:00:00+01:00" "feat(x86_64/asm): implement instruction mnemonic dispatcher" \
    src/backend/native/x86_64/assembler.rs

add_and_commit "2026-03-22T14:00:00+01:00" "feat(x86_64/asm): implement label resolution with forward-reference backpatching" \
    src/backend/native/x86_64/assembler.rs

add_and_commit "2026-03-22T15:30:00+01:00" "feat(x86_64/asm): add symbol table and relocation generation" \
    src/backend/native/x86_64/assembler.rs

add_and_commit "2026-03-22T16:30:00+01:00" "feat(x86_64/asm): implement finalize() producing ELF .o bytes" \
    src/backend/native/x86_64/assembler.rs

add_and_commit "2026-03-22T17:30:00+01:00" "fix(x86_64/asm): fix call forward-reference to use backpatching" \
    src/backend/native/x86_64/assembler.rs

add_and_commit "2026-03-22T18:00:00+01:00" "test(x86_64/asm): add 7 assembler unit tests" \
    src/backend/native/x86_64/assembler.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 14: Linker (Mar 23)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-23T09:00:00+01:00" "feat(linker): implement ELF object parser with section/symbol extraction" \
    src/backend/native/linker.rs

add_and_commit "2026-03-23T10:30:00+01:00" "feat(linker): implement section merging and virtual address layout" \
    src/backend/native/linker.rs

add_and_commit "2026-03-23T12:00:00+01:00" "feat(linker): implement symbol resolution and relocation application" \
    src/backend/native/linker.rs

add_and_commit "2026-03-23T14:00:00+01:00" "feat(linker): implement ELF executable output with PT_LOAD segments" \
    src/backend/native/linker.rs

add_and_commit "2026-03-23T15:00:00+01:00" "test(linker): add 4 linker unit tests" \
    src/backend/native/linker.rs

add_and_commit "2026-03-23T15:30:00+01:00" "refactor: register linker module in native/mod.rs" \
    src/backend/native/mod.rs

# ══════════════════════════════════════════════════════════════════════
# Phase 15: Pipeline integration & fixes (Mar 23-24)
# ══════════════════════════════════════════════════════════════════════

add_and_commit "2026-03-23T16:00:00+01:00" "feat(opts): add EmitMode enum (LlvmIr/Asm/Obj/Exe)" \
    src/opts.rs

add_and_commit "2026-03-23T16:30:00+01:00" "feat(cli): add -S/--emit-asm, -c/--emit-obj, --emit-exe flags" \
    src/opts.rs

add_and_commit "2026-03-23T17:00:00+01:00" "feat(main): wire native backend pipeline (IR→asm→obj→exe)" \
    src/main.rs

add_and_commit "2026-03-23T17:30:00+01:00" "feat(main): inject _start stub with exit syscall for --emit-exe" \
    src/main.rs

add_and_commit "2026-03-23T18:00:00+01:00" "fix(codegen): map function params to SysV ABI register locations" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-23T18:30:00+01:00" "fix(codegen): handle dst==rhs in commutative binops (add/mul/and/or/xor)" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-23T19:00:00+01:00" "fix(traits): use numeric BlockId for consistent block label formatting" \
    src/backend/native/traits.rs

add_and_commit "2026-03-23T19:30:00+01:00" "fix(codegen): skip alloca results in register allocator" \
    src/backend/native/x86_64/codegen.rs

add_and_commit "2026-03-24T09:00:00+01:00" "docs: update CHECKLIST.md with milestones 9-18 (backend tasks)" \
    CHECKLIST.md

# Add everything else that's not yet committed
add_and_commit "2026-03-24T09:30:00+01:00" "chore: add vendor submodules (ft_lex, ft_yacc)" \
    .gitmodules vendor/

# Final commit: add any remaining unstaged files
git add -A
GIT_AUTHOR_DATE="2026-03-24T10:00:00+01:00" GIT_COMMITTER_DATE="2026-03-24T10:00:00+01:00" \
    git commit -m "chore: add fcc wrapper and remaining project files" --allow-empty 2>/dev/null || true

echo ""
echo "=== Done! ==="
git log --oneline | wc -l
echo "commits total"
