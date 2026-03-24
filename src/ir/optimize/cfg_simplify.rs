// ir/optimize/cfg_simplify.rs — CFG simplification.
//
// Simplifies the control flow graph through seven sub-passes that run
// to a fixpoint: fold constant branches, fold constant switches,
// simplify redundant branches, thread jump chains, remove dead blocks,
// simplify trivial phis, and merge single-predecessor blocks.

use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::{BasicBlock, IrFunction};
use crate::ir::types::*;
use std::collections::{HashSet, VecDeque};

/// Run CFG simplification on a single function. Returns true if any changes.
pub fn cfg_simplify(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Run sub-passes to fixpoint
    loop {
        let mut iter_changed = false;

        iter_changed |= fold_constant_branches(func);
        iter_changed |= fold_constant_switches(func);
        iter_changed |= simplify_redundant_branches(func);
        iter_changed |= thread_jump_chains(func);
        iter_changed |= remove_dead_blocks(func);
        iter_changed |= simplify_trivial_phis(func);
        iter_changed |= merge_single_pred_blocks(func);

        if iter_changed {
            changed = true;
        } else {
            break;
        }
    }

    changed
}

/// Fold conditional branches with constant conditions to unconditional branches.
fn fold_constant_branches(func: &mut IrFunction) -> bool {
    let mut changed = false;

    for bi in 0..func.blocks.len() {
        let term = &func.blocks[bi].terminator;
        if let Terminator::CondBr { cond, true_bb, false_bb } = term {
            if let Some(val) = resolve_cond_const(cond) {
                let target = if val != 0 { *true_bb } else { *false_bb };
                func.blocks[bi].terminator = Terminator::Br { target };
                changed = true;
            } else if true_bb == false_bb {
                // Both targets the same
                let target = *true_bb;
                func.blocks[bi].terminator = Terminator::Br { target };
                changed = true;
            }
        }
    }

    changed
}

/// Fold constant switches to unconditional branches.
fn fold_constant_switches(func: &mut IrFunction) -> bool {
    let mut changed = false;

    for bi in 0..func.blocks.len() {
        if let Terminator::Switch { discr, default, cases, .. } = &func.blocks[bi].terminator {
            if let Some(val) = resolve_cond_const(discr) {
                let target = cases
                    .iter()
                    .find(|(cv, _)| *cv == val)
                    .map(|(_, bb)| *bb)
                    .unwrap_or(*default);
                func.blocks[bi].terminator = Terminator::Br { target };
                changed = true;
            }
        }
    }

    changed
}

/// Simplify redundant conditional branches (both targets the same).
fn simplify_redundant_branches(func: &mut IrFunction) -> bool {
    let mut changed = false;

    for bi in 0..func.blocks.len() {
        if let Terminator::CondBr { true_bb, false_bb, .. } = &func.blocks[bi].terminator {
            if true_bb == false_bb {
                let target = *true_bb;
                func.blocks[bi].terminator = Terminator::Br { target };
                changed = true;
            }
        }
    }

    changed
}

/// Thread jump chains: redirect predecessors of empty unconditional-branch blocks.
