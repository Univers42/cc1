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
