// target.rs — Target architecture enum with ABI size/alignment tables.
//
// Every phase of the compiler flows through Target. No hardcoded sizes anywhere.
// System V ABI i386: §2.1 Data Representation
// System V ABI x86_64: §3.1.2 Data Representation

/// Target architecture for code generation and semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    I386,
    X86_64,
}

impl Target {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "i386" | "i686" | "x86" | "ia32" => Ok(Target::I386),
            "x86_64" | "x86-64" | "amd64" | "x64" => Ok(Target::X86_64),
            _ => Err(format!("unknown target architecture: '{}'", s)),
        }
    }

    /// Pointer size in bytes.
    pub fn ptr_size(self) -> u32 {
        match self {
            Target::I386 => 4,
            Target::X86_64 => 8,
        }
    }

    /// Pointer alignment in bytes.
    pub fn ptr_align(self) -> u32 {
        self.ptr_size()
    }

    /// Size of `long` / `unsigned long` in bytes.
    pub fn long_size(self) -> u32 {
        match self {
            Target::I386 => 4,
            Target::X86_64 => 8,
        }
    }

    /// Alignment of `long` in bytes.
    pub fn long_align(self) -> u32 {
        self.long_size()
    }

    /// Size of `long long` / `unsigned long long` in bytes.
    pub fn long_long_size(self) -> u32 {
        8
    }

    /// Alignment of `long long` in bytes.
    pub fn long_long_align(self) -> u32 {
        match self {
            Target::I386 => 4,
            Target::X86_64 => 8,
        }
    }

    /// Alignment of `double` in bytes.
    pub fn double_align(self) -> u32 {
        match self {
            Target::I386 => 4,
            Target::X86_64 => 8,
        }
    }

    /// Size of `long double` in bytes.
    pub fn long_double_size(self) -> u32 {
        match self {
            Target::I386 => 12,
            Target::X86_64 => 16,
        }
    }

    /// Alignment of `long double` in bytes.
    pub fn long_double_align(self) -> u32 {
        match self {
            Target::I386 => 4,
            Target::X86_64 => 16,
        }
    }

    /// Target triple string for LLVM.
    pub fn triple(self) -> &'static str {
        match self {
            Target::I386 => "i386-pc-linux-gnu",
            Target::X86_64 => "x86_64-pc-linux-gnu",
        }
    }

    /// LLVM datalayout string.
    pub fn datalayout(self) -> &'static str {
        match self {
            Target::I386 => "e-m:e-p:32:32-f64:32:64-f80:32-n8:16:32-S128",
            Target::X86_64 => "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128",
        }
    }

    /// Maximum alignment for any scalar type on this target.
    pub fn max_align(self) -> u32 {
        match self {
            Target::I386 => 4,   // i386: no type requires > 4-byte alignment naturally
            Target::X86_64 => 16, // long double = 16-byte aligned
        }
    }
}

/// Scalar type descriptor for ABI queries. Each variant carries its (size, align)
/// as determined by the System V ABI for the given target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarKind {
    Bool,
    Char,
    SChar,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    LongLong,
    ULongLong,
    Float,
    Double,
    LongDouble,
    Pointer,
}

impl ScalarKind {
    /// Returns (size_bytes, align_bytes) for this scalar on the given target.
    pub fn size_align(self, target: Target) -> (u32, u32) {
        match self {
            ScalarKind::Bool => (1, 1),
            ScalarKind::Char | ScalarKind::SChar | ScalarKind::UChar => (1, 1),
            ScalarKind::Short | ScalarKind::UShort => (2, 2),
            ScalarKind::Int | ScalarKind::UInt => (4, 4),
            ScalarKind::Long | ScalarKind::ULong => (target.long_size(), target.long_align()),
            ScalarKind::LongLong | ScalarKind::ULongLong => {
                (target.long_long_size(), target.long_long_align())
            }
            ScalarKind::Float => (4, 4),
            ScalarKind::Double => (8, target.double_align()),
            ScalarKind::LongDouble => (target.long_double_size(), target.long_double_align()),
            ScalarKind::Pointer => (target.ptr_size(), target.ptr_align()),
        }
    }

    /// Returns the size in bytes.
    pub fn size(self, target: Target) -> u32 {
        self.size_align(target).0
    }

    /// Returns the alignment in bytes.
    pub fn align(self, target: Target) -> u32 {
        self.size_align(target).1
    }

    /// Returns true if this is an unsigned integer type.
    pub fn is_unsigned(self) -> bool {
        matches!(
            self,
            ScalarKind::Bool
                | ScalarKind::UChar
                | ScalarKind::UShort
                | ScalarKind::UInt
                | ScalarKind::ULong
                | ScalarKind::ULongLong
        )
    }

    /// Returns true if this is a floating-point type.
    pub fn is_float(self) -> bool {
        matches!(
            self,
            ScalarKind::Float | ScalarKind::Double | ScalarKind::LongDouble
        )
    }

    /// Returns true if this is an integer type (including char, bool).
    pub fn is_integer(self) -> bool {
        !self.is_float() && !matches!(self, ScalarKind::Pointer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_i386_ptr_size() {
        assert_eq!(Target::I386.ptr_size(), 4);
    }

    #[test]
    fn test_x86_64_ptr_size() {
        assert_eq!(Target::X86_64.ptr_size(), 8);
    }

    #[test]
    fn test_i386_long_size() {
        assert_eq!(Target::I386.long_size(), 4);
    }

    #[test]
    fn test_x86_64_long_size() {
        assert_eq!(Target::X86_64.long_size(), 8);
    }

    #[test]
    fn test_i386_double_align() {
        assert_eq!(Target::I386.double_align(), 4);
    }

    #[test]
    fn test_x86_64_double_align() {
        assert_eq!(Target::X86_64.double_align(), 8);
    }

    #[test]
    fn test_i386_long_double() {
        assert_eq!(Target::I386.long_double_size(), 12);
        assert_eq!(Target::I386.long_double_align(), 4);
    }

    #[test]
    fn test_x86_64_long_double() {
        assert_eq!(Target::X86_64.long_double_size(), 16);
        assert_eq!(Target::X86_64.long_double_align(), 16);
    }

    #[test]
    fn test_scalar_size_align_i386() {
        let t = Target::I386;
        assert_eq!(ScalarKind::Bool.size_align(t), (1, 1));
        assert_eq!(ScalarKind::Char.size_align(t), (1, 1));
        assert_eq!(ScalarKind::Short.size_align(t), (2, 2));
        assert_eq!(ScalarKind::Int.size_align(t), (4, 4));
        assert_eq!(ScalarKind::Long.size_align(t), (4, 4));
        assert_eq!(ScalarKind::LongLong.size_align(t), (8, 4));
        assert_eq!(ScalarKind::Float.size_align(t), (4, 4));
        assert_eq!(ScalarKind::Double.size_align(t), (8, 4));
        assert_eq!(ScalarKind::LongDouble.size_align(t), (12, 4));
        assert_eq!(ScalarKind::Pointer.size_align(t), (4, 4));
    }

    #[test]
    fn test_scalar_size_align_x86_64() {
        let t = Target::X86_64;
        assert_eq!(ScalarKind::Bool.size_align(t), (1, 1));
        assert_eq!(ScalarKind::Char.size_align(t), (1, 1));
        assert_eq!(ScalarKind::Short.size_align(t), (2, 2));
        assert_eq!(ScalarKind::Int.size_align(t), (4, 4));
        assert_eq!(ScalarKind::Long.size_align(t), (8, 8));
        assert_eq!(ScalarKind::LongLong.size_align(t), (8, 8));
        assert_eq!(ScalarKind::Float.size_align(t), (4, 4));
        assert_eq!(ScalarKind::Double.size_align(t), (8, 8));
        assert_eq!(ScalarKind::LongDouble.size_align(t), (16, 16));
        assert_eq!(ScalarKind::Pointer.size_align(t), (8, 8));
    }

    #[test]
    fn test_triple() {
        assert_eq!(Target::I386.triple(), "i386-pc-linux-gnu");
        assert_eq!(Target::X86_64.triple(), "x86_64-pc-linux-gnu");
    }

    #[test]
    fn test_target_from_str() {
        assert_eq!(Target::from_str("i386").unwrap(), Target::I386);
        assert_eq!(Target::from_str("i686").unwrap(), Target::I386);
        assert_eq!(Target::from_str("x86_64").unwrap(), Target::X86_64);
        assert_eq!(Target::from_str("amd64").unwrap(), Target::X86_64);
        assert!(Target::from_str("arm").is_err());
    }

    #[test]
    fn test_scalar_classification() {
        assert!(ScalarKind::UInt.is_unsigned());
        assert!(!ScalarKind::Int.is_unsigned());
        assert!(ScalarKind::Float.is_float());
        assert!(!ScalarKind::Int.is_float());
        assert!(ScalarKind::Int.is_integer());
        assert!(!ScalarKind::Pointer.is_integer());
    }
}
