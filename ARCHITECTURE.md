# ARCHITECTURE.md — cc1 Compiler Architecture

## Design Decision: Hand-Written Recursive Descent Parser

We use **ft_lex** (Rust emitter) for lexical analysis via `build.rs` integration, and a
**hand-written recursive descent parser** for syntax analysis. Rationale:

- `ft_yacc` only emits C code; FFI bridging into Rust would be fragile and unidiomatic.
- Production C compilers (GCC, Clang) use hand-written recursive descent — it provides
  superior error recovery and diagnostic quality.
- The subject explicitly permits this: *"you could use a recursive descent parser or
  combinator parsing"*.
- `ft_lex` with `--lang rust` generates `lex.yy.rs` directly, which we integrate cleanly.

## Data-Oriented Design (DOD)

All compiler data lives in **flat `Vec<T>` arenas** inside a central `Ctx` struct.
Cross-references use **typed integer handles** (`NodeId(u32)`, `TypeId(u32)`, `ScopeId(u32)`).

```
┌──────────────────────────────────────────────────────┐
│                     Ctx (Central)                     │
├──────────────┬──────────────┬────────────────────────┤
│ nodes: Vec<Node>   │ types: Vec<CType>   │ scopes: Vec<Scope> │
│ strings: Vec<u8>   │ files: Vec<FileInfo> │ diags: Vec<Diag>   │
└──────────────┴──────────────┴────────────────────────┘
         ▲                ▲               ▲
         │                │               │
     NodeId(u32)      TypeId(u32)     ScopeId(u32)
```

**Why not `Box<dyn Node>` or `Rc<Type>`?**
- Cache locality: contiguous memory, sequential access patterns
- Smaller footprint: 4-byte index vs 8-byte pointer
- No borrow-checker fights: no cyclic Rc/RefCell graphs
- O(1) arena-drop at phase end via `bumpalo`

## Compilation Pipeline

```
Source (.c)
    │
    ▼
┌─────────┐   Phase 1-3: trigraphs, line splicing, tokenization
│  Lexer  │   Output: Vec<Token> with Span attachments
└────┬────┘
     │
     ▼
┌─────────────┐   Phase 4 (bonus): macro expansion, #include, #if
│Preprocessor │   Output: filtered/expanded token stream
└─────┬───────┘
      │
      ▼
┌─────────┐   Recursive descent, builds flat AST in Ctx
│ Parser  │   Output: NodeId-indexed AST nodes
└────┬────┘
     │
     ▼
┌─────────┐   Type checking, layout, constant folding
│  Sema   │   Output: typed AST with TypeId annotations
└────┬────┘
     │
     ▼
┌─────────┐   AST → LLVM IR text (alloca/load/store pattern)
│ Codegen │   Output: .ll file
└─────────┘
```

## Module Map

```
src/
├── main.rs           cc1 entry point: CLI → pipeline
├── lib.rs            Library root, re-exports
├── target.rs         Target enum, ABI size/align tables
├── diagnostics.rs    Span, DiagEngine, formatted errors
├── source.rs         SourceMap, FileId, string interning
├── ctx.rs            Central Ctx: all flat Vec arenas
├── frontend/
│   ├── mod.rs
│   ├── lexer/
│   │   ├── mod.rs    Lexer state machine
│   │   └── token.rs  TokenKind enum, Token struct
│   ├── preprocessor/
│   │   ├── mod.rs    Preprocessor driver
│   │   ├── macro_engine.rs
│   │   └── conditional.rs
│   ├── parser/
│   │   ├── mod.rs    Recursive descent parser
│   │   ├── ast.rs    Node enum, ExprNode, StmtNode, DeclNode
│   │   ├── expr.rs   Expression parsing
│   │   ├── stmt.rs   Statement parsing
│   │   └── decl.rs   Declaration parsing
│   └── sema/
│       ├── mod.rs    Semantic analysis driver
│       ├── types.rs  CType, TypeArena, type rules
│       ├── scope.rs  ScopeId, symbol tables
│       ├── layout.rs Target-aware struct/union layout
│       └── consteval.rs  Target-precision constant folder
├── backend/
│   ├── mod.rs
│   └── codegen/
│       ├── mod.rs    Codegen driver
│       ├── llvm_ir.rs  IR builder: write LLVM IR text
│       ├── abi.rs    Calling convention classification
│       └── dwarf.rs  Debug metadata emission (bonus)
└── driver/
    └── mod.rs        fcc logic (also shell script)
```

## Handle Types

| Handle | Arena in Ctx | Stored Type |
|--------|-------------|-------------|
| `NodeId(u32)` | `nodes: Vec<Node>` | AST expression/statement/declaration |
| `TypeId(u32)` | `types: Vec<CType>` | All C type descriptors |
| `ScopeId(u32)` | `scopes: Vec<Scope>` | Scope chains with symbol tables |
| `FileId(u32)` | `files: Vec<FileInfo>` | Source file metadata |
| `InternId(u32)` | `strings: Vec<u8>` | Interned identifier/string data |
| `StringId(u32)` | `string_lits: Vec<StringLit>` | String literal constants |

## Target Abstraction

The `Target` enum flows through every phase. No hardcoded sizes anywhere.

```rust
pub enum Target { I386, X86_64 }

// sizeof(long) = 4 on I386, 8 on X86_64
// struct { char; double; int } = 16 on I386, 24 on X86_64
// ~(unsigned long)1 % 7 = 2 on I386, 0 on X86_64
```
