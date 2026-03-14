# cc1 — Complete Task Checklist

> Master checklist for building the `cc1` C89 compiler and `fcc` driver in Rust.
> Reference: `docs/txt/en.subject.txt` (v1.00), `docs/txt/prompt.md`, ISO 9899-1990, System V ABI.

---

## Status Legend

- [ ] Not started
- [x] Done
- [~] In progress

---

## Milestone 0 — Foundations (commits 1–15)

### Project Setup
- [x] Initialize Cargo project with `edition = "2021"`
- [x] Add `bumpalo` as only external dependency
- [x] Configure Makefile to build cc1 and fcc
- [x] Set up `.gitignore` for Rust artifacts
- [ ] Configure `build.rs` to detect vendor ft_lex/ft_yacc

### Core Data Structures (DOD)
- [x] Implement `Target` enum (`I386`, `X86_64`) with ABI tables
  - [x] `ptr_size()`, `long_size()`, `double_align()`, `long_double_size()`
  - [x] `triple()`, `datalayout()`
  - [x] Full scalar type size/align table per System V ABI
- [x] Implement `Span` type `{ file: FileId, lo: u32, hi: u32 }`
- [x] Implement `FileId(u32)` handle type
- [x] Implement `SourceMap`: file loading, `FileId` → name/content, span→line:col
- [x] Implement string interning arena (`InternId`)
- [x] Implement `NodeId(u32)`, `TypeId(u32)`, `ScopeId(u32)` handle types
- [x] Implement central `Ctx` struct holding all flat `Vec<T>` arenas

### Diagnostics
- [x] Implement `DiagEngine` with `error()`, `warning()`, `note()`, `fatal()`
- [x] Format errors as `file:line:col: error: message`
- [x] Support "previous definition was here" notes with secondary spans
- [x] Ensure no panics — all error paths go through DiagEngine

### CLI & Driver Skeleton
- [x] Implement `cc1 infile [-o outfile] [--target i386|x86_64] [-m32] [-g]`
- [x] Implement `--dump-ast`, `--dump-tokens`, `--dump-types` debug flags
- [x] Create `fcc` shell script skeleton (placeholder pipeline)

### CI & Testing Infrastructure
- [ ] Create golden-file test harness for `.c` → `.ll` comparison
- [x] Create unit test framework for each module
- [x] Write `ARCHITECTURE.md` explaining DOD and handle-based design

---

## Milestone 1 — Lexer / Translation Phases 1–3 (commits 16–44)

### Phase 1 — Physical Source Reading
- [x] Read source file as bytes, validate UTF-8
- [x] Map physical source characters to source character set
- [x] Implement trigraph replacement (`??=` → `#`, all 9 trigraphs per §2.2.1.1)

### Phase 2 — Line Splicing
- [x] Delete backslash-newline sequences (join physical lines)
- [x] Handle file not ending in newline (append one)

### Phase 3 — Tokenization
- [x] Define `TokenKind` enum with all C89 tokens:
  - [x] 32 keywords: `auto`, `break`, `case`, `char`, `const`, `continue`, `default`, `do`, `double`, `else`, `enum`, `extern`, `float`, `for`, `goto`, `if`, `int`, `long`, `register`, `return`, `short`, `signed`, `sizeof`, `static`, `struct`, `switch`, `typedef`, `union`, `unsigned`, `void`, `volatile`, `while`
  - [x] Identifiers
  - [x] Integer constants (decimal, octal `0...`, hex `0x...`)
  - [x] Integer suffixes: `u/U`, `l/L`, `ul/UL`, `ull/ULL`
  - [x] Floating constants with optional exponent, `f/F/l/L` suffix
  - [x] Character constants: `'a'`, `'\n'`, `'\x41'`, `'\101'`
  - [x] String literals with escape sequences per §3.1.4
  - [x] All operators and punctuators per §3.1.5
- [x] Implement maximal-munch rule for multi-char operators
- [x] Implement comment stripping (`/* ... */` only, no `//` in C89)
- [x] Attach `Span` to every token
- [x] Implement identifier interning via string arena
- [x] Implement token iterator with peek/put-back buffer
- [x] Implement string literal concatenation (adjacent strings → one)
- [x] Handle escape sequences: `\a`, `\b`, `\f`, `\n`, `\r`, `\t`, `\v`, `\\`, `\'`, `\"`, `\?`, `\0`, `\ooo`, `\xhh`

### Lexer Tests (target: ~100 tests)
- [x] Lex every operator in C89
- [x] Test trigraph edge cases
- [x] Test maximal munch: `x+++++y` → `x ++ ++ + y`
- [ ] Test integer overflow in constants → diagnostic
- [x] Test all escape sequences in string/char literals
- [x] Test comment edge cases (nested not allowed)
- [ ] Golden-file tests for all token kinds
- [ ] Performance: ≥ 5 MB/s on 10 KLOC file

---

## Milestone 2 — Preprocessor / Bonus: Metaprogramming (commits 45–72)

### Macro Engine
- [ ] Object-like macro `#define` and expansion
- [ ] Function-like macros: argument isolation, nested parens
- [ ] `#` stringification operator
- [ ] `##` token-pasting operator
- [ ] Rescan loop with blue-paint recursion guard
- [ ] `#undef` directive
- [ ] Multi-line macros (backslash continuation)

### File Inclusion
- [ ] `#include <system>` — search system paths
- [ ] `#include "local"` — search local then system paths
- [ ] Recursive `#include` (phases 1–4 on included file)
- [ ] Include depth limit (200) to prevent infinite recursion

### Conditional Compilation
- [ ] `#if` / `#ifdef` / `#ifndef` / `#elif` / `#else` / `#endif`
- [ ] `defined()` operator in `#if` expressions
- [ ] Target-aware constant expression evaluator for `#if`

### Predefined Macros & Directives
- [ ] `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`, `__STDC__`
- [ ] `#error` directive
- [ ] `#pragma` (ignore unknown gracefully)
- [ ] `#line` directive

### Preprocessor Tests (target: ~80 tests)
- [ ] Include guards (`#ifndef HEADER_H`)
- [ ] Recursive macro expansion depth limit
- [ ] Stringification edge cases
- [ ] `##` pasting that creates invalid tokens → diagnostic
- [ ] `#if` with arithmetic: precedence, shifts, comparisons
- [ ] Compare output to `clang -E -std=c89` on real headers

---

## Milestone 3 — Parser & AST (commits 73–118)

### AST Node Definitions (flat, handle-based)
- [x] `NodeId`-indexed `ExprNode` variants: `Literal`, `Ident`, `BinOp`, `UnaryOp`, `Call`, `Cast`, `SizeofType`, `SizeofExpr`, `MemberAccess`, `ArraySubscript`, `Deref`, `AddrOf`, `Ternary`, `Comma`, `Assign`, `CompoundAssign`, `PreIncDec`, `PostIncDec`
- [x] `StmtNode` variants: `Compound`, `If`, `While`, `DoWhile`, `For`, `Return`, `Break`, `Continue`, `Switch`, `Case`, `Default`, `Goto`, `Label`, `ExprStmt`, `Null`
- [x] `DeclNode` variants: `FuncDef`, `VarDecl`, `TypedefDecl`, `StructDecl`, `UnionDecl`, `EnumDecl`, `ParamDecl`
- [x] `NodeArena`: push/get by `NodeId`

### Hand-Written Recursive Descent Parser
- [x] Translation-unit: sequence of external declarations
- [x] Function definitions: declarator + compound statement
- [x] Declarations: specifiers, declarators, initializers
- [x] Storage-class specifiers: `auto`, `register`, `static`, `extern`, `typedef`
- [x] Type specifiers: `void`, `char`, `short`, `int`, `long`, `float`, `double`, `signed`, `unsigned`
- [x] Type qualifiers: `const`, `volatile`
- [x] Struct/union specifiers with member declarations
- [x] Enum specifiers with enumerator lists
- [x] Abstract declarators (for casts and `sizeof`)
- [x] Pointer declarators (`*`, `const`, `volatile`)
- [x] Function declarators (parameters, `...` ellipsis)
- [x] Array declarators (constant size, `[]` empty)
- [ ] K&R-style function definitions (old-style parameters)

### Statement Parsing
- [x] Compound statements `{ declaration* statement* }`
- [x] `if`/`else` with dangling-else resolution
- [x] `while`, `do-while`, `for` loops
- [x] `switch`/`case`/`default`/`break`/`continue`
- [x] `goto` and labels
- [x] `return` statement
- [x] Expression statements

### Expression Parsing (all 15 precedence levels)
- [x] Comma `,`
- [x] Assignment `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `<<=`, `>>=`, `&=`, `^=`, `|=`
- [x] Ternary `?:`
- [x] Logical `||`, `&&`
- [x] Bitwise `|`, `^`, `&`
- [x] Equality `==`, `!=`
- [x] Relational `<`, `>`, `<=`, `>=`
- [x] Shift `<<`, `>>`
- [x] Additive `+`, `-`
- [x] Multiplicative `*`, `/`, `%`
- [x] Unary: `&`, `*`, `-`, `+`, `~`, `!`, `++`, `--` (prefix)
- [x] Postfix: `++`, `--`, `[]`, `->`, `.`, function call
- [x] `sizeof(type)` and `sizeof expr`
- [x] Cast expressions `(type-name) expr`
- [x] Primary: identifier, constant, string, `(expr)`
- [x] Aggregate initializers `{ val, val, ... }`
- [x] Error recovery: skip to next `;` or `}`

### Parser Tests (target: ~120 tests)
- [ ] Golden-file AST dump for 30+ representative C89 programs
- [ ] All grammar edge cases from C89 Annex A
- [x] Error recovery on malformed input (no panic)
- [x] `--dump-ast` flag produces readable tree

---

## Milestone 4 — Semantic Analysis & Type System (commits 119–158)

### Type System
- [x] `TypeId`-indexed `CType` variants: `Void`, `Bool`, `Char`, `SChar`, `UChar`, `Short`, `UShort`, `Int`, `UInt`, `Long`, `ULong`, `LongLong`, `ULongLong`, `Float`, `Double`, `LongDouble`, `Pointer(TypeId)`, `Array(TypeId, Option<u64>)`, `Struct(StructId)`, `Union(UnionId)`, `Enum(EnumId)`, `Function(FuncTypeId)`, `Typedef(TypeId)`
- [x] `TypeArena`: intern and deduplicate types
- [x] `sizeof()` and `alignof()` → target-aware per ABI table
- [x] Integer promotion rules (§3.2.1.1): `char`/`short` → `int`
- [x] Usual arithmetic conversions (§3.2.1.5)

### Scope & Symbol Table
- [x] `ScopeId`-indexed scopes: file, block, function, prototype
- [x] Symbol lookup: walk scope chain to file scope
- [x] Symbol insertion with duplicate detection + diagnostic
- [x] `typedef` resolution
- [ ] Linkage analysis: `static` → internal, default → external

### Struct/Union/Enum
- [x] Forward declarations (incomplete types)
- [x] Struct layout engine: target-aware member offsets, padding, size
  - [ ] Verify i386: `struct { char; double; int }` → offsets 0, 4, 12 size 16
  - [ ] Verify x86_64: `struct { char; double; int }` → offsets 0, 8, 16 size 24
- [x] Union layout: all members at offset 0, size = max
- [x] Enum constant assignment (explicit and auto-increment)
- [ ] Bitfields in structs

### Type Checking
- [x] Assignment compatibility
- [x] Pointer arithmetic: `ptr ± int`, `ptr - ptr`
- [ ] L-value vs r-value classification
- [ ] Modifiable l-value checks (not `const`, not array)
- [x] Function call argument count and type checking
- [x] Variadic function call checking (after `...` any types)
- [x] Implicit `int` return in `main()`
- [x] Implicit function declaration (C89 allows `int f()`)
- [ ] Void pointer implicit conversion rules
- [ ] K&R function param type mapping

### Constant Folding (target-precision)
- [x] All integer ops at target precision (not host)
- [ ] `~(unsigned long)1 % 7` → 2 (i386) / 0 (x86_64)
- [x] `sizeof`, pointer arithmetic, enum values
- [ ] Array size deduction from initializer
- [x] String literal type: `array of char` with null terminator

### Sema Tests (target: ~150 tests)
- [ ] 20+ struct/union layouts verified against `clang -m32`
- [ ] Constant-eval test suite: enum values, static initializers
- [ ] Type-checking error suite: 40+ programs with specific errors
- [ ] L-value error suite
- [ ] Cross-compilation constant fold tests

---

## Milestone 5 — LLVM IR Code Generation (commits 159–230)

### IR Builder
- [x] `LlvmIrBuilder`: write IR as text to `BufWriter`
- [x] Emit `target datalayout` and `target triple` based on Target
- [x] Basic block numbering and SSA value naming (`%0`, `%1`, ...)
- [x] C type → LLVM type mapping (`i1`, `i8`, `i16`, `i32`, `i64`, `float`, `double`, `ptr`)

### Global Declarations
- [x] Global variable declarations with correct LLVM types
- [x] Private global string constants as `[N x i8]` arrays
- [x] `extern` function declarations (`declare`)
- [ ] Static local variables as globals with mangled name

### Function Code Generation
- [x] Function definitions: `define ret @name(params) { ... }`
- [x] Parameters as `alloca` + `store` in entry block
- [x] Local variable declarations as `alloca`
- [x] Load/store for variable reads and assignments

### Expression Code Generation
- [x] Arithmetic: `add`, `sub`, `mul`, `sdiv`/`udiv`, `srem`/`urem`
- [x] Bitwise: `and`, `or`, `xor`, `shl`, `lshr`/`ashr`
- [x] Comparisons: `icmp` eq/ne/slt/sgt/sle/sge/ult/ugt/ule/uge
- [x] Float arithmetic: `fadd`, `fsub`, `fmul`, `fdiv`
- [x] Float comparisons: `fcmp`
- [x] Logical `&&`, `||` with short-circuit via branch + phi
- [x] Unary: neg, bitwise not (`~`), logical not
- [x] Type conversions: `zext`, `sext`, `trunc`, `fpext`, `fptrunc`, `fptosi`, `sitofp`, `uitofp`, `fptoui`
- [x] Pointer arithmetic: `getelementptr`
- [ ] Struct member access: `getelementptr` with constant indices
- [x] Array subscript: `getelementptr`
- [x] Dereference (`*ptr`): `load` from ptr
- [x] Address-of (`&lval`): return alloca pointer
- [x] Function calls: `call` with argument list
- [x] Variadic calls: `call` with `...` in declare signature
- [x] Ternary `?:` with branch + phi
- [x] Comma operator: evaluate left for side-effects
- [x] Compound assignment (desugar to load/op/store)
- [x] Prefix/postfix `++`/`--`
- [x] `sizeof` as LLVM constant
- [x] Cast expressions → LLVM conversion ops
- [ ] Aggregate initializers (struct/array)
- [x] String literal → global + `getelementptr`
- [x] Integer promotions during expression codegen

### Statement Code Generation
- [x] `return` (implicit `ret i32 0` for `main`)
- [x] `if`/`else`: `br` to then/else/merge blocks
- [x] `while` loop: `br` to cond/body/after blocks
- [x] `do-while` loop
- [x] `for` loop
- [x] `break`/`continue` via stored block labels
- [ ] `switch`/`case`: LLVM `switch` instruction (currently if-else chain)
- [x] `goto`/labels: `br` to named block

### Codegen Tests (target: ~200 tests)
- [ ] Hello world → IR matches expected output
- [ ] All arithmetic operators → verify with `llvm-as`
- [ ] Struct pass/return → verify ABI
- [x] Pointer arithmetic correctness
- [x] String literal global emission
- [x] Control flow: if/else, loops, switch
- [ ] Verify emitted IR is parseable by `llvm-as` (CI check)

---

## Milestone 6 — Driver & Integration (commits 231–250)

### `fcc` Driver (Shell Script)
- [x] POSIX c17 option parsing: `-c`, `-S`, `-E`, `-o`, `-I`, `-D`, `-U`, `-L`, `-l`, `-s`, `-O`
- [ ] Preprocessing step: `clang -E -std=c89` (or `cc1 --preprocess`)
- [x] Compilation step: `cc1 infile -o outfile.ll`
- [ ] Assembly step: `llc -march=x86 outfile.ll -o outfile.s`
- [ ] Object step: `as --32 outfile.s -o outfile.o`
- [ ] Link step: `clang -m32 outfile.o -o binary`
- [ ] Pass `-I`/`-D`/`-U` to preprocessor
- [ ] Pass `-L`/`-l`/`-s` to linker
- [ ] Pass `-O` to `llc`
- [ ] `-c` flag: stop after `.o`
- [ ] `-S` flag: stop after `.s`
- [ ] `-E` flag: stop after preprocessing
- [ ] Multiple input files
- [ ] Temp-file management and cleanup on error
- [ ] Man-page style documentation

### Integration Tests (target: ~50 tests)
- [x] Compile and run 10+ real C programs end-to-end (14 e2e tests passing)
- [ ] `fcc -v` shows invoked commands
- [ ] All c17 options are accepted

---

## Milestone 7 — Bonus: Cross-Compilation (commits 251–265)

- [x] Default to x86_64 target; `-m32` switches to i386
- [x] Emit x86_64 `datalayout` and `triple`
- [x] Struct layouts switch correctly per target
- [x] `sizeof(long)` = 8 on x86_64, 4 on i386
- [ ] Constant folding uses 64-bit arithmetic for x86_64
- [ ] `enum e { A = ~(unsigned long)1 % 7 }` → A=0 (x86_64), A=2 (i386)
- [ ] `fcc -m32` passes correctly through pipeline
- [ ] 20 cross-compilation test programs

---

## Milestone 8 — Bonus: Debugging / DWARF (commits 266–280)

- [ ] `-g` flag in `cc1` CLI
- [ ] Emit `!DICompileUnit` (DW_LANG_C89)
- [ ] Emit `!DIFile` for each source file
- [ ] Emit `!DISubprogram` for each function
- [ ] Emit `!DILocalVariable` for each local variable
- [ ] Emit `!DILocation` on every instruction when `-g` active
- [ ] `!DIBasicType` for all scalar C types
- [ ] `!DIDerivedType` for pointers, typedefs, const/volatile
- [ ] `!DICompositeType` for structs, unions, arrays, enums
- [ ] Handle forward declaration stubs for recursive types
- [ ] gdb can set breakpoints by line
- [ ] gdb can print local variable values
- [ ] gdb shows correct file:line in backtraces

---

## Milestone 9 — SSA Intermediate Representation

### IR Types & Operands
- [ ] Define `IrType` enum: `I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32`, `U64`, `F32`, `F64`, `F128`, `Ptr`, `Void`, `I128`, `U128`, `Struct(Vec<IrType>)`, `Array(Box<IrType>, u64)`
- [ ] Define `Operand` enum: `Value(ValueId)`, `Const(ConstValue)`, `Global(String)`, `Label(BlockId)`
- [ ] Define `ConstValue`: `I8(i8)`, `I16(i16)`, `I32(i32)`, `I64(i64)`, `F32(f32)`, `F64(f64)`, `NullPtr`, `Undef`, `ZeroInit`
- [ ] Implement type size/align for `IrType` (target-aware)
- [ ] Implement `IrType::is_integer`, `is_float`, `is_pointer`, `bit_width`

### IR Module & Function
- [ ] Define `IrModule`: globals, functions, string literals, extern declarations, type definitions
- [ ] Define `IrFunction`: name, return type, params, basic blocks, value counter, linkage, visibility
- [ ] Define `BasicBlock`: label, instructions `Vec<Instruction>`, terminator
- [ ] Define `ValueId(u32)` handle for SSA values
- [ ] Define `BlockId(u32)` handle for basic blocks
- [ ] Define `GlobalInit` enum: integer, float, string, address, compound, zero-fill, label-diff

### SSA Instructions
- [ ] `Alloca { result, ty, align }` — stack allocation
- [ ] `DynAlloca { result, ty, count }` — VLA
- [ ] `Store { addr, value, ty }` / `Load { result, addr, ty }`
- [ ] `BinOp { result, op, lhs, rhs, ty }` — all arithmetic, bitwise, shift
- [ ] `UnaryOp { result, op, operand, ty }` — neg, bitnot, lognot
- [ ] `Cmp { result, pred, lhs, rhs, ty }` — icmp/fcmp
- [ ] `Cast { result, kind, src, src_ty, dst_ty }` — zext, sext, trunc, fp casts, ptr casts
- [ ] `Call { result, callee, args, ret_ty, is_variadic }` / `CallIndirect`
- [ ] `GetElementPtr { result, base, offset, elem_ty }` — GEP with typed offset
- [ ] `GlobalAddr { result, name }` — materialize global address
- [ ] `Select { result, cond, true_val, false_val, ty }` — conditional move
- [ ] `Copy { result, src }` — SSA copy (from phi elimination)
- [ ] `Phi { result, incoming: Vec<(BlockId, Operand)> }` — phi node
- [ ] `AtomicLoad`, `AtomicStore`, `AtomicRmw`, `AtomicCmpxchg`
- [ ] `StackRestore`, `InlineAsm`

### SSA Terminators
- [ ] `Ret { value: Option<Operand> }`
- [ ] `Br { target: BlockId }`
- [ ] `CondBr { cond, true_bb, false_bb }`
- [ ] `Switch { discr, default, cases: Vec<(i64, BlockId)> }`
- [ ] `IndirectBr { addr, targets }`
- [ ] `Unreachable`

### AST-to-IR Lowering
- [ ] Lower `TranslationUnit` → `IrModule`
- [ ] Lower `FuncDef` → `IrFunction` with entry block
- [ ] Lower declarations → `Alloca` + optional `Store`
- [ ] Lower expressions → SSA values (recursive `emit_expr`)
- [ ] Lower control flow → basic blocks with proper terminators
- [ ] Lower `switch/case` → `Switch` terminator or if-else chain
- [ ] Implicit `ret i32 0` for `main()`
- [ ] String literal → global constant + `GlobalAddr`
- [ ] Integer promotions during lowering
- [ ] Phi insertion for ternary `?:` and logical `&&`/`||`

### IR Tests
- [ ] Round-trip: construct IR, dump text, verify structure
- [ ] AST→IR for arithmetic expressions
- [ ] AST→IR for control flow (if/while/for)
- [ ] AST→IR for function calls
- [ ] AST→IR for pointer operations

---

## Milestone 10 — Backend Core Infrastructure

### Target Enum Extension
- [ ] Extend `Target` enum: `I386`, `X86_64`, `AArch64`, `RiscV64`
- [ ] Add `CodegenOptions` struct: `pic`, `function_return_thunk`, `indirect_branch_thunk`, `patchable_function_entry`, `cf_protection_branch`, `no_sse`, `general_regs_only`, `code_model_kernel`, `no_jump_tables`, `no_relax`, `debug_info`, `function_sections`, `data_sections`, `code16gcc`, `regparm`, `omit_frame_pointer`, `emit_cfi`
- [ ] Implement `Target::generate_assembly_with_opts_and_debug` dispatch
- [ ] Implement `AssemblerConfig` and `LinkerConfig` per target
- [ ] Add Cargo features: `gcc_assembler`, `gcc_linker`, `gcc_m16`

### ArchCodegen Trait (`traits.rs`)
- [ ] Define `ArchCodegen` trait with ~185 methods
- [ ] State access: `state()`, `state_ref()`
- [ ] Prologue/epilogue: `emit_prologue`, `emit_epilogue`, `calculate_stack_space`, `aligned_frame_size`
- [ ] Operand handling: `emit_load_operand`, `emit_store_result`, `emit_copy_value`
- [ ] Memory operations: `emit_store`, `emit_load`, `emit_load_with_const_offset`, `emit_store_with_const_offset`, `emit_seg_load`, `emit_seg_store`, `emit_global_load_rip_rel`, `emit_global_store_rip_rel`
- [ ] Arithmetic: `emit_binop`, `emit_unaryop`, `emit_float_binop`
- [ ] Comparisons: `emit_cmp`, `emit_fused_cmp_branch_blocks`
- [ ] Casts: `emit_cast`, `emit_cast_instrs`
- [ ] Control flow: `emit_branch`, `emit_cond_branch_blocks`, `emit_switch`, `emit_indirect_branch`
- [ ] Function calls: `emit_call` (8-phase), `emit_call_compute_stack_space`, `emit_call_f128_pre_convert`, `emit_call_spill_fptr`, `emit_call_stack_args`, `emit_call_sret_setup`, `emit_call_reg_args`, `emit_call_instruction`, `emit_call_cleanup`, `emit_call_store_result`
- [ ] Atomics: `emit_atomic_load`, `emit_atomic_store`, `emit_atomic_rmw`, `emit_atomic_cmpxchg`
- [ ] 128-bit: `emit_i128_binop`, `emit_i128_cmp`, `emit_i128_store_result`
- [ ] Register allocation: `get_phys_reg_for_value`, `emit_reg_to_reg_move`, `emit_acc_to_phys_reg`
- [ ] Implement `delegate_to_impl!` macro
- [ ] Implement ~64 default method impls (`emit_store_default`, `emit_load_default`, `emit_cast_default`, etc.)
- [ ] Free functions: `emit_store_default`, `emit_load_default`, `emit_cast_default`, `emit_unaryop_default`, `emit_return_default`

### CodegenState (`state.rs`)
- [ ] Define `CodegenState`: output buffer, stack slots, register assignments, label counter
- [ ] Define `StackSlot`: offset, size, alignment
- [ ] Define `SlotAddr` enum: `Direct(StackSlot)`, `Indirect(StackSlot)`, `OverAligned(StackSlot, u32)`
- [ ] Implement `resolve_slot_addr` — classify value as alloca/indirect/overaligned
- [ ] Implement `RegCache`: accumulator value tracking, conservative invalidation
- [ ] Track `f128_load_sources`, `f128_direct_slots` for x87 F128, `small_slot_values`

### Generation Driver (`generation.rs`)
- [ ] `generate_module`: pre-size buffer, symbol sets, data sections, function iteration, aliases
- [ ] `generate_function`: linkage, patchable entry, naked function, pre-scan, stack space, prologue, parameter stores
- [ ] `generate_instruction`: dispatch match per IR instruction variant, register cache management
- [ ] `generate_terminator`: dispatch per terminator variant
- [ ] Pre-scan analysis: value use counts, GEP fold map, cmp-branch fusion, GlobalAddr folding
- [ ] Implement `build_gep_fold_map`: constant-offset GEP folding (two-phase: collect + verify)
- [ ] Implement `build_global_addr_map`: symbol name mapping for RIP-relative folding
- [ ] Implement `build_foldable_global_addr_set`: identify GlobalAddr values with only foldable uses
- [ ] Implement compare-and-branch fusion (single-use Cmp + CondBranch)

### Call ABI Classification (`call_abi.rs`)
- [ ] Define `CallArgClass` enum: `IntReg`, `FloatReg`, `I128RegPair`, `F128Reg`, `StructByValReg`, `StructSseReg`, `StructMixedIntSseReg`, `StructMixedSseIntReg`, `StructSplitRegStack`, `LargeStructStack`, `LargeStructByRefReg`, `Stack`, `ZeroSizeSkip`
- [ ] Define `ParamClass` with concrete stack offsets
- [ ] Define `CallAbiConfig` struct: GP/FP reg counts, variadic rules, struct classification rules
- [ ] Implement `classify_args_core` — unified classification for all 4 architectures
- [ ] Implement `classify_call_args` (caller-side) and `classify_params_full` (callee-side)
- [ ] SysV struct classification (`classify_sysv_struct`) for x86-64

### Cast Classification (`cast.rs`)
- [ ] Define `CastKind` enum: ~20 variants (Noop, FloatToSigned, SignedToFloat, IntWiden, IntNarrow, etc.)
- [ ] Implement `classify_cast_with_f128`
- [ ] Define `FloatOp` classification and `classify_float_binop`
- [ ] Define `F128CmpKind` and `f128_cmp_libcall`

### Data Section Emission (`common.rs`)
- [ ] `classify_global` → `GlobalSection` enum: Extern, Custom, Rodata, Tdata, Data, Common, Tbss, Bss
- [ ] `emit_data_sections`: iterate globals, emit section directives
- [ ] `emit_init_data`: recursive GlobalInit emitter (integers, floats, strings, addresses, compounds)
- [ ] `emit_symbol_directives`: linkage (.globl/.local) and visibility (.hidden/.protected)
- [ ] `PtrDirective` type: `.quad` (x86), `.xword` (ARM), `.dword` (RISC-V)
- [ ] GCC assembler/linker invocation fallback

### Inline Assembly Framework (`inline_asm.rs`)
- [ ] Define `InlineAsmEmitter` trait
- [ ] 4-phase pipeline: classify constraints → load inputs → template substitution → store outputs
- [ ] GCC constraint parsing: register, memory, tied, immediate, specific register
- [ ] Template `%0`, `%1`, `%[name]` substitution with GCC modifiers (`%b`, `%w`, `%h`, `%P`, `%c`)
- [ ] Dialect alternatives `{att|intel}`

### F128 Soft-Float Framework (`f128_softfloat.rs`)
- [ ] Define `F128SoftFloat` trait (~48 primitive methods)
- [ ] Shared orchestration: `f128_operand_to_arg1`, `f128_emit_store`, `f128_emit_load`, `f128_emit_cast`, `f128_emit_binop`, `f128_cmp`, `f128_neg`

---

## Milestone 11 — Stack Layout & Register Allocation

### Three-Tier Stack Layout (`stack_layout/`)
- [ ] Tier 1: Alloca slots — permanent, non-shared
- [ ] Escape analysis: `compute_coalescable_allocas` — identify non-escaping single-block allocas, demote to Tier 3
- [ ] Dead alloca detection: skip unused non-parameter allocas
- [ ] Tier 2: Multi-block SSA temporaries — liveness-based interval coloring (min-heap greedy)
- [ ] Tier 3: Single-block values — block-local slot reuse with greedy recycling
- [ ] Copy alias tracking: `Copy` instructions share source slot
- [ ] Immediately-consumed value elimination (accumulator keeps value alive)
- [ ] Deferred slot finalization: block-local slots placed after Tier 1+2
- [ ] `calculate_stack_space_common` driver
- [ ] `analysis.rs`: use counting, immediately-consumed detection, block analysis
- [ ] `alloca_coalescing.rs`: escape analysis
- [ ] `copy_coalescing.rs`: copy alias tracking
- [ ] `slot_assignment.rs`: Tier 2 liveness packing, Tier 3 block-local reuse
- [ ] `inline_asm.rs`: callee-saved register scanning
- [ ] `regalloc_helpers.rs`: register allocation setup

### Liveness Analysis (`liveness.rs`)
- [ ] Program point numbering across all instructions and terminators
- [ ] Backward dataflow with compact bitsets (packed u64 words)
- [ ] Iterative fixed-point computation for loop back-edges
- [ ] Live interval construction from definition/use points and live-through blocks
- [ ] Call point extraction for register allocator
- [ ] Loop nesting depth via DFS back-edge detection
- [ ] Canonical operand iterators: `for_each_operand_in_instruction`, `for_each_value_use_in_instruction`, `for_each_operand_in_terminator`

### Linear Scan Register Allocator (`regalloc.rs`)
- [ ] Phase 1: Callee-saved registers for call-spanning values (x86: rbx, r12–r15)
- [ ] Phase 2: Caller-saved registers for non-call-spanning values (x86: r11, r10, r8, r9)
- [ ] Phase 3: Callee-saved spillover for remaining values
- [ ] Priority scoring: live range length + use count with loop-depth weighting (10^D)
- [ ] Eligibility filtering: whitelist of simple instruction results
- [ ] Exclusion: allocas, floats, F128, i128, single-use-immediately-consumed
- [ ] Integration: run during `calculate_stack_space`, cache liveness for Tier 2

---

## Milestone 12 — x86-64 Backend (SysV AMD64 ABI)

### x86-64 Code Generation (`x86/codegen/`)
- [ ] `emit.rs`: `X86Codegen` struct, `ArchCodegen` impl, `delegate_to_impl!`
- [ ] `alu.rs`: integer arithmetic (add, sub, imul, idiv, and, or, xor, shl, shr, sar)
- [ ] `comparison.rs`: cmp, fused cmp-branch, set{cc} for boolean results
- [ ] `float_ops.rs`: SSE2 float arithmetic (addss, subss, mulss, divss, addsd, etc.)
- [ ] `memory.rs`: load (movq/movl/movw/movb), store, memcpy, GEP with constant folding
- [ ] `calls.rs`: SysV AMD64 calling convention (rdi, rsi, rdx, rcx, r8, r9 + xmm0..7)
- [ ] `cast_ops.rs`: movzx, movsx, cvtsi2ss, cvtss2sd, cvttss2si, etc.
- [ ] `globals.rs`: RIP-relative addressing (`leaq symbol(%rip), %rax`), TLS access (FS segment)
- [ ] `prologue.rs`: push rbp, mov rsp rbp, sub rsp frame_size, callee-save push/pop, .cfi directives
- [ ] `returns.rs`: ret value in rax/xmm0, void returns, struct returns via sret
- [ ] `variadic.rs`: va_start (register save area), va_arg, va_copy
- [ ] `intrinsics.rs`: popcount, bswap, clz, ctz, overflow builtins
- [ ] `i128_ops.rs`: 128-bit add/sub/mul/div using rax:rdx pairs
- [ ] `f128.rs`: x87 fldt/fstpt/faddp/fmulp for long double
- [ ] `atomics.rs`: lock-prefixed instructions, cmpxchg, mfence
- [ ] `inline_asm.rs`: x86 InlineAsmEmitter implementation
- [ ] Switch compilation: jump table vs compare-and-branch (density heuristic: min 4 cases, max 4096 range, 40% density)

### x86-64 Peephole Optimizer (`x86/codegen/peephole/`)
- [ ] Line classification: `LineInfo` struct (kind, dest reg, stack offset, extension, reg bitmask)
- [ ] NOP marking strategy (preserve indices, final compaction)
- [ ] Phase 1 — Local passes (8 rounds): store/load elimination, redundant jump, self-move, redundant cltq, push/pop
- [ ] Phase 2 — Global passes: store forwarding, register copy propagation, dead store elimination, cmp-branch fusion, memory operand folding
- [ ] Phase 3 — Post-global cleanup (4 rounds)
- [ ] Phase 4 — Loop trampoline elimination
- [ ] Phase 5 — Tail call optimization + never-read store elimination
- [ ] Phase 6 — Unused callee-save elimination
- [ ] Phase 7 — Frame compaction
- [ ] Shared utilities: `peephole_common.rs` (word matching, register replacement, LineStore)

### x86-64 Assembler (`x86/assembler/`)
- [ ] `parser.rs`: AT&T syntax tokenizer and parser → `Vec<AsmStatement>`
- [ ] `encoder/`: REX prefix, ModR/M, SIB, displacement, immediate encoding
  - [ ] GP integer instructions
  - [ ] SSE/SSE2 instructions
  - [ ] x87 instructions
  - [ ] System instructions
  - [ ] Atomic / lock-prefix instructions
- [ ] `elf_writer.rs`: ELFCLASS64/EM_X86_64, .text/.data/.rodata/.bss, symbol table, relocation entries (R_X86_64_*)
- [ ] Shared: `asm_expr.rs` (expression evaluator), `asm_preprocess.rs` (macros, .rept, .if)

### x86-64 Linker (`x86/linker/`)
- [ ] `input.rs`: read ELF64 .o files, parse sections/symbols/relocations
- [ ] `link.rs`: main link driver (CRT discovery, archive resolution, symbol resolution)
- [ ] `types.rs`: linker-internal types (InputSection, OutputSection, resolved symbols)
- [ ] `plt_got.rs`: PLT/GOT construction for dynamic linking
- [ ] `elf.rs`: ELF helpers for section merging
- [ ] `emit_exec.rs`: write ELF64 executable (program headers, dynamic section)
- [ ] `emit_shared.rs`: write shared library output
- [ ] Relocation application: R_X86_64_64, R_X86_64_PC32, R_X86_64_PLT32, R_X86_64_GOTPCREL, R_X86_64_GOTTPOFF, R_X86_64_TPOFF32
- [ ] TLS IE-to-LE relaxation, copy relocations

---

## Milestone 13 — i686 Backend (cdecl, ILP32)

### i686 Code Generation (`i686/codegen/`)
- [ ] `emit.rs`: `I686Codegen` struct, cdecl calling convention (all args on stack)
- [ ] `alu.rs`: 32-bit integer arithmetic
- [ ] `comparison.rs`, `casts.rs`, `float_ops.rs` (x87)
- [ ] `memory.rs`: load/store with 32-bit addresses
- [ ] `calls.rs`: cdecl (push args right-to-left), -mregparm=N support
- [ ] `prologue.rs`: push ebp, mov esp ebp, sub esp frame_size
- [ ] `i128_ops.rs`: 128-bit via eax:edx:ecx:ebx register quad
- [ ] `variadic.rs`, `returns.rs`, `globals.rs`, `atomics.rs`, `inline_asm.rs`, `intrinsics.rs`

### i686 Peephole Optimizer
- [ ] Four-phase structure (8/1/4 rounds + never-read store elimination)

### i686 Assembler
- [ ] Reuse x86 AT&T parser, 32-bit encoder (no REX)
- [ ] ELFCLASS32 / EM_386 / Elf32_Sym / Elf32_Rel

### i686 Linker
- [ ] 32-bit ELF, R_386_32, R_386_PC32, R_386_PLT32, R_386_GOTPC, R_386_GOTOFF, R_386_GOT32X
- [ ] .rel sections (no addend, unlike .rela)

---

## Milestone 14 — AArch64 Backend (AAPCS64)

### AArch64 Code Generation (`arm/codegen/`)
- [ ] `emit.rs`: `ArmCodegen` struct, AAPCS64 (x0..x7 GP + v0..v7 FP)
- [ ] `alu.rs`: add, sub, mul, sdiv, and, orr, eor, lsl, lsr, asr
- [ ] `comparison.rs`: cmp + b.{cc}, cset
- [ ] `float_ops.rs`: NEON/FP fadd, fsub, fmul, fdiv
- [ ] `memory.rs`: ldr/str with scaled/unscaled offset, ldp/stp
- [ ] `calls.rs`: AAPCS64 (sret via x8, large structs by reference)
- [ ] `prologue.rs`: stp x29,x30,[sp,#-N]!, mov x29,sp, callee-save stp pairs
- [ ] `f128.rs`: IEEE binary128 soft-float via compiler-rt (__addtf3, etc.), Q-register
- [ ] `variadic.rs`: va_start (register dump), va_arg
- [ ] `globals.rs`: ADRP + ADD for PC-relative addressing

### AArch64 Peephole Optimizer
- [ ] Three-phase (8/1/4): store/load elimination, self-move (64-bit only), branch-over-branch fusion

### AArch64 Assembler
- [ ] ARM assembly syntax parser
- [ ] Fixed 32-bit instruction encoding, imm12 auto-shift
- [ ] ELFCLASS64 / EM_AARCH64

### AArch64 Linker
- [ ] ADR_PREL_PG_HI21, ADD_ABS_LO12_NC, CALL26, JUMP26, LDST*, ADR_GOT_PAGE
- [ ] PLT/GOT, IFUNC/IPLT, GLOB_DAT, copy relocations, TLS

---

## Milestone 15 — RISC-V 64 Backend (LP64D)

### RISC-V Code Generation (`riscv/codegen/`)
- [ ] `emit.rs`: `RiscVCodegen` struct, LP64D (a0..a7 GP + fa0..fa7 FP)
- [ ] `alu.rs`: add, sub, mul, div, rem, and, or, xor, sll, srl, sra
- [ ] `comparison.rs`: beq/bne/blt/bge/bltu/bgeu
- [ ] `float_ops.rs`: fadd.s/fadd.d, fsub, fmul, fdiv
- [ ] `memory.rs`: ld/sd/lw/sw with 12-bit signed offset
- [ ] `calls.rs`: LP64D (even-aligned i128 register pairs)
- [ ] `prologue.rs`: addi sp,sp,-N; sd ra/s0; addi s0,sp,N
- [ ] `f128.rs`: IEEE binary128 soft-float, GP pair (a0:a1)
- [ ] `globals.rs`: lui + addi (medlow), auipc + addi (medany)

### RISC-V Peephole Optimizer
- [ ] Three-phase (8/1/4) adapted to RV instruction set

### RISC-V Assembler
- [ ] RV assembly parser
- [ ] 32-bit instruction encoding + RV64C compression (16-bit compact form)
- [ ] ELFCLASS64 / EM_RISCV

### RISC-V Linker
- [ ] R_RISCV_HI20, LO12_I, LO12_S, CALL, PCREL_HI20, GOT_HI20, BRANCH
- [ ] Linker relaxation markers

---

## Milestone 16 — Shared ELF Infrastructure

### ELF Module (`elf/`)
- [ ] `mod.rs`: core ELF types (Elf64_Ehdr, Elf64_Shdr, Elf64_Phdr, Elf64_Sym, Elf64_Rela, Elf32 variants)
- [ ] `constants.rs`: ELF format constants (ET_*, EM_*, SHT_*, SHF_*, STB_*, STT_*, R_X86_64_*, R_386_*, R_AARCH64_*, R_RISCV_*)
- [ ] `string_table.rs`: StringTable builder for .strtab/.shstrtab/.dynstr
- [ ] `section_flags.rs`: parse section flags from GAS directives (.section .text,"ax")
- [ ] `archive.rs`: AR archive (.a) parsing (regular and thin archives)
- [ ] `io.rs`: binary read/write helpers (LE16/LE32/LE64, BE variants)
- [ ] `parse_string.rs`: GAS string literal parsing with escape sequences
- [ ] `linker_symbols.rs`: linker-generated symbols (_GLOBAL_OFFSET_TABLE_, _DYNAMIC, etc.)
- [ ] `symbol_table.rs`: ELF symbol table emission
- [ ] `numeric_labels.rs`: GAS numeric label (1:, 2f, 3b) support
- [ ] `object_writer.rs`: high-level ELF object file writer
- [ ] `writer_base.rs`: low-level ELF writer (headers, sections, relocations)

### Shared Linker Infrastructure (`linker_common/`)
- [ ] `types.rs`: Elf64Section, Elf64Symbol, Elf64Object, DynSymbol
- [ ] `parse_object.rs`: parse ELF64 relocatable objects (.o)
- [ ] `parse_shared.rs`: extract dynamic symbols and SONAME from .so
- [ ] `symbols.rs`: GlobalSymbolOps trait, InputSection/OutputSection
- [ ] `merge.rs`: merge input sections into output sections, COMMON symbols
- [ ] `dynamic.rs`: match undefined globals against shared library exports
- [ ] `archive.rs`: load archives (.a, thin archives), iterative resolution
- [ ] `resolve_lib.rs`: resolve `-l` library names to filesystem paths
- [ ] `args.rs`: parse `-Wl,` linker flags into structured `LinkerArgs`
- [ ] `check.rs`: post-link undefined symbol validation
- [ ] `section_map.rs`: section ordering and address assignment
- [ ] `write.rs`: shared ELF executable writing helpers
- [ ] `dynstr.rs`: dynamic string table builder
- [ ] `hash.rs`: GNU hash table and SysV hash table generation
- [ ] `eh_frame.rs`: .eh_frame FDE counting and .eh_frame_hdr builder
- [ ] `gc_sections.rs`: `--gc-sections` BFS reachability analysis

---

## Milestone 17 — Integration & End-to-End Pipeline

### Pipeline Wiring
- [ ] `main.rs`: AST → IR lowering → assembly generation → assemble → link
- [ ] Extend `Opts` for new backend flags: `--emit-asm`, `--emit-obj`, `--emit-ir`, `-fPIC`, `-fpatchable-function-entry`, `-fcf-protection`, `-mno-sse`, `-mgeneral-regs-only`, `-mcmodel=kernel`, `-fno-jump-tables`, `-mno-relax`, `-ffunction-sections`, `-fdata-sections`, `-fomit-frame-pointer`, `-f[no-]asynchronous-unwind-tables`
- [ ] `CCC_KEEP_ASM` env var to preserve intermediate .s file
- [ ] Update `fcc` driver to use native backend instead of LLVM
- [ ] Support `-static` flag for static linking

### End-to-End Tests
- [ ] All 14 existing e2e tests pass through native backend
- [ ] Cross-architecture tests: same C program → correct behavior on x86-64 and i686
- [ ] GCC torture test subset (≥200 tests)
- [ ] Compile known C codebase (e.g., zlib) with native backend
- [ ] Verify ABI compliance: struct pass/return interop with GCC-compiled code

### Performance
- [ ] Profile codegen on 10 KLOC file, eliminate bottlenecks
- [ ] Assembly size comparison against GCC -O0 on representative programs
- [ ] Peephole optimizer effectiveness: measure before/after on zlib

---

## Milestone 18 — Hardening & Final Polish

- [ ] Run GCC torture test suite `execute/` subset (≥200 tests)
- [ ] Run GCC torture test suite `compile/` subset
- [ ] Verify all ABI struct layouts against clang (50 structs)
- [ ] Verify all constant-eval results against clang
- [ ] Better error recovery in parser for multi-error reports
- [ ] `--Werror`, `--Wall`, `--Wno-implicit` flags
- [ ] Profile `cc1` on 10 KLOC file; eliminate bottlenecks
- [ ] No `unwrap()` in production paths
- [ ] Compile a known C codebase with `fcc`
- [ ] Final `cargo clippy`, `cargo fmt`, zero warnings
- [ ] Tag `v1.0.0`

---

## C89 Compliance Matrix

### §3.1 Lexical Elements
- [x] Keywords (32)
- [x] Identifiers
- [x] Constants (integer, floating, enumeration, character)
- [x] String literals
- [x] Operators and punctuators
- [ ] Header names

### §3.2 Conversions
- [x] Arithmetic operands
- [ ] Other operands (lvalue, void, pointers)

### §3.3 Expressions (15 precedence levels)
- [x] Primary (§3.3.1)
- [x] Postfix (§3.3.2)
- [x] Unary (§3.3.3)
- [x] Cast (§3.3.4)
- [x] Multiplicative (§3.3.5)
- [x] Additive (§3.3.6)
- [x] Shift (§3.3.7)
- [x] Relational (§3.3.8)
- [x] Equality (§3.3.9)
- [x] Bitwise AND (§3.3.10)
- [x] Bitwise XOR (§3.3.11)
- [x] Bitwise OR (§3.3.12)
- [x] Logical AND (§3.3.13)
- [x] Logical OR (§3.3.14)
- [x] Conditional (§3.3.15)
- [x] Assignment (§3.3.16)
- [x] Comma (§3.3.17)
- [x] Constant expressions (§3.4)

### §3.5 Declarations
- [x] Storage-class specifiers
- [x] Type specifiers
- [x] Type qualifiers
- [x] Declarators
- [x] Type names
- [x] Typedef
- [x] Initialization

### §3.6 Statements
- [x] Labeled
- [x] Compound
- [x] Expression
- [x] Selection (if, switch)
- [x] Iteration (while, do, for)
- [x] Jump (goto, continue, break, return)

### §3.7 External Definitions
- [x] Function definitions
- [x] External object definitions

---

## Key Correctness Invariants

1. `sizeof(long)` == `target.long_size()` — NEVER hardcode
2. `constant_fold(~(unsigned long)1 % 7)` depends on `target.long_size()`
3. Struct layout uses `target.double_align()` for double members
4. All LLVM ptr types are just `ptr` (opaque pointers, LLVM 15+)
5. Variadic calls must match `declare i32 @printf(ptr, ...)` signature
6. `main()` with no explicit return → implicit `ret i32 0`
7. String literals are `private global [N x i8]` — N includes null byte
8. Every `NodeId` reference is a `u32` index — never store `&Node`
9. Diagnostics fire on `Span`, never on line number alone
10. The compiler never panics — all error paths go through `DiagEngine`

---

## Test Count Targets

| Category | Target | Current |
|----------|--------|---------|
| Lexer unit tests | ~100 | 44 |
| Preprocessor tests | ~80 | 0 |
| Parser tests | ~120 | 24 |
| Semantic analysis tests | ~150 | 7 |
| LLVM IR codegen tests | ~200 | 9 |
| SSA IR lowering tests | ~100 | 0 |
| Native backend tests | ~300 | 0 |
| x86-64 assembler tests | ~50 | 0 |
| x86-64 linker tests | ~30 | 0 |
| ELF format tests | ~20 | 0 |
| Peephole optimizer tests | ~50 | 0 |
| Integration tests | ~50 | 14 (e2e) |
| Cross-compilation tests | ~30 | 0 |
| Debug/DWARF tests | ~20 | 0 |
| Torture tests | ~250 | 0 |
| **TOTAL** | **~1550** | **104 unit + 14 e2e** |
