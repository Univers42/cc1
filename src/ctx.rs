// ctx.rs — Central context holding all flat Vec arenas.
//
// This is the heart of the Data-Oriented Design. All AST nodes, types, scopes,
// and interned strings live here. Cross-references use typed integer handles.

use std::collections::HashMap;
use crate::source::StringInterner;
use crate::target::Target;

// ── Handle Types ──────────────────────────────────────────────────────

/// Handle into the node arena (AST nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

/// Handle into the type arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

/// Handle into the scope arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

/// Handle into the struct/union definition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordId(pub u32);

/// Handle into the enum definition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumId(pub u32);

/// Handle into the function type table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncTypeId(pub u32);

/// Handle into the symbol table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

/// Sentinel value: "no node" (e.g., an optional child).
pub const NODE_NONE: NodeId = NodeId(u32::MAX);

/// Sentinel value: "no type" (not yet resolved).
pub const TYPE_NONE: TypeId = TypeId(u32::MAX);

/// Sentinel: "no scope".
pub const SCOPE_NONE: ScopeId = ScopeId(u32::MAX);

// ── Node Arena ────────────────────────────────────────────────────────

use crate::source::Span;

/// Top-level AST node. Every node carries a span for diagnostics.
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
    /// Type annotation filled in by sema. TYPE_NONE until then.
    pub ty: TypeId,
}

/// Discriminated union of all AST node kinds.
/// Flat: children are referenced by NodeId, not by pointer.
#[derive(Debug, Clone)]
pub enum NodeKind {
    // ── Translation Unit ──
    TranslationUnit { decls: Vec<NodeId> },

    // ── Declarations ──
    FuncDef {
        return_type: NodeId,  // type specifier node
        name: crate::source::InternId,
        params: Vec<NodeId>,
        body: NodeId,         // compound statement
        is_variadic: bool,
        storage_class: StorageClass,
    },
    VarDecl {
        name: crate::source::InternId,
        type_node: NodeId,    // type specifier node
        init: NodeId,         // NODE_NONE if no initializer
        storage_class: StorageClass,
    },
    ParamDecl {
        name: crate::source::InternId,
        type_node: NodeId,
    },
    TypedefDecl {
        name: crate::source::InternId,
        type_node: NodeId,
    },
    StructDecl {
        tag: Option<crate::source::InternId>,
        members: Vec<NodeId>,
    },
    UnionDecl {
        tag: Option<crate::source::InternId>,
        members: Vec<NodeId>,
    },
    EnumDecl {
        tag: Option<crate::source::InternId>,
        enumerators: Vec<(crate::source::InternId, NodeId)>, // name, optional value expr
    },
    MemberDecl {
        name: crate::source::InternId,
        type_node: NodeId,
        bitfield: NodeId,     // NODE_NONE if not a bitfield
    },

    // ── Statements ──
    CompoundStmt { stmts: Vec<NodeId> },
    IfStmt { cond: NodeId, then_br: NodeId, else_br: NodeId },
    WhileStmt { cond: NodeId, body: NodeId },
    DoWhileStmt { body: NodeId, cond: NodeId },
    ForStmt { init: NodeId, cond: NodeId, incr: NodeId, body: NodeId },
    ReturnStmt { expr: NodeId },
    BreakStmt,
    ContinueStmt,
    SwitchStmt { expr: NodeId, body: NodeId },
    CaseStmt { expr: NodeId, body: NodeId },
    DefaultStmt { body: NodeId },
    GotoStmt { label: crate::source::InternId },
    LabelStmt { label: crate::source::InternId, stmt: NodeId },
    ExprStmt { expr: NodeId },
    NullStmt,

    // ── Expressions ──
    IntLiteral { value: u64, suffix: IntSuffix },
    FloatLiteral { value: f64, suffix: FloatSuffix },
    CharLiteral { value: u8 },
    StringLiteral { bytes: Vec<u8> },
    Ident { name: crate::source::InternId },

    BinaryOp { op: BinOp, lhs: NodeId, rhs: NodeId },
    UnaryOp { op: UnaryOp, operand: NodeId },
    PostfixOp { op: PostfixOp, operand: NodeId },

    /// Assignment: lhs = rhs (or compound: lhs += rhs, etc.)
    Assign { op: AssignOp, lhs: NodeId, rhs: NodeId },

    /// Ternary: cond ? then_expr : else_expr
    Ternary { cond: NodeId, then_expr: NodeId, else_expr: NodeId },

    /// Function call: callee(args...)
    Call { callee: NodeId, args: Vec<NodeId> },

    /// Cast: (type)expr
    Cast { type_node: NodeId, expr: NodeId },

    /// sizeof(type) or sizeof expr
    SizeofType { type_node: NodeId },
    SizeofExpr { expr: NodeId },

    /// Member access: expr.member or expr->member
    MemberAccess { expr: NodeId, member: crate::source::InternId, is_arrow: bool },

    /// Array subscript: expr[index]
    ArraySubscript { expr: NodeId, index: NodeId },

    /// Address-of: &expr
    AddrOf { expr: NodeId },

    /// Dereference: *expr
    Deref { expr: NodeId },

    /// Comma expression: lhs, rhs
    Comma { lhs: NodeId, rhs: NodeId },

    /// Aggregate initializer: { val, val, ... }
    InitList { values: Vec<NodeId> },

    // ── Type Specifier Nodes ──
    // (Used in declaration parsing to represent the declared type)
    TypeSpec { spec: TypeSpecKind },
    PointerTo { base: NodeId, is_const: bool, is_volatile: bool },
    ArrayOf { base: NodeId, size: NodeId }, // size = NODE_NONE for []
    FuncType { return_type: NodeId, params: Vec<NodeId>, is_variadic: bool },
}

// ── Supporting Enums ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageClass {
    None,
    Auto,
    Register,
    Static,
    Extern,
    Typedef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntSuffix {
    None,
    U,
    L,
    UL,
    LL,
    ULL,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatSuffix {
    None,   // double
    F,      // float
    L,      // long double
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    BitAnd, BitOr, BitXor, Shl, Shr,
    LogAnd, LogOr,
    Eq, Ne, Lt, Gt, Le, Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,       // -
    BitNot,    // ~
    LogNot,    // !
    PreInc,    // ++x
    PreDec,    // --x
    Plus,      // +x (no-op for arithmetic)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostfixOp {
    PostInc,   // x++
    PostDec,   // x--
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,     // =
    AddAssign,  // +=
    SubAssign,  // -=
    MulAssign,  // *=
    DivAssign,  // /=
    ModAssign,  // %=
    ShlAssign,  // <<=
    ShrAssign,  // >>=
    AndAssign,  // &=
    XorAssign,  // ^=
    OrAssign,   // |=
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeSpecKind {
    Void,
    Char,
    SignedChar,
    UnsignedChar,
    Short,
    UnsignedShort,
    Int,
    UnsignedInt,
    Long,
    UnsignedLong,
    Float,
    Double,
    LongDouble,
    Signed,
    Unsigned,
    Struct,
    Union,
    Enum,
    TypedefName,
}

// ── Central Context ───────────────────────────────────────────────────

/// Resolved C type — produced by semantic analysis.
/// Stored in the type arena, referenced by TypeId handles.
#[derive(Debug, Clone, PartialEq)]
pub enum CType {
    Void,
    Char,
    SChar,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    Float,
    Double,
    LongDouble,
    Pointer { pointee: TypeId },
    Array { elem: TypeId, len: Option<u64> },
    Struct { tag: Option<crate::source::InternId>, members: Vec<MemberInfo>, size: u32, align: u32, complete: bool },
    Union { tag: Option<crate::source::InternId>, members: Vec<MemberInfo>, size: u32, align: u32, complete: bool },
    Enum { tag: Option<crate::source::InternId> },
    Function { ret: TypeId, params: Vec<(crate::source::InternId, TypeId)>, variadic: bool },
}

/// Information about a struct/union member, with computed offset.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberInfo {
    pub name: crate::source::InternId,
    pub ty: TypeId,
    pub offset: u32,
    pub bitfield_width: Option<u32>,
}

/// Symbol in the symbol table.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: crate::source::InternId,
    pub ty: TypeId,
    pub kind: SymbolKind,
    pub span: Span,
    pub scope: ScopeId,
}

/// Kinds of symbols.
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Variable,
    Function,
    Typedef,
    EnumConstant(i64),
    Parameter,
}

/// A lexical scope.
#[derive(Debug, Clone)]
pub struct Scope {
    pub parent: Option<ScopeId>,
    pub symbols: HashMap<crate::source::InternId, SymbolId>,
    pub kind: ScopeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    File,
    Block,
    Function,
    Prototype,
}

/// The central data store for all compiler arenas.
/// All AST nodes, types, etc. are stored in flat Vecs here.
pub struct Ctx {
    pub target: Target,
    pub nodes: Vec<Node>,
    pub strings: StringInterner,
    // Type system (filled in by sema)
    pub types: Vec<CType>,
    pub scopes: Vec<Scope>,
    pub symbols: Vec<Symbol>,
    pub current_scope: ScopeId,
}

impl Ctx {
    pub fn new(target: Target) -> Self {
        // Create file scope as scope 0
        let file_scope = Scope {
            parent: None,
            symbols: HashMap::new(),
            kind: ScopeKind::File,
        };
        Self {
            target,
            nodes: Vec::with_capacity(4096),
            strings: StringInterner::new(),
            types: Vec::with_capacity(256),
            scopes: vec![file_scope],
            symbols: Vec::with_capacity(256),
            current_scope: ScopeId(0),
        }
    }

    /// Allocate a new AST node and return its handle.
    pub fn push_node(&mut self, kind: NodeKind, span: Span) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            kind,
            span,
            ty: TYPE_NONE,
        });
        id
    }

    /// Get a reference to a node by its handle.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    /// Get a mutable reference to a node by its handle.
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0 as usize]
    }

    /// Intern a string and return a handle.
    pub fn intern(&mut self, s: &str) -> crate::source::InternId {
        self.strings.intern(s)
    }

    /// Look up an interned string.
    pub fn get_str(&self, id: crate::source::InternId) -> &str {
        self.strings.get(id)
    }

    // ── Type Arena ──────────────────────────────────────────────────

    /// Intern a type and return its handle.
    pub fn push_type(&mut self, ty: CType) -> TypeId {
        let id = TypeId(self.types.len() as u32);
        self.types.push(ty);
        id
    }

    /// Get a reference to a type by handle.
    pub fn get_type(&self, id: TypeId) -> &CType {
        &self.types[id.0 as usize]
    }

    /// Get a mutable reference to a type by handle.
    pub fn get_type_mut(&mut self, id: TypeId) -> &mut CType {
        &mut self.types[id.0 as usize]
    }

    // ── Scope Management ────────────────────────────────────────────

    /// Create a new scope under the current one, and enter it.
    pub fn push_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope {
            parent: Some(self.current_scope),
            symbols: HashMap::new(),
            kind,
        });
        self.current_scope = id;
        id
    }

    /// Pop back to the parent scope.
    pub fn pop_scope(&mut self) {
        if let Some(parent) = self.scopes[self.current_scope.0 as usize].parent {
            self.current_scope = parent;
        }
    }

    /// Add a symbol to the current scope.
    pub fn add_symbol(&mut self, sym: Symbol) -> SymbolId {
        let id = SymbolId(self.symbols.len() as u32);
        let name = sym.name;
        self.symbols.push(sym);
        self.scopes[self.current_scope.0 as usize]
            .symbols
            .insert(name, id);
        id
    }

    /// Look up a symbol by name, walking up the scope chain.
    pub fn lookup_symbol(&self, name: crate::source::InternId) -> Option<SymbolId> {
        let mut scope_id = self.current_scope;
        loop {
            let scope = &self.scopes[scope_id.0 as usize];
            if let Some(&sym_id) = scope.symbols.get(&name) {
                return Some(sym_id);
            }
            match scope.parent {
                Some(parent) => scope_id = parent,
                None => return None,
            }
        }
    }

    /// Look up a symbol only in the current scope (for duplicate detection).
    pub fn lookup_symbol_current_scope(&self, name: crate::source::InternId) -> Option<SymbolId> {
        self.scopes[self.current_scope.0 as usize]
            .symbols
            .get(&name)
            .copied()
    }

    /// Get a reference to a symbol by handle.
    pub fn get_symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0 as usize]
    }

    // ── Type Predicates ─────────────────────────────────────────────

    /// Check if a type is an integer type.
    pub fn is_integer_type(&self, ty: TypeId) -> bool {
        if ty == TYPE_NONE { return false; }
        matches!(
            self.get_type(ty),
            CType::Char | CType::SChar | CType::UChar |
            CType::Short | CType::UShort |
            CType::Int | CType::UInt |
            CType::Long | CType::ULong |
            CType::Enum { .. }
        )
    }

    /// Check if a type is an arithmetic type (integer or floating).
    pub fn is_arithmetic_type(&self, ty: TypeId) -> bool {
        if ty == TYPE_NONE { return false; }
        self.is_integer_type(ty) || matches!(
            self.get_type(ty),
            CType::Float | CType::Double | CType::LongDouble
        )
    }

    /// Check if a type is a scalar type (arithmetic or pointer).
    pub fn is_scalar_type(&self, ty: TypeId) -> bool {
        if ty == TYPE_NONE { return false; }
        self.is_arithmetic_type(ty) || matches!(self.get_type(ty), CType::Pointer { .. })
    }

    /// Check if a type is a pointer type.
    pub fn is_pointer_type(&self, ty: TypeId) -> bool {
        if ty == TYPE_NONE { return false; }
        matches!(self.get_type(ty), CType::Pointer { .. })
    }

    /// Get the size of a type in bytes, per the target ABI.
    pub fn type_size(&self, ty: TypeId) -> u32 {
        match self.get_type(ty) {
            CType::Void => 0,
            CType::Char | CType::SChar | CType::UChar => 1,
            CType::Short | CType::UShort => 2,
            CType::Int | CType::UInt | CType::Enum { .. } => 4,
            CType::Long | CType::ULong => self.target.long_size() as u32,
            CType::Float => 4,
            CType::Double => 8,
            CType::LongDouble => self.target.long_double_size() as u32,
            CType::Pointer { .. } => self.target.ptr_size() as u32,
            CType::Array { elem, len } => {
                let elem_size = self.type_size(*elem);
                elem_size * len.unwrap_or(0) as u32
            }
            CType::Struct { size, .. } | CType::Union { size, .. } => *size,
            CType::Function { .. } => 0,
        }
    }

    /// Get the alignment of a type in bytes, per the target ABI.
    pub fn type_align(&self, ty: TypeId) -> u32 {
        match self.get_type(ty) {
            CType::Void => 1,
            CType::Char | CType::SChar | CType::UChar => 1,
            CType::Short | CType::UShort => 2,
            CType::Int | CType::UInt | CType::Enum { .. } => 4,
            CType::Long | CType::ULong => self.target.long_size() as u32,
            CType::Float => 4,
            CType::Double => self.target.double_align() as u32,
            CType::LongDouble => self.target.long_double_align() as u32,
            CType::Pointer { .. } => self.target.ptr_size() as u32,
            CType::Array { elem, .. } => self.type_align(*elem),
            CType::Struct { align, .. } | CType::Union { align, .. } => *align,
            CType::Function { .. } => 1,
        }
    }

    /// Return the LLVM IR type string for a given CType.
    pub fn llvm_type(&self, ty: TypeId) -> String {
        if ty == TYPE_NONE { return "i32".into(); }
        match self.get_type(ty) {
            CType::Void => "void".into(),
            CType::Char | CType::SChar | CType::UChar => "i8".into(),
            CType::Short | CType::UShort => "i16".into(),
            CType::Int | CType::UInt | CType::Enum { .. } => "i32".into(),
            CType::Long | CType::ULong => {
                if self.target.long_size() == 4 { "i32".into() } else { "i64".into() }
            }
            CType::Float => "float".into(),
            CType::Double => "double".into(),
            CType::LongDouble => "x86_fp80".into(),
            CType::Pointer { .. } => "ptr".into(),
            CType::Array { elem, len } => {
                format!("[{} x {}]", len.unwrap_or(0), self.llvm_type(*elem))
            }
            CType::Struct { members, .. } => {
                let fields: Vec<String> = members.iter()
                    .map(|m| self.llvm_type(m.ty))
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            }
            CType::Union { size, .. } => format!("[{} x i8]", size),
            CType::Function { ret, params, variadic } => {
                let ret_s = self.llvm_type(*ret);
                let params_s: Vec<String> = params.iter()
                    .map(|(_, t)| self.llvm_type(*t))
                    .collect();
                let va = if *variadic { ", ..." } else { "" };
                format!("{} ({}{})", ret_s, params_s.join(", "), va)
            }
        }
    }

    /// Return the LLVM IR type string for a function return type (returning void-as-void).
    pub fn llvm_ret_type(&self, ty: TypeId) -> String {
        self.llvm_type(ty)
    }

    /// Check if a type is unsigned.
    pub fn is_unsigned(&self, ty: TypeId) -> bool {
        matches!(
            self.get_type(ty),
            CType::UChar | CType::UShort | CType::UInt | CType::ULong
        )
    }

    /// Check if a type is a floating-point type.
    pub fn is_float_type(&self, ty: TypeId) -> bool {
        matches!(
            self.get_type(ty),
            CType::Float | CType::Double | CType::LongDouble
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_get_node() {
        let mut ctx = Ctx::new(Target::I386);
        let id = ctx.push_node(NodeKind::NullStmt, Span::dummy());
        assert_eq!(id, NodeId(0));
        assert!(matches!(ctx.node(id).kind, NodeKind::NullStmt));
    }

    #[test]
    fn test_node_type_initially_none() {
        let mut ctx = Ctx::new(Target::I386);
        let id = ctx.push_node(NodeKind::NullStmt, Span::dummy());
        assert_eq!(ctx.node(id).ty, TYPE_NONE);
    }

    #[test]
    fn test_intern_and_get_str() {
        let mut ctx = Ctx::new(Target::I386);
        let a = ctx.intern("main");
        let b = ctx.intern("main");
        assert_eq!(a, b);
        assert_eq!(ctx.get_str(a), "main");
    }

    #[test]
    fn test_multiple_nodes() {
        let mut ctx = Ctx::new(Target::I386);
        let n0 = ctx.push_node(NodeKind::NullStmt, Span::dummy());
        let n1 = ctx.push_node(NodeKind::BreakStmt, Span::dummy());
        let n2 = ctx.push_node(NodeKind::ContinueStmt, Span::dummy());
        assert_eq!(n0, NodeId(0));
        assert_eq!(n1, NodeId(1));
        assert_eq!(n2, NodeId(2));
        assert!(matches!(ctx.node(n1).kind, NodeKind::BreakStmt));
    }

    #[test]
    fn test_node_none_sentinel() {
        assert_eq!(NODE_NONE.0, u32::MAX);
    }
}
