// ir/module.rs — Top-level IR structures: module, function, basic block, globals.

use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::types::*;

// ─── BasicBlock ──────────────────────────────────────────────────────────────

/// A basic block within an IR function.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    /// Block identifier (index in `IrFunction::blocks`).
    pub id: BlockId,
    /// Human-readable label (e.g., `"entry"`, `"if.then"`, `"for.cond"`).
    pub label: String,
    /// Instructions in this block (excluding the terminator).
    pub insts: Vec<Instruction>,
    /// Terminator — every well-formed block has exactly one.
    pub terminator: Terminator,
    /// Predecessor block ids (filled in after construction).
    pub preds: Vec<BlockId>,
}

impl BasicBlock {
    pub fn new(id: BlockId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            insts: Vec::new(),
            terminator: Terminator::Unreachable,
            preds: Vec::new(),
        }
    }

    /// Append an instruction and return its result ValueId (if any).
    pub fn push(&mut self, inst: Instruction) -> Option<ValueId> {
        let result = inst.result();
        self.insts.push(inst);
        result
    }

    /// Set the block terminator.
    pub fn set_terminator(&mut self, term: Terminator) {
        self.terminator = term;
    }
}

// ─── IrFunction ──────────────────────────────────────────────────────────────

/// One IR param: name hint + type.
#[derive(Debug, Clone)]
pub struct IrParam {
    pub name: String,
    pub ty: IrType,
    /// The ValueId assigned to this parameter in the function.
    pub value: ValueId,
}

/// An IR function definition.
#[derive(Debug, Clone)]
pub struct IrFunction {
    /// Mangled name.
    pub name: String,
    /// Return type (Void for void functions).
    pub ret_ty: IrType,
    /// Parameter list.
    pub params: Vec<IrParam>,
    /// Whether this is a variadic function (...).
    pub is_variadic: bool,
    /// Basic blocks (index 0 is always the entry block).
    pub blocks: Vec<BasicBlock>,
    /// Linkage class.
    pub linkage: Linkage,
    /// Section override (e.g., ".text.startup").
    pub section: Option<String>,
    /// Value counter — next value id to allocate.
    next_value: u32,
    /// Block counter — next block id to allocate.
    next_block: u32,
}

impl IrFunction {
    pub fn new(name: impl Into<String>, ret_ty: IrType, linkage: Linkage) -> Self {
        Self {
            name: name.into(),
            ret_ty,
            params: Vec::new(),
            is_variadic: false,
            blocks: Vec::new(),
            linkage,
            section: None,
            next_value: 0,
            next_block: 0,
        }
    }

    /// Add a parameter and return the ValueId assigned to it.
    pub fn add_param(&mut self, name: impl Into<String>, ty: IrType) -> ValueId {
        let vid = self.alloc_value();
        self.params.push(IrParam {
            name: name.into(),
            ty,
            value: vid,
        });
        vid
    }

    /// Allocate a new ValueId.
    pub fn alloc_value(&mut self) -> ValueId {
        let id = ValueId(self.next_value);
        self.next_value += 1;
        id
    }

    /// Create a new basic block and return its BlockId.
    pub fn create_block(&mut self, label: impl Into<String>) -> BlockId {
        let bid = BlockId(self.next_block);
        self.next_block += 1;
        self.blocks.push(BasicBlock::new(bid, label));
        bid
    }

    /// Get a mutable reference to a block by its BlockId.
    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock {
        &mut self.blocks[id.0 as usize]
    }

    /// Get an immutable reference to a block by its BlockId.
    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id.0 as usize]
    }

    /// The entry block id (always BlockId(0)).
    pub fn entry_block(&self) -> BlockId {
        BlockId(0)
    }

    /// Number of values allocated so far.
    pub fn value_count(&self) -> u32 {
        self.next_value
    }

    /// Number of blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Compute predecessor lists for all blocks.
    pub fn compute_predecessors(&mut self) {
        // Clear existing
        for bb in &mut self.blocks {
            bb.preds.clear();
        }
        // Collect edges
        let edges: Vec<(BlockId, BlockId)> = self
            .blocks
            .iter()
            .flat_map(|bb| {
                bb.terminator
                    .successors()
                    .into_iter()
                    .map(move |succ| (bb.id, succ))
            })
            .collect();
        for (pred, succ) in edges {
            self.blocks[succ.0 as usize].preds.push(pred);
        }
    }
}

// ─── GlobalInit ──────────────────────────────────────────────────────────────

/// Initializer for global / static variables.
#[derive(Debug, Clone)]
pub enum GlobalInit {
    /// Integer constant (stored in `u64`, truncated to actual width).
    Integer(u64),
    /// Floating-point constant (f64 covers f32 too; f128 stored in two u64s).
    Float(f64),
    /// A string literal (null-terminated bytes).
    String(Vec<u8>),
    /// Address of another global symbol + optional byte offset.
    Address { symbol: String, offset: i64 },
    /// Compound initializer (struct/array): list of (byte_offset, init).
    Compound(Vec<(usize, GlobalInit)>),
    /// Zero-fill N bytes (BSS-like).
    ZeroFill(usize),
    /// Label difference (GNU extension: `.long .L1 - .L2`).
    LabelDiff { pos: String, neg: String },
}

// ─── GlobalVariable ──────────────────────────────────────────────────────────

/// A global or static variable.
#[derive(Debug, Clone)]
pub struct GlobalVariable {
    /// Symbol name.
    pub name: String,
    /// Type of the data stored.
    pub ty: IrType,
    /// Initializer (None → extern declaration).
    pub init: Option<GlobalInit>,
    /// Linkage class.
    pub linkage: Linkage,
    /// Visibility.
    pub visibility: Visibility,
    /// Explicit section name override.
    pub section: Option<String>,
    /// Alignment in bytes (0 = natural).
    pub align: u32,
    /// true → goes in .rodata / const section.
    pub is_const: bool,
    /// true → thread-local storage.
    pub is_tls: bool,
}

/// An extern function declaration (not a definition).
#[derive(Debug, Clone)]
pub struct ExternFunc {
    pub name: String,
    pub ret_ty: IrType,
    pub params: Vec<IrType>,
    pub is_variadic: bool,
}

// ─── IrModule ────────────────────────────────────────────────────────────────

/// The top-level IR module representing one translation unit.
#[derive(Debug, Clone)]
pub struct IrModule {
    /// Source file name.
    pub source_file: String,
    /// Function definitions.
    pub functions: Vec<IrFunction>,
    /// Global variables and static data.
    pub globals: Vec<GlobalVariable>,
    /// Extern function declarations (no body).
    pub externs: Vec<ExternFunc>,
    /// String literal pool: (label, bytes).
    pub string_literals: Vec<(String, Vec<u8>)>,
    /// Next unique string literal counter.
    string_counter: u32,
}

impl IrModule {
    pub fn new(source_file: impl Into<String>) -> Self {
        Self {
            source_file: source_file.into(),
            functions: Vec::new(),
            globals: Vec::new(),
            externs: Vec::new(),
            string_literals: Vec::new(),
            string_counter: 0,
        }
    }

    /// Add a function definition.
    pub fn add_function(&mut self, func: IrFunction) -> usize {
        let idx = self.functions.len();
        self.functions.push(func);
        idx
    }

    /// Add a global variable.
    pub fn add_global(&mut self, gv: GlobalVariable) -> usize {
        let idx = self.globals.len();
        self.globals.push(gv);
        idx
    }

    /// Add an extern function declaration.
    pub fn add_extern(&mut self, ef: ExternFunc) -> usize {
        let idx = self.externs.len();
        self.externs.push(ef);
        idx
    }

    /// Intern a string literal, returning its label name (e.g., `.LC0`).
    pub fn intern_string(&mut self, bytes: Vec<u8>) -> String {
        // Deduplicate
        for (label, existing) in &self.string_literals {
            if existing == &bytes {
                return label.clone();
            }
        }
        let label = format!(".LC{}", self.string_counter);
        self.string_counter += 1;
        self.string_literals.push((label.clone(), bytes));
        label
    }

    /// Find a function definition by name.
    pub fn find_function(&self, name: &str) -> Option<&IrFunction> {
        self.functions.iter().find(|f| f.name == name)
    }

    /// Find a global variable by name.
    pub fn find_global(&self, name: &str) -> Option<&GlobalVariable> {
        self.globals.iter().find(|g| g.name == name)
    }

    /// Compute predecessors for all functions.
    pub fn compute_all_predecessors(&mut self) {
        for func in &mut self.functions {
            func.compute_predecessors();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_module() {
        let m = IrModule::new("test.c");
        assert_eq!(m.source_file, "test.c");
        assert!(m.functions.is_empty());
        assert!(m.globals.is_empty());
        assert!(m.externs.is_empty());
    }

    #[test]
    fn test_create_function() {
        let mut f = IrFunction::new("main", IrType::I32, Linkage::External);
        let argc = f.add_param("argc", IrType::I32);
        assert_eq!(argc, ValueId(0));
        let argv = f.add_param("argv", IrType::Ptr);
        assert_eq!(argv, ValueId(1));
        assert_eq!(f.params.len(), 2);
    }

    #[test]
    fn test_create_blocks() {
        let mut f = IrFunction::new("foo", IrType::Void, Linkage::External);
        let entry = f.create_block("entry");
        let then = f.create_block("if.then");
        let else_ = f.create_block("if.else");
        assert_eq!(entry, BlockId(0));
        assert_eq!(then, BlockId(1));
        assert_eq!(else_, BlockId(2));
        assert_eq!(f.block_count(), 3);
    }

    #[test]
    fn test_block_instructions() {
        let mut f = IrFunction::new("bar", IrType::I32, Linkage::External);
        let bid = f.create_block("entry");
        let v0 = f.alloc_value();
        f.block_mut(bid).push(Instruction::Alloca {
            result: v0,
            ty: IrType::I32,
            align: 4,
        });
        let v1 = f.alloc_value();
        f.block_mut(bid).push(Instruction::Load {
            result: v1,
            addr: Operand::Value(v0),
            ty: IrType::I32,
        });
        f.block_mut(bid).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v1)),
        });
        assert_eq!(f.block(bid).insts.len(), 2);
    }

    #[test]
    fn test_compute_predecessors() {
        let mut f = IrFunction::new("cfg", IrType::Void, Linkage::External);
        let b0 = f.create_block("entry");
        let b1 = f.create_block("loop.header");
        let b2 = f.create_block("loop.body");
        let b3 = f.create_block("exit");

        let cond = f.alloc_value();
        // entry -> loop.header
        f.block_mut(b0)
            .set_terminator(Terminator::Br { target: b1 });
        // loop.header -> loop.body or exit
        f.block_mut(b1).set_terminator(Terminator::CondBr {
            cond: Operand::Value(cond),
            true_bb: b2,
            false_bb: b3,
        });
        // loop.body -> loop.header (back edge)
        f.block_mut(b2)
            .set_terminator(Terminator::Br { target: b1 });
        // exit: return
        f.block_mut(b3).set_terminator(Terminator::Ret {
            value: None,
        });

        f.compute_predecessors();

        assert!(f.block(b0).preds.is_empty()); // entry has no preds
        assert_eq!(f.block(b1).preds, vec![BlockId(0), BlockId(2)]); // from entry + back edge
        assert_eq!(f.block(b2).preds, vec![BlockId(1)]); // from header
        assert_eq!(f.block(b3).preds, vec![BlockId(1)]); // from header
    }

    #[test]
    fn test_string_interning() {
        let mut m = IrModule::new("test.c");
        let l1 = m.intern_string(b"hello\0".to_vec());
        let l2 = m.intern_string(b"world\0".to_vec());
        let l3 = m.intern_string(b"hello\0".to_vec()); // duplicate
        assert_eq!(l1, ".LC0");
        assert_eq!(l2, ".LC1");
        assert_eq!(l3, ".LC0"); // deduplicated
        assert_eq!(m.string_literals.len(), 2);
    }

    #[test]
    fn test_global_variable() {
        let mut m = IrModule::new("test.c");
        m.add_global(GlobalVariable {
            name: "counter".into(),
            ty: IrType::I32,
            init: Some(GlobalInit::Integer(0)),
            linkage: Linkage::External,
            visibility: Visibility::Default,
            section: None,
            align: 4,
            is_const: false,
            is_tls: false,
        });
        m.add_global(GlobalVariable {
            name: "msg".into(),
            ty: IrType::Array(Box::new(IrType::I8), 6),
            init: Some(GlobalInit::String(b"hello\0".to_vec())),
            linkage: Linkage::Internal,
            visibility: Visibility::Default,
            section: None,
            align: 1,
            is_const: true,
            is_tls: false,
        });
        assert_eq!(m.globals.len(), 2);
        assert!(m.find_global("counter").is_some());
        assert!(m.find_global("msg").unwrap().is_const);
    }

    #[test]
    fn test_extern_func() {
        let mut m = IrModule::new("test.c");
        m.add_extern(ExternFunc {
            name: "printf".into(),
            ret_ty: IrType::I32,
            params: vec![IrType::Ptr],
            is_variadic: true,
        });
        assert_eq!(m.externs.len(), 1);
        assert_eq!(m.externs[0].name, "printf");
    }
}
