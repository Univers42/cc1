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

