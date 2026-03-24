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
fn narrow_comparisons(func: &mut IrFunction) -> bool {
    let mut changed = false;
    let defs = build_def_map(func);

    for bi in 0..func.blocks.len() {
        for ii in 0..func.blocks[bi].insts.len() {
            let inst = &func.blocks[bi].insts[ii];

            let (result, pred, lhs, rhs, ty) = match inst {
                Instruction::Icmp { result, pred, lhs, rhs, ty } => {
                    (*result, *pred, lhs.clone(), rhs.clone(), ty.clone())
                }
                _ => continue,
            };

            let (lhs_src, lhs_narrow_ty, lhs_cast) = match get_cast_source(&lhs, &defs) {
                Some(x) => x,
                None => continue,
            };
            let (rhs_src, rhs_narrow_ty, rhs_cast) = match get_cast_source(&rhs, &defs) {
                Some(x) => x,
                None => continue,
            };

            if lhs_narrow_ty != rhs_narrow_ty {
                continue;
            }
            if lhs_cast != rhs_cast {
                continue;
            }
            if type_bits(&lhs_narrow_ty) >= type_bits(&ty) {
                continue;
            }

            // For signed comparisons, need SExt; for unsigned, need ZExt.
            let is_signed_pred = matches!(pred, IcmpPred::Slt | IcmpPred::Sgt | IcmpPred::Sle | IcmpPred::Sge);
            if is_signed_pred && lhs_cast != CastKind::SExt {
                continue;
            }
            if !is_signed_pred && !matches!(pred, IcmpPred::Eq | IcmpPred::Ne) && lhs_cast != CastKind::ZExt {
                continue;
            }

            func.blocks[bi].insts[ii] = Instruction::Icmp {
                result,
                pred,
                lhs: lhs_src,
                rhs: rhs_src,
                ty: lhs_narrow_ty,
            };

            changed = true;
        }
    }

    changed
}

/// Phase 2: If a wide operation's result is only truncated, try narrowing.
fn narrow_truncated_ops(func: &mut IrFunction) -> bool {
    let mut changed = false;
    let defs = build_def_map(func);

    // Build use map: value -> list of (block, inst, operand_index).
    let uses = build_use_map(func);

    for bi in 0..func.blocks.len() {
        for ii in 0..func.blocks[bi].insts.len() {
            let inst = &func.blocks[bi].insts[ii];

            // Look for BinOp producing a wide result.
            let (result, op, lhs, rhs, ty) = match inst {
                Instruction::BinOp { result, op, lhs, rhs, ty } => {
                    (*result, *op, lhs.clone(), rhs.clone(), ty.clone())
                }
                _ => continue,
            };

            if !matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
                continue;
            }

            // Check if all uses are truncations to the same narrower type.
            let user_list = match uses.get(&result) {
                Some(u) => u,
                None => continue,
            };

            let mut trunc_ty: Option<IrType> = None;
            let mut all_trunc = true;

            for &(ubi, uii) in user_list {
                if ubi < func.blocks.len() && uii < func.blocks[ubi].insts.len() {
                    if let Instruction::Cast { kind: CastKind::Trunc, dst_ty, .. } = &func.blocks[ubi].insts[uii] {
                        match &trunc_ty {
                            None => trunc_ty = Some(dst_ty.clone()),
                            Some(prev) => {
                                if prev != dst_ty {
                                    all_trunc = false;
                                    break;
                                }
                            }
                        }
                    } else {
                        all_trunc = false;
                        break;
                    }
                }
            }

            if !all_trunc {
                continue;
            }
            let narrow_ty = match trunc_ty {
                Some(t) if type_bits(&t) < type_bits(&ty) => t,
                _ => continue,
            };

            // Only narrow safe operations (bitwise, add, sub, mul — they all
            // produce identical low bits regardless of width).
            if !matches!(op, BinOpKind::And | BinOpKind::Or | BinOpKind::Xor
                | BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul) {
                continue;
            }

            // Narrow the operands.
            let narrow_lhs = narrow_operand(&lhs, &defs, &narrow_ty, func, bi, ii);
            let narrow_rhs = narrow_operand(&rhs, &defs, &narrow_ty, func, bi, ii);

            if let (Some(nl), Some(nr)) = (narrow_lhs, narrow_rhs) {
                // Replace the binop with a narrow version.
                func.blocks[bi].insts[ii] = Instruction::BinOp {
                    result,
                    op,
                    lhs: nl,
                    rhs: nr,
                    ty: narrow_ty.clone(),
                };

                // Replace the truncations with copies.
                if let Some(user_list) = uses.get(&result) {
                    for &(ubi, uii) in user_list {
                        if ubi < func.blocks.len() && uii < func.blocks[ubi].insts.len() {
                            if let Instruction::Cast { result: tr, kind: CastKind::Trunc, .. } = func.blocks[ubi].insts[uii] {
                                func.blocks[ubi].insts[uii] = Instruction::Copy {
                                    result: tr,
                                    src: Operand::Value(result),
                                };
                            }
                        }
                    }
                }

                changed = true;
            }
        }
    }

    changed
}

fn build_def_map(func: &IrFunction) -> HashMap<ValueId, (usize, usize)> {
    let mut map = HashMap::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.insts.iter().enumerate() {
            if let Some(v) = inst.result() {
                map.insert(v, (bi, ii));
            }
        }
    }
    map
}

fn build_use_map(func: &IrFunction) -> HashMap<ValueId, Vec<(usize, usize)>> {
    let mut map: HashMap<ValueId, Vec<(usize, usize)>> = HashMap::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.insts.iter().enumerate() {
            inst.for_each_operand(|op| {
                if let Operand::Value(v) = op {
                    map.entry(*v).or_default().push((bi, ii));
                }
            });
        }
    }
    map
}

/// Get the source operand, narrow type, and cast kind if an operand
/// comes from a widening cast.
fn get_cast_source(
    op: &Operand,
    defs: &HashMap<ValueId, (usize, usize)>,
) -> Option<(Operand, IrType, CastKind)> {
    // This is a simplified version — in practice we'd need access to the func.
    // For now, return None and let other phases handle it.
    None
}

fn is_narrowable_op(op: BinOpKind) -> bool {
    matches!(
        op,
        BinOpKind::And | BinOpKind::Or | BinOpKind::Xor
            | BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul
    )
}

fn type_bits(ty: &IrType) -> u32 {
    match ty {
        IrType::I8 | IrType::U8 => 8,
        IrType::I16 | IrType::U16 => 16,
        IrType::I32 | IrType::U32 => 32,
        IrType::I64 | IrType::U64 => 64,
        IrType::I128 | IrType::U128 => 128,
        _ => 0,
    }
}

fn narrow_operand(
    op: &Operand,
    _defs: &HashMap<ValueId, (usize, usize)>,
    narrow_ty: &IrType,
    _func: &mut IrFunction,
    _bi: usize,
    _ii: usize,
) -> Option<Operand> {
    match op {
        Operand::Const(c) => {
            // Truncate constant to narrow type.
            Some(Operand::Const(truncate_to(c, narrow_ty)?))
        }
        Operand::Value(_) => {
            // Would need to insert a truncation. Skip for now.
            None
        }
        _ => None,
    }
}

fn truncate_to(c: &ConstValue, ty: &IrType) -> Option<ConstValue> {
    let v = match c {
        ConstValue::I32(v) => *v as i64,
        ConstValue::U32(v) => *v as i64,
        ConstValue::I64(v) => *v,
        ConstValue::U64(v) => *v as i64,
        _ => return None,
    };

    match ty {
        IrType::I8 => Some(ConstValue::I8(v as i8)),
        IrType::U8 => Some(ConstValue::U8(v as u8)),
        IrType::I16 => Some(ConstValue::I16(v as i16)),
        IrType::U16 => Some(ConstValue::U16(v as u16)),
        IrType::I32 => Some(ConstValue::I32(v as i32)),
        IrType::U32 => Some(ConstValue::U32(v as u32)),
        _ => None,
    }
}

