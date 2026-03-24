// ir/optimize/div_by_const.rs — Division-by-constant strength reduction.
//
// Replaces unsigned and signed integer divisions/modulos by compile-time
// constants with magic-number multiply + shift sequences. Only operates
// on 32-bit operands and only when compiling for a 64-bit target.
//
// References:
// - Hacker's Delight, Chapter 10 (Warren)
// - "Division by Invariant Integers using Multiplication" (Granlund & Montgomery)

use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::*;

/// Run division-by-constant on a function.
/// `is_64bit_target` should be true for x86_64.
pub fn div_by_const(func: &mut IrFunction, is_64bit_target: bool) -> bool {
    if !is_64bit_target {
        return false;
    }

    let mut changed = false;

    for bi in 0..func.blocks.len() {
        let mut ii = 0;
        while ii < func.blocks[bi].insts.len() {
            let inst = &func.blocks[bi].insts[ii];

            let replacement = match inst {
                Instruction::BinOp { result, op, lhs, rhs, ty } => {
                    // Only handle I32/U32 types.
                    if !matches!(ty, IrType::I32 | IrType::U32) {
                        None
                    } else if let Operand::Const(c) = rhs {
                        match (op, extract_u64(c)) {
                            (BinOpKind::UDiv, Some(d)) if d > 1 => {
                                try_udiv_by_const(*result, lhs.clone(), d, ty.clone(), func)
                            }
                            (BinOpKind::SDiv, Some(d)) if d > 1 && d < 0x80000000 => {
                                try_sdiv_by_const(*result, lhs.clone(), d as i64, ty.clone(), func)
                            }
                            (BinOpKind::URem, Some(d)) if d > 1 => {
                                try_urem_by_const(*result, lhs.clone(), d, ty.clone(), func)
                            }
                            (BinOpKind::SRem, Some(d)) if d > 1 && d < 0x80000000 => {
                                try_srem_by_const(*result, lhs.clone(), d as i64, ty.clone(), func)
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(new_insts) = replacement {
                // Replace the division with the sequence.
                func.blocks[bi].insts.splice(ii..=ii, new_insts.into_iter());
                changed = true;
                // Don't increment ii — we replaced in place, but we may have
                // inserted multiple instructions.
                ii += 1; // skip past the replacement
            } else {
                ii += 1;
            }
        }
    }

    changed
}

fn extract_u64(c: &ConstValue) -> Option<u64> {
    match c {
        ConstValue::I32(v) => Some(*v as u32 as u64),
        ConstValue::U32(v) => Some(*v as u64),
        ConstValue::I64(v) => Some(*v as u64),
        ConstValue::U64(v) => Some(*v),
        _ => None,
    }
}

/// Try to replace `result = lhs / d` (unsigned, 32-bit).
fn try_udiv_by_const(
    result: ValueId,
    lhs: Operand,
    d: u64,
    ty: IrType,
    func: &mut IrFunction,
) -> Option<Vec<Instruction>> {
    if d == 0 {
        return None;
    }

    // Power of 2: shift right.
    if d.is_power_of_two() {
        let shift = d.trailing_zeros();
        return Some(vec![Instruction::BinOp {
            result,
            op: BinOpKind::LShr,
            lhs,
            rhs: Operand::Const(ConstValue::I32(shift as i32)),
            ty,
        }]);
    }

    // Magic number multiplication.
    let (magic, shift) = compute_unsigned_magic_32(d as u32);

    let mut insts = Vec::new();

    let wide_lhs = func.alloc_value();
    insts.push(Instruction::Cast {
        result: wide_lhs,
        kind: CastKind::ZExt,
        src: lhs,
        src_ty: ty.clone(),
        dst_ty: IrType::I64,
    });

    let mul_result = func.alloc_value();
    insts.push(Instruction::BinOp {
        result: mul_result,
        op: BinOpKind::Mul,
        lhs: Operand::Value(wide_lhs),
        rhs: Operand::Const(ConstValue::I64(magic as i64)),
        ty: IrType::I64,
    });

    let shifted = func.alloc_value();
    insts.push(Instruction::BinOp {
        result: shifted,
        op: BinOpKind::LShr,
        lhs: Operand::Value(mul_result),
        rhs: Operand::Const(ConstValue::I64(32 + shift as i64)),
        ty: IrType::I64,
    });

    insts.push(Instruction::Cast {
        result,
        kind: CastKind::Trunc,
        src: Operand::Value(shifted),
        src_ty: IrType::I64,
        dst_ty: ty,
    });

    Some(insts)
}

/// Try to replace `result = lhs / d` (signed, 32-bit).
