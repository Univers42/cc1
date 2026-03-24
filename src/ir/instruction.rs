// ir/instruction.rs — SSA instruction and terminator definitions.
//
// Every instruction produces at most one value (identified by `result: ValueId`).
// Instructions live in basic blocks; the last item in each block is a terminator.

use crate::ir::types::*;

/// An SSA instruction in a basic block.
#[derive(Debug, Clone)]
pub enum Instruction {
    /// Stack allocation: `result = alloca ty, align N`
    Alloca {
        result: ValueId,
        ty: IrType,
        align: u32,
    },

    /// Dynamic (VLA) stack allocation: `result = dynalloca ty, count`
    DynAlloca {
        result: ValueId,
        ty: IrType,
        count: Operand,
    },

    /// Store value to memory: `store ty value, ptr addr`
    Store {
        addr: Operand,
        value: Operand,
        ty: IrType,
    },

    /// Load value from memory: `result = load ty, ptr addr`
    Load {
        result: ValueId,
        addr: Operand,
        ty: IrType,
    },

    /// Binary operation: `result = op ty lhs, rhs`
    BinOp {
        result: ValueId,
        op: BinOpKind,
        lhs: Operand,
        rhs: Operand,
        ty: IrType,
    },

    /// Unary operation: `result = op ty operand`
    UnaryOp {
        result: ValueId,
        op: UnaryOpKind,
        operand: Operand,
        ty: IrType,
    },

    /// Integer comparison: `result = icmp pred ty lhs, rhs`
    Icmp {
        result: ValueId,
        pred: IcmpPred,
        lhs: Operand,
        rhs: Operand,
        ty: IrType,
    },

    /// Float comparison: `result = fcmp pred ty lhs, rhs`
    Fcmp {
        result: ValueId,
        pred: FcmpPred,
        lhs: Operand,
        rhs: Operand,
        ty: IrType,
    },

    /// Type cast: `result = cast_kind src_ty operand to dst_ty`
    Cast {
        result: ValueId,
        kind: CastKind,
        src: Operand,
        src_ty: IrType,
        dst_ty: IrType,
    },

    /// Direct function call: `result = call ret_ty @callee(args...)`
    Call {
        result: ValueId,
        callee: String,
        args: Vec<(Operand, IrType)>,
        ret_ty: IrType,
        is_variadic: bool,
    },

    /// Indirect call through function pointer: `result = call ret_ty operand(args...)`
    CallIndirect {
        result: ValueId,
        func_ptr: Operand,
        args: Vec<(Operand, IrType)>,
        ret_ty: IrType,
        is_variadic: bool,
    },

    /// GEP: `result = getelementptr elem_ty, ptr base, offset`
    GetElementPtr {
        result: ValueId,
        base: Operand,
        offset: Operand,
        elem_ty: IrType,
    },

    /// Materialize a global's address: `result = globaladdr @name`
    GlobalAddr {
        result: ValueId,
        name: String,
    },

    /// Label address (for computed goto): `result = labeladdr bb`
    LabelAddr {
        result: ValueId,
        block: BlockId,
    },

    /// Conditional select: `result = select cond, ty true_val, false_val`
    Select {
        result: ValueId,
        cond: Operand,
        true_val: Operand,
        false_val: Operand,
        ty: IrType,
    },

    /// SSA copy (from phi elimination): `result = copy src`
    Copy {
        result: ValueId,
        src: Operand,
    },

    /// Phi node: `result = phi ty [val1, bb1], [val2, bb2], ...`
    Phi {
        result: ValueId,
        ty: IrType,
        incoming: Vec<(BlockId, Operand)>,
    },

    /// Atomic load: `result = atomic_load ty, ptr addr, ordering`
    AtomicLoad {
        result: ValueId,
        addr: Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    },

    /// Atomic store: `atomic_store ty value, ptr addr, ordering`
    AtomicStore {
        addr: Operand,
        value: Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    },

    /// Atomic read-modify-write: `result = atomicrmw op ty, ptr addr, value, ordering`
    AtomicRmw {
        result: ValueId,
        op: AtomicRmwOp,
        addr: Operand,
        value: Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    },

    /// Atomic compare-and-exchange: `result = cmpxchg ptr addr, ty expected, desired, ordering`
    AtomicCmpxchg {
        result: ValueId,
        addr: Operand,
        expected: Operand,
        desired: Operand,
        ty: IrType,
        success_ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
    },

    /// Restore the stack pointer (after dynamic alloca).
    StackRestore {
        saved_sp: Operand,
    },

    /// Inline assembly block.
    InlineAsm {
        result: ValueId,
        template: String,
        constraints: String,
        operands: Vec<(Operand, IrType)>,
        has_side_effects: bool,
        align_stack: bool,
    },

    /// No-op placeholder (dead instructions get replaced with this).
    Nop,
}

/// A basic block terminator — the last instruction in a block.
#[derive(Debug, Clone)]
pub enum Terminator {
    /// Return from function: `ret ty value` or `ret void`.
    Ret {
        value: Option<Operand>,
    },

    /// Unconditional branch: `br bb`.
    Br {
        target: BlockId,
    },

    /// Conditional branch: `br cond, true_bb, false_bb`.
    CondBr {
        cond: Operand,
        true_bb: BlockId,
        false_bb: BlockId,
    },

    /// Switch: `switch ty discr, default_bb [val1: bb1, val2: bb2, ...]`.
    Switch {
        discr: Operand,
        ty: IrType,
        default: BlockId,
        cases: Vec<(i64, BlockId)>,
    },

    /// Indirect branch (computed goto): `indirectbr addr, [bb1, bb2, ...]`.
    IndirectBr {
        addr: Operand,
        targets: Vec<BlockId>,
    },

    /// Unreachable code (after noreturn calls, etc.).
    Unreachable,
}

impl Instruction {
    /// Returns the result ValueId produced by this instruction, if any.
    pub fn result(&self) -> Option<ValueId> {
        match self {
            Instruction::Alloca { result, .. }
            | Instruction::DynAlloca { result, .. }
            | Instruction::Load { result, .. }
            | Instruction::BinOp { result, .. }
            | Instruction::UnaryOp { result, .. }
            | Instruction::Icmp { result, .. }
            | Instruction::Fcmp { result, .. }
            | Instruction::Cast { result, .. }
            | Instruction::Call { result, .. }
            | Instruction::CallIndirect { result, .. }
            | Instruction::GetElementPtr { result, .. }
            | Instruction::GlobalAddr { result, .. }
            | Instruction::LabelAddr { result, .. }
            | Instruction::Select { result, .. }
            | Instruction::Copy { result, .. }
            | Instruction::Phi { result, .. }
            | Instruction::AtomicLoad { result, .. }
            | Instruction::AtomicRmw { result, .. }
            | Instruction::AtomicCmpxchg { result, .. }
            | Instruction::InlineAsm { result, .. } => Some(*result),

            Instruction::Store { .. }
            | Instruction::AtomicStore { .. }
            | Instruction::StackRestore { .. }
            | Instruction::Nop => None,
        }
    }

    /// Returns the result type of this instruction.
    pub fn result_type(&self) -> IrType {
        match self {
            Instruction::Alloca { .. } => IrType::Ptr,
            Instruction::DynAlloca { .. } => IrType::Ptr,
            Instruction::Load { ty, .. } => ty.clone(),
            Instruction::BinOp { ty, .. } => ty.clone(),
            Instruction::UnaryOp { ty, .. } => ty.clone(),
            Instruction::Icmp { .. } | Instruction::Fcmp { .. } => IrType::I8,
            Instruction::Cast { dst_ty, .. } => dst_ty.clone(),
            Instruction::Call { ret_ty, .. } => ret_ty.clone(),
            Instruction::CallIndirect { ret_ty, .. } => ret_ty.clone(),
            Instruction::GetElementPtr { .. } => IrType::Ptr,
            Instruction::GlobalAddr { .. } => IrType::Ptr,
            Instruction::LabelAddr { .. } => IrType::Ptr,
            Instruction::Select { ty, .. } => ty.clone(),
            Instruction::Copy { .. } => IrType::I64, // best-effort
            Instruction::Phi { ty, .. } => ty.clone(),
            Instruction::AtomicLoad { ty, .. } => ty.clone(),
            Instruction::AtomicRmw { ty, .. } => ty.clone(),
            Instruction::AtomicCmpxchg { ty, .. } => ty.clone(),
            Instruction::InlineAsm { .. } => IrType::Void,
            Instruction::Store { .. }
            | Instruction::AtomicStore { .. }
            | Instruction::StackRestore { .. }
            | Instruction::Nop => IrType::Void,
        }
    }

    /// Iterate over operand values referenced by this instruction.
    pub fn for_each_operand<F: FnMut(&Operand)>(&self, mut f: F) {
        match self {
            Instruction::Alloca { .. } | Instruction::Nop => {}
            Instruction::DynAlloca { count, .. } => f(count),
            Instruction::Store { addr, value, .. } => {
                f(addr);
                f(value);
            }
            Instruction::Load { addr, .. } => f(addr),
            Instruction::BinOp { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Instruction::UnaryOp { operand, .. } => f(operand),
            Instruction::Icmp { lhs, rhs, .. } | Instruction::Fcmp { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Instruction::Cast { src, .. } => f(src),
            Instruction::Call { args, .. } => {
                for (op, _) in args {
                    f(op);
                }
            }
            Instruction::CallIndirect {
                func_ptr, args, ..
            } => {
                f(func_ptr);
                for (op, _) in args {
                    f(op);
                }
            }
            Instruction::GetElementPtr { base, offset, .. } => {
                f(base);
                f(offset);
            }
            Instruction::GlobalAddr { .. } | Instruction::LabelAddr { .. } => {}
            Instruction::Select {
                cond,
                true_val,
                false_val,
                ..
            } => {
                f(cond);
                f(true_val);
                f(false_val);
            }
            Instruction::Copy { src, .. } => f(src),
            Instruction::Phi { incoming, .. } => {
                for (_, op) in incoming {
                    f(op);
                }
            }
            Instruction::AtomicLoad { addr, .. } => f(addr),
            Instruction::AtomicStore { addr, value, .. } => {
                f(addr);
                f(value);
            }
            Instruction::AtomicRmw {
                addr, value, ..
            } => {
                f(addr);
                f(value);
            }
            Instruction::AtomicCmpxchg {
                addr,
                expected,
                desired,
                ..
            } => {
                f(addr);
                f(expected);
                f(desired);
            }
            Instruction::StackRestore { saved_sp } => f(saved_sp),
            Instruction::InlineAsm { operands, .. } => {
                for (op, _) in operands {
                    f(op);
                }
            }
        }
    }

    /// Iterate over value uses (only Operand::Value) referenced by this instruction.
    pub fn for_each_value_use<F: FnMut(ValueId)>(&self, mut f: F) {
        self.for_each_operand(|op| {
            if let Operand::Value(v) = op {
                f(*v);
            }
        });
    }

    /// Iterate over operands mutably, allowing in-place replacement.
    pub fn for_each_operand_mut<F: FnMut(&mut Operand)>(&mut self, mut f: F) {
        match self {
            Instruction::Alloca { .. } | Instruction::Nop => {}
            Instruction::DynAlloca { count, .. } => f(count),
            Instruction::Store { addr, value, .. } => {
                f(addr);
                f(value);
            }
            Instruction::Load { addr, .. } => f(addr),
            Instruction::BinOp { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Instruction::UnaryOp { operand, .. } => f(operand),
            Instruction::Icmp { lhs, rhs, .. } | Instruction::Fcmp { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Instruction::Cast { src, .. } => f(src),
            Instruction::Call { args, .. } => {
                for (op, _) in args {
                    f(op);
                }
            }
            Instruction::CallIndirect {
                func_ptr, args, ..
            } => {
                f(func_ptr);
                for (op, _) in args {
                    f(op);
                }
            }
            Instruction::GetElementPtr { base, offset, .. } => {
                f(base);
                f(offset);
            }
            Instruction::GlobalAddr { .. } | Instruction::LabelAddr { .. } => {}
            Instruction::Select {
                cond,
                true_val,
                false_val,
                ..
            } => {
                f(cond);
                f(true_val);
                f(false_val);
            }
            Instruction::Copy { src, .. } => f(src),
            Instruction::Phi { incoming, .. } => {
                for (_, op) in incoming {
                    f(op);
                }
            }
            Instruction::AtomicLoad { addr, .. } => f(addr),
            Instruction::AtomicStore { addr, value, .. } => {
                f(addr);
                f(value);
            }
            Instruction::AtomicRmw { addr, value, .. } => {
                f(addr);
                f(value);
            }
            Instruction::AtomicCmpxchg {
                addr,
                expected,
                desired,
                ..
            } => {
                f(addr);
                f(expected);
                f(desired);
            }
            Instruction::StackRestore { saved_sp } => f(saved_sp),
            Instruction::InlineAsm { operands, .. } => {
                for (op, _) in operands {
                    f(op);
                }
            }
        }
    }

    /// Returns true if this instruction has side effects and should never be deleted by DCE.
    pub fn has_side_effects(&self) -> bool {
        matches!(
            self,
            Instruction::Store { .. }
                | Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::Alloca { .. }
                | Instruction::DynAlloca { .. }
                | Instruction::AtomicStore { .. }
                | Instruction::AtomicRmw { .. }
                | Instruction::AtomicCmpxchg { .. }
                | Instruction::StackRestore { .. }
                | Instruction::InlineAsm { .. }
        )
    }

    /// Returns true if this instruction is pure (no side effects, safe to hoist/CSE).
    pub fn is_pure(&self) -> bool {
        matches!(
            self,
            Instruction::BinOp { .. }
                | Instruction::UnaryOp { .. }
                | Instruction::Icmp { .. }
                | Instruction::Fcmp { .. }
                | Instruction::Cast { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::GlobalAddr { .. }
                | Instruction::Copy { .. }
                | Instruction::Select { .. }
        )
    }
    }
}

impl Terminator {
    /// Iterate over operands in this terminator.
    pub fn for_each_operand<F: FnMut(&Operand)>(&self, mut f: F) {
        match self {
            Terminator::Ret { value: Some(v) } => f(v),
            Terminator::Ret { value: None } | Terminator::Unreachable => {}
            Terminator::Br { .. } => {}
            Terminator::CondBr { cond, .. } => f(cond),
            Terminator::Switch { discr, .. } => f(discr),
            Terminator::IndirectBr { addr, .. } => f(addr),
        }
    }

    /// Get the successor block IDs of this terminator.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Terminator::Ret { .. } | Terminator::Unreachable => vec![],
            Terminator::Br { target } => vec![*target],
            Terminator::CondBr {
                true_bb, false_bb, ..
            } => vec![*true_bb, *false_bb],
            Terminator::Switch {
                default, cases, ..
            } => {
                let mut succs = vec![*default];
                for (_, bb) in cases {
                    succs.push(*bb);
                }
                succs
            }
            Terminator::IndirectBr { targets, .. } => targets.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instruction_result() {
        let inst = Instruction::BinOp {
            result: ValueId(0),
            op: BinOpKind::Add,
            lhs: Operand::Value(ValueId(1)),
            rhs: Operand::Value(ValueId(2)),
            ty: IrType::I32,
        };
        assert_eq!(inst.result(), Some(ValueId(0)));

        let store = Instruction::Store {
            addr: Operand::Value(ValueId(0)),
            value: Operand::Const(ConstValue::I32(42)),
            ty: IrType::I32,
        };
        assert_eq!(store.result(), None);
    }

    #[test]
    fn test_terminator_successors() {
        let br = Terminator::Br {
            target: BlockId(1),
        };
        assert_eq!(br.successors(), vec![BlockId(1)]);

        let cond = Terminator::CondBr {
            cond: Operand::Value(ValueId(0)),
            true_bb: BlockId(1),
            false_bb: BlockId(2),
        };
        assert_eq!(cond.successors(), vec![BlockId(1), BlockId(2)]);

        let ret = Terminator::Ret { value: None };
        assert!(ret.successors().is_empty());
    }

    #[test]
    fn test_switch_successors() {
        let sw = Terminator::Switch {
            discr: Operand::Value(ValueId(0)),
            ty: IrType::I32,
            default: BlockId(3),
            cases: vec![(0, BlockId(0)), (1, BlockId(1)), (2, BlockId(2))],
        };
        assert_eq!(
            sw.successors(),
            vec![BlockId(3), BlockId(0), BlockId(1), BlockId(2)]
        );
    }

    #[test]
    fn test_for_each_operand() {
        let inst = Instruction::BinOp {
            result: ValueId(3),
            op: BinOpKind::Add,
            lhs: Operand::Value(ValueId(1)),
            rhs: Operand::Const(ConstValue::I32(5)),
            ty: IrType::I32,
        };
        let mut ops = Vec::new();
        inst.for_each_operand(|op| ops.push(op.clone()));
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0], Operand::Value(ValueId(1)));
        assert_eq!(ops[1], Operand::Const(ConstValue::I32(5)));
    }

    #[test]
    fn test_for_each_value_use() {
        let inst = Instruction::BinOp {
            result: ValueId(3),
            op: BinOpKind::Mul,
            lhs: Operand::Value(ValueId(1)),
            rhs: Operand::Value(ValueId(2)),
            ty: IrType::I32,
        };
        let mut values = Vec::new();
        inst.for_each_value_use(|v| values.push(v));
        assert_eq!(values, vec![ValueId(1), ValueId(2)]);
    }

    #[test]
    fn test_alloca_result_type() {
        let alloca = Instruction::Alloca {
            result: ValueId(0),
            ty: IrType::I32,
            align: 4,
        };
        assert_eq!(alloca.result_type(), IrType::Ptr);
    }
}
