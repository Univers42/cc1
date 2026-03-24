// ir/optimize/if_convert.rs — Diamond/triangle if-conversion to Select.
//
// Detects simple diamond or triangle patterns in the CFG and converts
// them into Select instructions when both arms are small and side-effect-free.

use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::IrFunction;
use crate::ir::types::*;

/// Maximum number of instructions in a convertible arm.
const MAX_ARM_INSTRS: usize = 8;

/// Run if-conversion on a single function.
pub fn if_convert(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Run to fixpoint — conversion may expose more opportunities.
    loop {
        let round_changed = if_convert_round(func);
        if !round_changed {
            break;
        }
        changed = true;
    }

    changed
}

fn if_convert_round(func: &mut IrFunction) -> bool {
    let mut changed = false;
    func.compute_predecessors();
    let preds: Vec<Vec<usize>> = func.blocks.iter()
        .map(|b| b.preds.iter().map(|p| p.0 as usize).collect())
        .collect();

    // Iterate blocks looking for CondBr with diamond/triangle shapes.
    let num_blocks = func.blocks.len();
    for bi in 0..num_blocks {
        let (cond_op, true_bb, false_bb) = match &func.blocks[bi].terminator {
            Terminator::CondBr { cond, true_bb, false_bb } => {
                (cond.clone(), *true_bb, *false_bb)
            }
            _ => continue,
        };

        // Try diamond: true_bb and false_bb both branch unconditionally to
        // the same merge block.
        if let Some(merge_bi) = detect_diamond(func, &preds, bi, true_bb, false_bb) {
            if try_convert_diamond(func, bi, true_bb, false_bb, merge_bi, &cond_op) {
                changed = true;
                continue;
            }
        }

        // Try triangle: true_bb branches to false_bb (or vice versa).
        if try_convert_triangle(func, &preds, bi, true_bb, false_bb, &cond_op) {
            changed = true;
            continue;
        }
    }

    changed
}

/// Detect diamond shape: A → T + F, T → M, F → M.
fn detect_diamond(
    func: &IrFunction,
    preds: &[Vec<usize>],
    _cond_bi: usize,
    true_bb: BlockId,
    false_bb: BlockId,
) -> Option<usize> {
    let ti = true_bb.0 as usize;
    let fi = false_bb.0 as usize;

    if ti >= func.blocks.len() || fi >= func.blocks.len() {
        return None;
    }

    // Both arms must be single-predecessor.
    if preds.get(ti).map_or(true, |p| p.len() != 1) {
        return None;
    }
    if preds.get(fi).map_or(true, |p| p.len() != 1) {
        return None;
    }

    // Both must branch unconditionally to the same target.
    let t_target = match &func.blocks[ti].terminator {
        Terminator::Br { target } => target.0 as usize,
        _ => return None,
    };
    let f_target = match &func.blocks[fi].terminator {
        Terminator::Br { target } => target.0 as usize,
        _ => return None,
    };

    if t_target == f_target {
        Some(t_target)
    } else {
        None
    }
}

/// Try converting a diamond pattern to selects.
