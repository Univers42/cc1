// ir/types.rs — IR type system.
//
// These types are target-independent descriptions that the backend uses
// for instruction selection, register allocation, and ABI classification.

use crate::target::Target;

/// SSA value handle — index into a function's value table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ValueId(pub u32);

/// Basic block handle — index into a function's block list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

/// Sentinel: no value.
pub const VALUE_NONE: ValueId = ValueId(u32::MAX);

/// Sentinel: no block.
pub const BLOCK_NONE: BlockId = BlockId(u32::MAX);

// ── IR Types ──────────────────────────────────────────────────────────

/// Machine-level type for IR values. Closer to hardware than CType.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IrType {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    I128,
    U128,
    F32,
    F64,
    F128,
    Ptr,
    Void,
    /// Aggregate struct (for ABI pass-by-value).
    Struct(Vec<IrType>),
    /// Fixed-size array (for aggregates).
    Array(Box<IrType>, u64),
}

impl IrType {
    /// Size in bytes on the given target.
    pub fn size(&self, target: Target) -> u64 {
        match self {
            IrType::I8 | IrType::U8 => 1,
            IrType::I16 | IrType::U16 => 2,
            IrType::I32 | IrType::U32 | IrType::F32 => 4,
            IrType::I64 | IrType::U64 | IrType::F64 => 8,
            IrType::I128 | IrType::U128 => 16,
            IrType::F128 => 16,
            IrType::Ptr => target.ptr_size() as u64,
            IrType::Void => 0,
            IrType::Struct(fields) => {
                let mut offset: u64 = 0;
                for f in fields {
                    let align = f.align(target);
                    offset = (offset + align - 1) & !(align - 1);
                    offset += f.size(target);
                }
                let struct_align = self.align(target);
                (offset + struct_align - 1) & !(struct_align - 1)
            }
            IrType::Array(elem, count) => elem.size(target) * count,
        }
    }

    /// Alignment in bytes on the given target.
    pub fn align(&self, target: Target) -> u64 {
        match self {
            IrType::I8 | IrType::U8 => 1,
            IrType::I16 | IrType::U16 => 2,
            IrType::I32 | IrType::U32 | IrType::F32 => 4,
            IrType::I64 | IrType::U64 | IrType::F64 => 8,
            IrType::I128 | IrType::U128 => 16,
            IrType::F128 => 16,
            IrType::Ptr => target.ptr_size() as u64,
            IrType::Void => 1,
            IrType::Struct(fields) => {
                fields.iter().map(|f| f.align(target)).max().unwrap_or(1)
            }
            IrType::Array(elem, _) => elem.align(target),
        }
    }

    /// Bit width for integer types; 0 for non-integer.
    pub fn bit_width(&self) -> u32 {
        match self {
            IrType::I8 | IrType::U8 => 8,
            IrType::I16 | IrType::U16 => 16,
            IrType::I32 | IrType::U32 => 32,
            IrType::I64 | IrType::U64 => 64,
            IrType::I128 | IrType::U128 => 128,
            IrType::Ptr => 64, // default; overrid by target
            _ => 0,
        }
    }

    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            IrType::I8
                | IrType::I16
                | IrType::I32
                | IrType::I64
                | IrType::I128
                | IrType::U8
                | IrType::U16
                | IrType::U32
                | IrType::U64
                | IrType::U128
        )
    }

    pub fn is_signed(&self) -> bool {
        matches!(
            self,
            IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64 | IrType::I128
        )
    }

    pub fn is_unsigned(&self) -> bool {
        matches!(
            self,
            IrType::U8 | IrType::U16 | IrType::U32 | IrType::U64 | IrType::U128
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, IrType::F32 | IrType::F64 | IrType::F128)
    }

    pub fn is_pointer(&self) -> bool {
        matches!(self, IrType::Ptr)
    }

    pub fn is_void(&self) -> bool {
        matches!(self, IrType::Void)
    }

    pub fn is_aggregate(&self) -> bool {
        matches!(self, IrType::Struct(_) | IrType::Array(..))
    }

    /// The corresponding unsigned type (for signed integers).
    pub fn to_unsigned(&self) -> IrType {
        match self {
            IrType::I8 => IrType::U8,
            IrType::I16 => IrType::U16,
            IrType::I32 => IrType::U32,
            IrType::I64 => IrType::U64,
            IrType::I128 => IrType::U128,
            other => other.clone(),
        }
    }

    /// The corresponding signed type (for unsigned integers).
    pub fn to_signed(&self) -> IrType {
        match self {
            IrType::U8 => IrType::I8,
            IrType::U16 => IrType::I16,
            IrType::U32 => IrType::I32,
            IrType::U64 => IrType::I64,
            IrType::U128 => IrType::I128,
            other => other.clone(),
        }
    }
}

impl std::fmt::Display for IrType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IrType::I8 => write!(f, "i8"),
            IrType::I16 => write!(f, "i16"),
            IrType::I32 => write!(f, "i32"),
            IrType::I64 => write!(f, "i64"),
            IrType::U8 => write!(f, "u8"),
            IrType::U16 => write!(f, "u16"),
            IrType::U32 => write!(f, "u32"),
            IrType::U64 => write!(f, "u64"),
            IrType::I128 => write!(f, "i128"),
            IrType::U128 => write!(f, "u128"),
            IrType::F32 => write!(f, "f32"),
            IrType::F64 => write!(f, "f64"),
            IrType::F128 => write!(f, "f128"),
            IrType::Ptr => write!(f, "ptr"),
            IrType::Void => write!(f, "void"),
            IrType::Struct(fields) => {
                write!(f, "{{ ")?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", field)?;
                }
                write!(f, " }}")
            }
            IrType::Array(elem, len) => write!(f, "[{} x {}]", len, elem),
        }
    }
}

// ── Operand ───────────────────────────────────────────────────────────

/// An operand to an IR instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// An SSA value produced by another instruction.
    Value(ValueId),
    /// A compile-time constant.
    Const(ConstValue),
    /// A global symbol reference.
    Global(String),
    /// A basic block label (for phi nodes and branches).
    Label(BlockId),
}

impl Operand {
    /// If this operand is a Value, return its ValueId.
    pub fn as_value(&self) -> Option<ValueId> {
        match self {
            Operand::Value(v) => Some(*v),
            _ => None,
        }
    }

    /// If this operand is a constant integer, return its value.
    pub fn as_const_i64(&self) -> Option<i64> {
        match self {
            Operand::Const(ConstValue::I32(v)) => Some(*v as i64),
            Operand::Const(ConstValue::I64(v)) => Some(*v),
            Operand::Const(ConstValue::I8(v)) => Some(*v as i64),
            Operand::Const(ConstValue::I16(v)) => Some(*v as i64),
            _ => None,
        }
    }
}

impl std::fmt::Display for Operand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Operand::Value(v) => write!(f, "%{}", v.0),
            Operand::Const(c) => write!(f, "{}", c),
            Operand::Global(name) => write!(f, "@{}", name),
            Operand::Label(b) => write!(f, "bb{}", b.0),
        }
    }
}

// ── Constant Values ───────────────────────────────────────────────────

/// Compile-time constant value.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    NullPtr,
    Undef,
    ZeroInit,
    /// A byte string (for string literal initializers).
    Bytes(Vec<u8>),
}

impl std::fmt::Display for ConstValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstValue::I8(v) => write!(f, "{}", v),
            ConstValue::I16(v) => write!(f, "{}", v),
            ConstValue::I32(v) => write!(f, "{}", v),
            ConstValue::I64(v) => write!(f, "{}", v),
            ConstValue::U8(v) => write!(f, "{}", v),
            ConstValue::U16(v) => write!(f, "{}", v),
            ConstValue::U32(v) => write!(f, "{}", v),
            ConstValue::U64(v) => write!(f, "{}", v),
            ConstValue::F32(v) => write!(f, "{:e}", v),
            ConstValue::F64(v) => write!(f, "{:e}", v),
            ConstValue::NullPtr => write!(f, "null"),
            ConstValue::Undef => write!(f, "undef"),
            ConstValue::ZeroInit => write!(f, "zeroinit"),
            ConstValue::Bytes(b) => write!(f, "c\"{}\"", String::from_utf8_lossy(b)),
        }
    }
}

// ── Comparison Predicates ─────────────────────────────────────────────

/// Integer comparison predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IcmpPred {
    Eq,
    Ne,
    Slt,
    Sgt,
    Sle,
    Sge,
    Ult,
    Ugt,
    Ule,
    Uge,
}

impl std::fmt::Display for IcmpPred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IcmpPred::Eq => write!(f, "eq"),
            IcmpPred::Ne => write!(f, "ne"),
            IcmpPred::Slt => write!(f, "slt"),
            IcmpPred::Sgt => write!(f, "sgt"),
            IcmpPred::Sle => write!(f, "sle"),
            IcmpPred::Sge => write!(f, "sge"),
            IcmpPred::Ult => write!(f, "ult"),
            IcmpPred::Ugt => write!(f, "ugt"),
            IcmpPred::Ule => write!(f, "ule"),
            IcmpPred::Uge => write!(f, "uge"),
        }
    }
}

/// Floating-point comparison predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FcmpPred {
    Oeq,
    One,
    Olt,
    Ogt,
    Ole,
    Oge,
    Ord,
    Uno,
    Ueq,
    Une,
    Ult,
    Ugt,
    Ule,
    Uge,
}

impl std::fmt::Display for FcmpPred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FcmpPred::Oeq => write!(f, "oeq"),
            FcmpPred::One => write!(f, "one"),
            FcmpPred::Olt => write!(f, "olt"),
            FcmpPred::Ogt => write!(f, "ogt"),
            FcmpPred::Ole => write!(f, "ole"),
            FcmpPred::Oge => write!(f, "oge"),
            FcmpPred::Ord => write!(f, "ord"),
            FcmpPred::Uno => write!(f, "uno"),
            FcmpPred::Ueq => write!(f, "ueq"),
            FcmpPred::Une => write!(f, "une"),
            FcmpPred::Ult => write!(f, "ult"),
            FcmpPred::Ugt => write!(f, "ugt"),
            FcmpPred::Ule => write!(f, "ule"),
            FcmpPred::Uge => write!(f, "uge"),
        }
    }
}

/// Binary operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    SDiv,
    UDiv,
    SRem,
    URem,
    And,
    Or,
    Xor,
    Shl,
    LShr,
    AShr,

    // Float operations.
    FAdd,
    FSub,
    FMul,
    FDiv,
    FRem,
}

impl std::fmt::Display for BinOpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinOpKind::Add => write!(f, "add"),
            BinOpKind::Sub => write!(f, "sub"),
            BinOpKind::Mul => write!(f, "mul"),
            BinOpKind::SDiv => write!(f, "sdiv"),
            BinOpKind::UDiv => write!(f, "udiv"),
            BinOpKind::SRem => write!(f, "srem"),
            BinOpKind::URem => write!(f, "urem"),
            BinOpKind::And => write!(f, "and"),
            BinOpKind::Or => write!(f, "or"),
            BinOpKind::Xor => write!(f, "xor"),
            BinOpKind::Shl => write!(f, "shl"),
            BinOpKind::LShr => write!(f, "lshr"),
            BinOpKind::AShr => write!(f, "ashr"),
            BinOpKind::FAdd => write!(f, "fadd"),
            BinOpKind::FSub => write!(f, "fsub"),
            BinOpKind::FMul => write!(f, "fmul"),
            BinOpKind::FDiv => write!(f, "fdiv"),
            BinOpKind::FRem => write!(f, "frem"),
        }
    }
}

/// Unary operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOpKind {
    Neg,
    FNeg,
    BitNot,
    LogNot,
}

impl std::fmt::Display for UnaryOpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnaryOpKind::Neg => write!(f, "neg"),
            UnaryOpKind::FNeg => write!(f, "fneg"),
            UnaryOpKind::BitNot => write!(f, "bitnot"),
            UnaryOpKind::LogNot => write!(f, "lognot"),
        }
    }
}

/// Cast kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CastKind {
    ZExt,
    SExt,
    Trunc,
    FPToSI,
    FPToUI,
    SIToFP,
    UIToFP,
    FPExt,
    FPTrunc,
    PtrToInt,
    IntToPtr,
    Bitcast,
}

impl std::fmt::Display for CastKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CastKind::ZExt => write!(f, "zext"),
            CastKind::SExt => write!(f, "sext"),
            CastKind::Trunc => write!(f, "trunc"),
            CastKind::FPToSI => write!(f, "fptosi"),
            CastKind::FPToUI => write!(f, "fptoui"),
            CastKind::SIToFP => write!(f, "sitofp"),
            CastKind::UIToFP => write!(f, "uitofp"),
            CastKind::FPExt => write!(f, "fpext"),
            CastKind::FPTrunc => write!(f, "fptrunc"),
            CastKind::PtrToInt => write!(f, "ptrtoint"),
            CastKind::IntToPtr => write!(f, "inttoptr"),
            CastKind::Bitcast => write!(f, "bitcast"),
        }
    }
}

/// Linkage kind for functions and globals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Linkage {
    External,
    Internal,
    Private,
    Weak,
    Common,
}

/// Visibility kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    Default,
    Hidden,
    Protected,
    Internal,
}

/// Atomic memory ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtomicOrdering {
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

/// Atomic RMW operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtomicRmwOp {
    Xchg,
    Add,
    Sub,
    And,
    Or,
    Xor,
    Max,
    Min,
    UMax,
    UMin,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::Target;

    #[test]
    fn test_ir_type_sizes() {
        let t = Target::X86_64;
        assert_eq!(IrType::I8.size(t), 1);
        assert_eq!(IrType::I16.size(t), 2);
        assert_eq!(IrType::I32.size(t), 4);
        assert_eq!(IrType::I64.size(t), 8);
        assert_eq!(IrType::F32.size(t), 4);
        assert_eq!(IrType::F64.size(t), 8);
        assert_eq!(IrType::Ptr.size(t), 8);
        assert_eq!(IrType::Void.size(t), 0);
        assert_eq!(IrType::I128.size(t), 16);
    }

    #[test]
    fn test_ir_type_sizes_i386() {
        let t = Target::I386;
        assert_eq!(IrType::Ptr.size(t), 4);
        assert_eq!(IrType::I64.size(t), 8);
    }

    #[test]
    fn test_ir_type_predicates() {
        assert!(IrType::I32.is_integer());
        assert!(IrType::U64.is_integer());
        assert!(IrType::I32.is_signed());
        assert!(IrType::U32.is_unsigned());
        assert!(IrType::F64.is_float());
        assert!(IrType::Ptr.is_pointer());
        assert!(IrType::Void.is_void());
        assert!(!IrType::I32.is_float());
        assert!(!IrType::F32.is_integer());
    }

    #[test]
    fn test_ir_type_bit_width() {
        assert_eq!(IrType::I8.bit_width(), 8);
        assert_eq!(IrType::I16.bit_width(), 16);
        assert_eq!(IrType::I32.bit_width(), 32);
        assert_eq!(IrType::I64.bit_width(), 64);
        assert_eq!(IrType::I128.bit_width(), 128);
        assert_eq!(IrType::F32.bit_width(), 0);
    }

    #[test]
    fn test_struct_size_align() {
        let t = Target::X86_64;
        // struct { i8, i32 } => align 4, offset 0+pad3+4 = 8
        let s = IrType::Struct(vec![IrType::I8, IrType::I32]);
        assert_eq!(s.align(t), 4);
        assert_eq!(s.size(t), 8);
    }

    #[test]
    fn test_array_size() {
        let t = Target::X86_64;
        let arr = IrType::Array(Box::new(IrType::I32), 10);
        assert_eq!(arr.size(t), 40);
        assert_eq!(arr.align(t), 4);
    }

    #[test]
    fn test_display_types() {
        assert_eq!(format!("{}", IrType::I32), "i32");
        assert_eq!(format!("{}", IrType::Ptr), "ptr");
        assert_eq!(
            format!("{}", IrType::Struct(vec![IrType::I32, IrType::I64])),
            "{ i32, i64 }"
        );
        assert_eq!(
            format!("{}", IrType::Array(Box::new(IrType::I8), 100)),
            "[100 x i8]"
        );
    }

    #[test]
    fn test_sign_conversion() {
        assert_eq!(IrType::I32.to_unsigned(), IrType::U32);
        assert_eq!(IrType::U64.to_signed(), IrType::I64);
        assert_eq!(IrType::F32.to_unsigned(), IrType::F32); // no-op for floats
    }

    #[test]
    fn test_operand_display() {
        assert_eq!(format!("{}", Operand::Value(ValueId(3))), "%3");
        assert_eq!(format!("{}", Operand::Const(ConstValue::I32(42))), "42");
        assert_eq!(format!("{}", Operand::Global("main".into())), "@main");
        assert_eq!(format!("{}", Operand::Label(BlockId(0))), "bb0");
    }

    #[test]
    fn test_const_value_display() {
        assert_eq!(format!("{}", ConstValue::NullPtr), "null");
        assert_eq!(format!("{}", ConstValue::Undef), "undef");
        assert_eq!(format!("{}", ConstValue::I32(-1)), "-1");
    }

    #[test]
    fn test_operand_as_value() {
        let op = Operand::Value(ValueId(5));
        assert_eq!(op.as_value(), Some(ValueId(5)));

        let op2 = Operand::Const(ConstValue::I32(10));
        assert_eq!(op2.as_value(), None);
    }
}
