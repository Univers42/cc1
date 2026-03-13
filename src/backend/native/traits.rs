// backend/native/traits.rs — ArchCodegen trait definition.
//
// Each architecture (x86-64, i686, AArch64, RISC-V 64) implements this trait.
// The generation driver calls these methods to produce assembly text.

#[allow(unused_imports)]
use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::{BasicBlock, IrFunction, IrModule};
use crate::ir::types::*;
use crate::backend::native::state::CodegenState;

/// Architecture-specific code generation interface.
///
/// The generation driver walks the IR and calls these methods in order.
/// Each method appends assembly text to `state.asm`.
pub trait ArchCodegen {
    // ── Module lifecycle ────────────────────────────────────────────

    /// Emit the assembly file header (.file, .text directives, etc.).
    fn emit_file_header(&self, state: &mut CodegenState, module: &IrModule);

    /// Emit the assembly file footer.
    fn emit_file_footer(&self, state: &mut CodegenState, module: &IrModule);

    // ── Data sections ───────────────────────────────────────────────

    /// Emit a global variable definition.
    fn emit_global(&self, state: &mut CodegenState, name: &str, ty: &IrType,
                   init: &crate::ir::module::GlobalInit, align: u32, linkage: Linkage);

    /// Emit a string literal in .rodata.
    fn emit_string_literal(&self, state: &mut CodegenState, label: &str, bytes: &[u8]);

    /// Emit a BSS (zero-initialized) variable.
    fn emit_bss(&self, state: &mut CodegenState, name: &str, size: u64, align: u32, linkage: Linkage);

    // ── Function lifecycle ──────────────────────────────────────────

    /// Emit the function prologue (label, .globl, frame setup, callee-saved regs).
    fn emit_function_prologue(&self, state: &mut CodegenState, func: &IrFunction);

    /// Emit the function epilogue (callee-saved restore, leave/ret).
    fn emit_function_epilogue(&self, state: &mut CodegenState, func: &IrFunction);

    /// Compute the stack layout for a function (allocas, spills, alignment).
    fn compute_stack_layout(&self, state: &mut CodegenState, func: &IrFunction);

    // ── Basic block lifecycle ───────────────────────────────────────

    /// Emit a basic block label.
    fn emit_block_label(&self, state: &mut CodegenState, func: &IrFunction, block: &BasicBlock);

    // ── Instructions ────────────────────────────────────────────────

    /// Emit code for an alloca instruction.
    fn emit_alloca(&self, state: &mut CodegenState, result: ValueId, ty: &IrType, align: u32);

    /// Emit code for a store instruction.
    fn emit_store(&self, state: &mut CodegenState, addr: &Operand, value: &Operand, ty: &IrType);

    /// Emit code for a load instruction.
    fn emit_load(&self, state: &mut CodegenState, result: ValueId, addr: &Operand, ty: &IrType);

    /// Emit code for a binary operation.
    fn emit_binop(&self, state: &mut CodegenState, result: ValueId, op: BinOpKind,
                  lhs: &Operand, rhs: &Operand, ty: &IrType);

    /// Emit code for a unary operation.
    fn emit_unaryop(&self, state: &mut CodegenState, result: ValueId, op: UnaryOpKind,
                    operand: &Operand, ty: &IrType);

    /// Emit code for an integer comparison.
    fn emit_icmp(&self, state: &mut CodegenState, result: ValueId, pred: IcmpPred,
                 lhs: &Operand, rhs: &Operand, ty: &IrType);

    /// Emit code for a floating-point comparison.
    fn emit_fcmp(&self, state: &mut CodegenState, result: ValueId, pred: FcmpPred,
                 lhs: &Operand, rhs: &Operand, ty: &IrType);

    /// Emit code for a type cast.
    fn emit_cast(&self, state: &mut CodegenState, result: ValueId, kind: CastKind,
                 src: &Operand, src_ty: &IrType, dst_ty: &IrType);

    /// Emit code for a direct function call.
    fn emit_call(&self, state: &mut CodegenState, result: ValueId, callee: &str,
                 args: &[(Operand, IrType)], ret_ty: &IrType, is_variadic: bool);

    /// Emit code for an indirect function call.
    fn emit_call_indirect(&self, state: &mut CodegenState, result: ValueId,
                          func_ptr: &Operand, args: &[(Operand, IrType)],
                          ret_ty: &IrType, is_variadic: bool);

    /// Emit code for a GEP (get element pointer).
    fn emit_gep(&self, state: &mut CodegenState, result: ValueId,
                base: &Operand, offset: &Operand, elem_ty: &IrType);

    /// Emit code for materializing a global address.
    fn emit_global_addr(&self, state: &mut CodegenState, result: ValueId, name: &str);

    /// Emit code for a select (conditional move).
    fn emit_select(&self, state: &mut CodegenState, result: ValueId,
                   cond: &Operand, true_val: &Operand, false_val: &Operand, ty: &IrType);

    /// Emit code for a phi node (parallel copy).
    fn emit_phi(&self, state: &mut CodegenState, result: ValueId, ty: &IrType,
                incoming: &[(BlockId, Operand)]);

    // ── Terminators ─────────────────────────────────────────────────

    /// Emit code for a return instruction.
    fn emit_ret(&self, state: &mut CodegenState, value: &Option<Operand>);

    /// Emit code for an unconditional branch.
    fn emit_br(&self, state: &mut CodegenState, target: BlockId);

    /// Emit code for a conditional branch.
    fn emit_cond_br(&self, state: &mut CodegenState, cond: &Operand,
                    true_bb: BlockId, false_bb: BlockId);

    /// Emit code for a switch instruction.
    fn emit_switch(&self, state: &mut CodegenState, discr: &Operand, ty: &IrType,
                   default: BlockId, cases: &[(i64, BlockId)]);

    // ── Helpers ─────────────────────────────────────────────────────

    /// Get the block label string for a given BlockId.
    fn block_label(&self, _func: &IrFunction, bid: BlockId) -> String {
        format!(".LBB_{}", bid.0)
    }

    /// Get the assembly name for a register holding the given value.
    fn value_location(&self, state: &CodegenState, vid: ValueId) -> String;

    /// Move an operand into a register and return the register name.
    fn materialize_operand(&self, state: &mut CodegenState, op: &Operand, ty: &IrType) -> String;
}
