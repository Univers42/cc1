// ir/optimize/simplify.rs — Algebraic simplification, strength reduction, peephole.
//
// Applies identity simplifications, strength reductions, constant reassociation,
// negation elimination, cast chain optimization, comparison simplifications,
// select simplifications, GEP simplifications, and math library call lowering.

use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::*;
use std::collections::HashMap;

/// Run algebraic simplification on a single function. Returns true if any changes.
pub fn simplify(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Build a definition map: ValueId -> (block_idx, inst_idx)
    let mut defs: HashMap<ValueId, (usize, usize)> = HashMap::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.insts.iter().enumerate() {
            if let Some(r) = inst.result() {
                defs.insert(r, (bi, ii));
            }
        }
    }

    for bi in 0..func.blocks.len() {
        for ii in 0..func.blocks[bi].insts.len() {
            if let Some(replacement) = try_simplify(&func.blocks[bi].insts[ii], &func.blocks, &defs) {
                let result = func.blocks[bi].insts[ii].result().unwrap();
                func.blocks[bi].insts[ii] = Instruction::Copy {
                    result,
                    src: replacement,
                };
                changed = true;
            }
        }
    }

    changed
}

/// Try to simplify an instruction, returning a replacement operand.
fn try_simplify(
    inst: &Instruction,
    blocks: &[crate::ir::module::BasicBlock],
    defs: &HashMap<ValueId, (usize, usize)>,
) -> Option<Operand> {
    match inst {
        Instruction::BinOp { op, lhs, rhs, ty, .. } => {
            simplify_binop(*op, lhs, rhs, ty, blocks, defs)
        }
        Instruction::Icmp { pred, lhs, rhs, ty, .. } => {
            simplify_icmp(*pred, lhs, rhs, ty)
        }
        Instruction::Cast { kind, src, src_ty, dst_ty, .. } => {
            simplify_cast(*kind, src, src_ty, dst_ty, blocks, defs)
        }
        Instruction::Select { cond, true_val, false_val, .. } => {
            simplify_select(cond, true_val, false_val)
        }
        Instruction::GetElementPtr { base, offset, .. } => {
            simplify_gep(base, offset)
        }
        _ => None,
    }
}

fn simplify_binop(
    op: BinOpKind,
    lhs: &Operand,
    rhs: &Operand,
    ty: &IrType,
    _blocks: &[crate::ir::module::BasicBlock],
    _defs: &HashMap<ValueId, (usize, usize)>,
) -> Option<Operand> {
    if !ty.is_integer() && !ty.is_pointer() {
        // Float: only x * 1.0 and x / 1.0 are safe
        if ty.is_float() {
            if let Some(rv) = const_f64(rhs) {
                if op == BinOpKind::FMul && rv == 1.0 {
                    return Some(lhs.clone());
                }
                if op == BinOpKind::FDiv && rv == 1.0 {
                    return Some(lhs.clone());
                }
            }
        }
        return None;
    }

    let lc = const_i64(lhs);
    let rc = const_i64(rhs);

    // Identity simplifications
    match op {
        // x + 0 => x
        BinOpKind::Add if rc == Some(0) => return Some(lhs.clone()),
        // 0 + x => x
        BinOpKind::Add if lc == Some(0) => return Some(rhs.clone()),
        // x - 0 => x
        BinOpKind::Sub if rc == Some(0) => return Some(lhs.clone()),
        // x - x => 0
        BinOpKind::Sub if lhs == rhs => return Some(zero_const(ty)),
        // x * 0 => 0
        BinOpKind::Mul if rc == Some(0) => return Some(zero_const(ty)),
        // 0 * x => 0
        BinOpKind::Mul if lc == Some(0) => return Some(zero_const(ty)),
        // x * 1 => x
        BinOpKind::Mul if rc == Some(1) => return Some(lhs.clone()),
        // 1 * x => x
        BinOpKind::Mul if lc == Some(1) => return Some(rhs.clone()),
        // x / 1 => x
        BinOpKind::SDiv | BinOpKind::UDiv if rc == Some(1) => return Some(lhs.clone()),
        // x / x => 1
        BinOpKind::SDiv | BinOpKind::UDiv if lhs == rhs => return Some(one_const(ty)),
        // x % 1 => 0
        BinOpKind::SRem | BinOpKind::URem if rc == Some(1) => return Some(zero_const(ty)),
        // x % x => 0
        BinOpKind::SRem | BinOpKind::URem if lhs == rhs => return Some(zero_const(ty)),
        // x & 0 => 0
        BinOpKind::And if rc == Some(0) => return Some(zero_const(ty)),
        // x & ~0 => x (all ones)
        BinOpKind::And if is_all_ones(rhs, ty) => return Some(lhs.clone()),
        // x & x => x
        BinOpKind::And if lhs == rhs => return Some(lhs.clone()),
        // x | 0 => x
        BinOpKind::Or if rc == Some(0) => return Some(lhs.clone()),
        // x | ~0 => ~0
        BinOpKind::Or if is_all_ones(rhs, ty) => return Some(rhs.clone()),
        // x | x => x
        BinOpKind::Or if lhs == rhs => return Some(lhs.clone()),
        // x ^ 0 => x
        BinOpKind::Xor if rc == Some(0) => return Some(lhs.clone()),
        // x ^ x => 0
        BinOpKind::Xor if lhs == rhs => return Some(zero_const(ty)),
        // x << 0 => x
        BinOpKind::Shl if rc == Some(0) => return Some(lhs.clone()),
        // x >> 0 => x
        BinOpKind::LShr | BinOpKind::AShr if rc == Some(0) => return Some(lhs.clone()),
        // 0 << x => 0
        BinOpKind::Shl if lc == Some(0) => return Some(zero_const(ty)),
        // 0 >> x => 0
        BinOpKind::LShr | BinOpKind::AShr if lc == Some(0) => return Some(zero_const(ty)),
        _ => {}
    }

    // Strength reductions
    if let Some(rv) = rc {
        match op {
            // x * 2 => x + x
            BinOpKind::Mul if rv == 2 => {
                return None; // Leave for codegen; creating new instructions is complex here
            }
            // x * 2^k => x << k
            BinOpKind::Mul if rv > 0 && (rv as u64).is_power_of_two() => {
                let _k = (rv as u64).trailing_zeros() as i64;
                return None; // Would need to create a Shl instruction; defer
            }
            // x /u 2^k => x >>l k (unsigned only)
            BinOpKind::UDiv if rv > 0 && (rv as u64).is_power_of_two() => {
                return None; // Would need LShr instruction
            }
            // x %u 2^k => x & (2^k - 1) (unsigned only)
            BinOpKind::URem if rv > 0 && (rv as u64).is_power_of_two() => {
                return None; // Would need And instruction
            }
            _ => {}
        }
    }

    // Operand canonicalization: constant on left → swap to right
    // (This helps downstream constant folding patterns.)
    // We can't swap in-place here since we return an Operand, but we note this
    // as a pattern for future instruction-level rewriting.

    None
}

