// ir/optimize/constant_fold.rs — Constant expression evaluation at compile time.
//
// Evaluates operations whose operands are all compile-time constants,
// replacing the instruction with the computed result. Runs to a fixpoint.

use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::IrFunction;
use crate::ir::types::*;
use std::collections::HashMap;

/// Run constant folding on a single function. Returns true if any changes were made.
pub fn constant_fold(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Iterate to fixpoint: after folding, new constants may enable more folding.
    loop {
        let mut iteration_changed = false;

        // Build constant map: ValueId -> ConstValue
        let mut const_map: HashMap<ValueId, ConstValue> = HashMap::new();

        // First, collect all Copy instructions with constant sources.
        for block in &func.blocks {
            for inst in &block.insts {
                match inst {
                    Instruction::Copy { result, src: Operand::Const(c) } => {
                        const_map.insert(*result, c.clone());
                    }
                    _ => {}
                }
            }
        }

        // Try to fold each instruction.
        for bi in 0..func.blocks.len() {
            for ii in 0..func.blocks[bi].insts.len() {
                let inst = &func.blocks[bi].insts[ii];
                if let Some(result) = inst.result() {
                    if let Some(folded) = try_fold(inst, &const_map) {
                        const_map.insert(result, folded.clone());
                        func.blocks[bi].insts[ii] = Instruction::Copy {
                            result,
                            src: Operand::Const(folded),
                        };
                        iteration_changed = true;
                    }
                }
            }

            // Also fold terminator conditions.
            let term = &func.blocks[bi].terminator;
            if let Some(new_term) = try_fold_terminator(term, &const_map) {
                func.blocks[bi].terminator = new_term;
                iteration_changed = true;
            }
        }

        if iteration_changed {
            changed = true;
        } else {
            break;
        }
    }

    changed
}

/// Resolve an operand to a constant if possible.
fn resolve_const(op: &Operand, consts: &HashMap<ValueId, ConstValue>) -> Option<ConstValue> {
    match op {
        Operand::Const(c) => Some(c.clone()),
        Operand::Value(v) => consts.get(v).cloned(),
        _ => None,
    }
}

/// Try to fold an instruction to a constant.
fn try_fold(inst: &Instruction, consts: &HashMap<ValueId, ConstValue>) -> Option<ConstValue> {
    match inst {
        Instruction::BinOp { op, lhs, rhs, ty, .. } => {
            let l = resolve_const(lhs, consts)?;
            let r = resolve_const(rhs, consts)?;
            fold_binop(*op, &l, &r, ty)
        }

        Instruction::UnaryOp { op, operand, ty, .. } => {
            let v = resolve_const(operand, consts)?;
            fold_unaryop(*op, &v, ty)
        }

        Instruction::Icmp { pred, lhs, rhs, ty, .. } => {
            let l = resolve_const(lhs, consts)?;
            let r = resolve_const(rhs, consts)?;
            fold_icmp(*pred, &l, &r, ty)
        }

        Instruction::Fcmp { pred, lhs, rhs, .. } => {
            let l = resolve_const(lhs, consts)?;
            let r = resolve_const(rhs, consts)?;
            fold_fcmp(*pred, &l, &r)
        }

        Instruction::Cast { kind, src, src_ty, dst_ty, .. } => {
            let v = resolve_const(src, consts)?;
            fold_cast(*kind, &v, src_ty, dst_ty)
        }

        Instruction::Select { cond, true_val, false_val, .. } => {
            // Both arms same?
            let tv = resolve_const(true_val, consts);
            let fv = resolve_const(false_val, consts);
            if tv.is_some() && tv == fv {
                return tv;
            }
            // Constant condition?
            let c = resolve_const(cond, consts)?;
            let cond_val = const_to_i64(&c)?;
            if cond_val != 0 {
                resolve_const(true_val, consts).or_else(|| {
                    // Can't fold if true_val isn't const
                    None
                })
            } else {
                resolve_const(false_val, consts).or_else(|| None)
            }
        }

        Instruction::GetElementPtr { base, offset, elem_ty, .. } => {
            // GEP with constant base (null) and constant offset.
            let _b = resolve_const(base, consts)?;
            let _o = resolve_const(offset, consts)?;
            // Only fold GEP(null, 0) = null
            None // GEP folding is complex, skip for now
        }

        _ => None,
    }
}

/// Try to fold a terminator (constant condBr → unconditional br).
fn try_fold_terminator(term: &Terminator, consts: &HashMap<ValueId, ConstValue>) -> Option<Terminator> {
    match term {
        Terminator::CondBr { cond, true_bb, false_bb, .. } => {
            let c = resolve_const(cond, consts)?;
            let val = const_to_i64(&c)?;
            if val != 0 {
                Some(Terminator::Br { target: *true_bb })
            } else {
                Some(Terminator::Br { target: *false_bb })
            }
        }

        Terminator::Switch { discr, ty: _, default, cases } => {
            let d = resolve_const(discr, consts)?;
            let val = const_to_i64(&d)?;
            for (case_val, block) in cases {
                if *case_val == val {
                    return Some(Terminator::Br { target: *block });
                }
            }
            Some(Terminator::Br { target: *default })
        }

        _ => None,
    }
}

/// Convert a ConstValue to i64 for condition testing.
fn const_to_i64(c: &ConstValue) -> Option<i64> {
    match c {
        ConstValue::I8(v) => Some(*v as i64),
        ConstValue::I16(v) => Some(*v as i64),
        ConstValue::I32(v) => Some(*v as i64),
        ConstValue::I64(v) => Some(*v),
        ConstValue::U8(v) => Some(*v as i64),
        ConstValue::U16(v) => Some(*v as i64),
        ConstValue::U32(v) => Some(*v as i64),
        ConstValue::U64(v) => Some(*v as i64),
        ConstValue::NullPtr => Some(0),
        _ => None,
    }
}

/// Convert a ConstValue to u64 for unsigned operations.
fn const_to_u64(c: &ConstValue) -> Option<u64> {
    match c {
        ConstValue::I8(v) => Some(*v as u8 as u64),
        ConstValue::I16(v) => Some(*v as u16 as u64),
        ConstValue::I32(v) => Some(*v as u32 as u64),
        ConstValue::I64(v) => Some(*v as u64),
        ConstValue::U8(v) => Some(*v as u64),
        ConstValue::U16(v) => Some(*v as u64),
        ConstValue::U32(v) => Some(*v as u64),
        ConstValue::U64(v) => Some(*v),
        ConstValue::NullPtr => Some(0),
        _ => None,
    }
}

/// Create a ConstValue from i64 with the given type.
fn i64_to_const(val: i64, ty: &IrType) -> ConstValue {
    match ty {
        IrType::I8 => ConstValue::I8(val as i8),
        IrType::I16 => ConstValue::I16(val as i16),
        IrType::I32 => ConstValue::I32(val as i32),
        IrType::I64 => ConstValue::I64(val),
        IrType::U8 => ConstValue::U8(val as u8),
        IrType::U16 => ConstValue::U16(val as u16),
        IrType::U32 => ConstValue::U32(val as u32),
        IrType::U64 => ConstValue::U64(val as u64),
        _ => ConstValue::I64(val),
    }
}

fn u64_to_const(val: u64, ty: &IrType) -> ConstValue {
    match ty {
        IrType::I8 => ConstValue::I8(val as i8),
        IrType::I16 => ConstValue::I16(val as i16),
        IrType::I32 => ConstValue::I32(val as i32),
        IrType::I64 => ConstValue::I64(val as i64),
        IrType::U8 => ConstValue::U8(val as u8),
        IrType::U16 => ConstValue::U16(val as u16),
        IrType::U32 => ConstValue::U32(val as u32),
        IrType::U64 => ConstValue::U64(val),
        _ => ConstValue::U64(val),
    }
}

fn const_to_f64(c: &ConstValue) -> Option<f64> {
    match c {
        ConstValue::F32(v) => Some(*v as f64),
        ConstValue::F64(v) => Some(*v),
        _ => None,
    }
}

fn f64_to_const(val: f64, ty: &IrType) -> ConstValue {
    match ty {
        IrType::F32 => ConstValue::F32(val as f32),
        IrType::F64 => ConstValue::F64(val),
        _ => ConstValue::F64(val),
    }
}

/// Fold a binary operation with constant operands.
fn fold_binop(op: BinOpKind, lhs: &ConstValue, rhs: &ConstValue, ty: &IrType) -> Option<ConstValue> {
    // Integer operations
    if ty.is_integer() || ty.is_pointer() {
        let l = const_to_i64(lhs)?;
        let r = const_to_i64(rhs)?;
        let lu = const_to_u64(lhs)?;
        let ru = const_to_u64(rhs)?;

        let result = match op {
            BinOpKind::Add => Some(i64_to_const(l.wrapping_add(r), ty)),
            BinOpKind::Sub => Some(i64_to_const(l.wrapping_sub(r), ty)),
            BinOpKind::Mul => Some(i64_to_const(l.wrapping_mul(r), ty)),
            BinOpKind::SDiv => {
                if r == 0 { None } else { Some(i64_to_const(l.wrapping_div(r), ty)) }
            }
            BinOpKind::UDiv => {
                if ru == 0 { None } else { Some(u64_to_const(lu.wrapping_div(ru), ty)) }
            }
            BinOpKind::SRem => {
                if r == 0 { None } else { Some(i64_to_const(l.wrapping_rem(r), ty)) }
            }
            BinOpKind::URem => {
                if ru == 0 { None } else { Some(u64_to_const(lu.wrapping_rem(ru), ty)) }
            }
            BinOpKind::And => Some(u64_to_const(lu & ru, ty)),
            BinOpKind::Or => Some(u64_to_const(lu | ru, ty)),
            BinOpKind::Xor => Some(u64_to_const(lu ^ ru, ty)),
            BinOpKind::Shl => {
                let shift = ru & 63;
                Some(u64_to_const(lu.wrapping_shl(shift as u32), ty))
            }
            BinOpKind::LShr => {
                let shift = ru & 63;
                Some(u64_to_const(lu.wrapping_shr(shift as u32), ty))
            }
            BinOpKind::AShr => {
                let shift = ru & 63;
                Some(i64_to_const(l.wrapping_shr(shift as u32), ty))
            }
            _ => None,
        };
        // Truncate to type width
        return result.map(|c| truncate_const(&c, ty));
    }

    // Float operations
    if ty.is_float() {
        let l = const_to_f64(lhs)?;
        let r = const_to_f64(rhs)?;
        let result = match op {
            BinOpKind::FAdd => l + r,
            BinOpKind::FSub => l - r,
            BinOpKind::FMul => l * r,
            BinOpKind::FDiv => l / r, // Float div by zero is fine (produces Inf/NaN)
            BinOpKind::FRem => l % r,
            _ => return None,
        };
        return Some(f64_to_const(result, ty));
    }

    None
}

/// Truncate a constant to fit the given type width.
fn truncate_const(c: &ConstValue, ty: &IrType) -> ConstValue {
    let bits = ty.bit_width();
    if bits == 0 || bits >= 64 {
        return c.clone();
    }
    let mask = if bits < 64 { (1u64 << bits) - 1 } else { u64::MAX };
    let raw = const_to_u64(c).unwrap_or(0) & mask;
    if ty.is_signed() {
        // Sign extend from bit_width
        let sign_bit = 1u64 << (bits - 1);
        let val = if raw & sign_bit != 0 {
            (raw | !mask) as i64
        } else {
            raw as i64
        };
        i64_to_const(val, ty)
    } else {
        u64_to_const(raw, ty)
    }
}

/// Fold a unary operation with a constant operand.
fn fold_unaryop(op: UnaryOpKind, val: &ConstValue, ty: &IrType) -> Option<ConstValue> {
    match op {
        UnaryOpKind::Neg => {
            if ty.is_integer() {
                let v = const_to_i64(val)?;
                Some(truncate_const(&i64_to_const(v.wrapping_neg(), ty), ty))
            } else {
                None
            }
        }
        UnaryOpKind::FNeg => {
            let v = const_to_f64(val)?;
            Some(f64_to_const(-v, ty))
        }
        UnaryOpKind::BitNot => {
            let v = const_to_u64(val)?;
            Some(truncate_const(&u64_to_const(!v, ty), ty))
        }
        UnaryOpKind::LogNot => {
            let v = const_to_i64(val)?;
            let result = if v == 0 { 1i64 } else { 0i64 };
            Some(i64_to_const(result, ty))
        }
    }
}

/// Fold an integer comparison with constant operands.
fn fold_icmp(pred: IcmpPred, lhs: &ConstValue, rhs: &ConstValue, _ty: &IrType) -> Option<ConstValue> {
    let l = const_to_i64(lhs)?;
    let r = const_to_i64(rhs)?;
    let lu = const_to_u64(lhs)?;
    let ru = const_to_u64(rhs)?;

    let result = match pred {
        IcmpPred::Eq => l == r,
        IcmpPred::Ne => l != r,
        IcmpPred::Slt => l < r,
        IcmpPred::Sgt => l > r,
        IcmpPred::Sle => l <= r,
        IcmpPred::Sge => l >= r,
        IcmpPred::Ult => lu < ru,
        IcmpPred::Ugt => lu > ru,
        IcmpPred::Ule => lu <= ru,
        IcmpPred::Uge => lu >= ru,
    };

    Some(ConstValue::I8(if result { 1 } else { 0 }))
}

/// Fold a float comparison.
fn fold_fcmp(pred: FcmpPred, lhs: &ConstValue, rhs: &ConstValue) -> Option<ConstValue> {
    let l = const_to_f64(lhs)?;
    let r = const_to_f64(rhs)?;

    let result = match pred {
        FcmpPred::Oeq => l == r && !l.is_nan() && !r.is_nan(),
        FcmpPred::One => l != r && !l.is_nan() && !r.is_nan(),
        FcmpPred::Olt => l < r,
        FcmpPred::Ogt => l > r,
        FcmpPred::Ole => l <= r,
        FcmpPred::Oge => l >= r,
        FcmpPred::Ord => !l.is_nan() && !r.is_nan(),
        FcmpPred::Uno => l.is_nan() || r.is_nan(),
        FcmpPred::Ueq => l == r || l.is_nan() || r.is_nan(),
        FcmpPred::Une => l != r || l.is_nan() || r.is_nan(),
        FcmpPred::Ult => l < r || l.is_nan() || r.is_nan(),
        FcmpPred::Ugt => l > r || l.is_nan() || r.is_nan(),
        FcmpPred::Ule => l <= r || l.is_nan() || r.is_nan(),
        FcmpPred::Uge => l >= r || l.is_nan() || r.is_nan(),
    };

    Some(ConstValue::I8(if result { 1 } else { 0 }))
}

/// Fold a type cast with a constant operand.
fn fold_cast(kind: CastKind, val: &ConstValue, src_ty: &IrType, dst_ty: &IrType) -> Option<ConstValue> {
    match kind {
        CastKind::ZExt => {
            let v = const_to_u64(val)?;
            Some(u64_to_const(v, dst_ty))
        }
        CastKind::SExt => {
            let v = const_to_i64(val)?;
            Some(i64_to_const(v, dst_ty))
        }
        CastKind::Trunc => {
            let v = const_to_u64(val)?;
            Some(truncate_const(&u64_to_const(v, dst_ty), dst_ty))
        }
        CastKind::FPToSI => {
            let v = const_to_f64(val)?;
            if v.is_nan() || v.is_infinite() {
                return None; // Safety: don't fold NaN/Inf to int
            }
            Some(i64_to_const(v as i64, dst_ty))
        }
        CastKind::FPToUI => {
            let v = const_to_f64(val)?;
            if v.is_nan() || v.is_infinite() || v < 0.0 {
                return None;
            }
            Some(u64_to_const(v as u64, dst_ty))
        }
        CastKind::SIToFP => {
            let v = const_to_i64(val)?;
            Some(f64_to_const(v as f64, dst_ty))
        }
        CastKind::UIToFP => {
            let v = const_to_u64(val)?;
            Some(f64_to_const(v as f64, dst_ty))
        }
        CastKind::FPExt => {
            let v = const_to_f64(val)?;
            Some(f64_to_const(v, dst_ty))
        }
        CastKind::FPTrunc => {
            let v = const_to_f64(val)?;
            Some(f64_to_const(v, dst_ty))
        }
        CastKind::PtrToInt => {
            if matches!(val, ConstValue::NullPtr) {
                Some(u64_to_const(0, dst_ty))
            } else {
                None
            }
        }
        CastKind::IntToPtr => {
            let v = const_to_u64(val)?;
            if v == 0 {
                Some(ConstValue::NullPtr)
            } else {
                None
            }
        }
        CastKind::Bitcast => {
            if src_ty == dst_ty {
                Some(val.clone())
            } else {
                None
            }
        }
    }
}

