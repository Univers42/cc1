// ir/optimize/narrow.rs — Integer narrowing / type demotion.
//
// Eliminates unnecessary widening operations left over from C's integer
// promotion rules. Three phases:
//
// 1. Binary ops with cast: `(cast_up a) op (cast_up b)` → `cast_up (a op_narrow b)`
// 2. Binary ops without cast: if a 32-bit result is only used in a truncation,
//    try to replace with a narrower operation.
// 3. Comparison narrowing: `(zext a) cmp (zext b)` → `a cmp_narrow b`

use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::*;
use std::collections::HashMap;

/// Run the narrowing pass on a function.
pub fn narrow(func: &mut IrFunction) -> bool {
    let mut changed = false;

    changed |= narrow_binops_with_cast(func);
    changed |= narrow_comparisons(func);
    changed |= narrow_truncated_ops(func);

    changed
}

/// Phase 1: Binary operations where both operands are widened from the same type.
/// `(zext/sext a) op (zext/sext b)` → `zext/sext (a narrow_op b)`
fn narrow_binops_with_cast(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Build a map from ValueId to its defining instruction.
    let defs = build_def_map(func);

    for bi in 0..func.blocks.len() {
        for ii in 0..func.blocks[bi].insts.len() {
            let inst = &func.blocks[bi].insts[ii];

            let (result, op, lhs, rhs, ty) = match inst {
                Instruction::BinOp { result, op, lhs, rhs, ty } => {
                    (*result, *op, lhs.clone(), rhs.clone(), ty.clone())
                }
                _ => continue,
            };

            // Only handle integer binops that are safe to narrow.
            if !is_narrowable_op(op) {
                continue;
            }

            // Check if both operands are casts from the same narrow type.
            let (lhs_src, lhs_narrow_ty, lhs_cast) = match get_cast_source(&lhs, &defs) {
                Some(x) => x,
                None => continue,
            };
            let (rhs_src, rhs_narrow_ty, rhs_cast) = match get_cast_source(&rhs, &defs) {
                Some(x) => x,
                None => continue,
            };

            // Types must match and cast kinds must be compatible.
            if lhs_narrow_ty != rhs_narrow_ty {
                continue;
            }
            if lhs_cast != rhs_cast {
                continue;
            }

            // Must be widening (source type is smaller).
            if type_bits(&lhs_narrow_ty) >= type_bits(&ty) {
                continue;
            }

            // For overflow-sensitive ops (Add, Sub, Mul), only allow if
            // the cast is ZExt and the op is safe unsigned.
            if matches!(op, BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul) {
                // Be conservative here — only narrow bitwise ops.
                if !matches!(op, BinOpKind::And | BinOpKind::Or | BinOpKind::Xor) {
                    continue;
                }
            }

            // Create the narrow operation.
            let narrow_result = func.alloc_value();

            func.blocks[bi].insts[ii] = Instruction::Cast {
                result,
                kind: lhs_cast,
                src: Operand::Value(narrow_result),
                src_ty: lhs_narrow_ty.clone(),
                dst_ty: ty,
            };

            // Insert the narrow binop before the cast.
            func.blocks[bi].insts.insert(ii, Instruction::BinOp {
                result: narrow_result,
                op,
                lhs: lhs_src,
                rhs: rhs_src,
                ty: lhs_narrow_ty,
            });

            changed = true;
        }
    }

    changed
}

/// Phase 3: Comparison narrowing.
/// `(zext a) cmp (zext b)` → `a cmp b`
