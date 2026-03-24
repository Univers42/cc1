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
fn thread_jump_chains(func: &mut IrFunction) -> bool {
    let mut changed = false;
    const MAX_CHAIN_DEPTH: usize = 32;

    // Build a map of block_id -> final target for empty blocks.
    let mut redirect: Vec<Option<BlockId>> = vec![None; func.blocks.len()];

    for (bi, block) in func.blocks.iter().enumerate() {
        if block.insts.is_empty() {
            if let Terminator::Br { target } = &block.terminator {
                // Follow chain
                let mut final_target = *target;
                let mut visited = HashSet::new();
                visited.insert(BlockId(bi as u32));
                let mut depth = 0;
                while depth < MAX_CHAIN_DEPTH {
                    let ti = final_target.0 as usize;
                    if ti >= func.blocks.len() { break; }
                    if !func.blocks[ti].insts.is_empty() { break; }
                    if let Terminator::Br { target: next } = &func.blocks[ti].terminator {
                        if visited.contains(next) { break; } // cycle
                        visited.insert(*next);
                        final_target = *next;
                        depth += 1;
                    } else {
                        break;
                    }
                }
                if final_target != *target {
                    redirect[bi] = Some(final_target);
                }
            }
        }
    }

    // Apply redirections to all terminators.
    for bi in 0..func.blocks.len() {
        let term = &mut func.blocks[bi].terminator;
        match term {
            Terminator::Br { target } => {
                let ti = target.0 as usize;
                if ti < redirect.len() {
                    if let Some(new_target) = redirect[ti] {
                        *target = new_target;
                        changed = true;
                    }
                }
            }
            Terminator::CondBr { true_bb, false_bb, .. } => {
                let ti = true_bb.0 as usize;
                if ti < redirect.len() {
                    if let Some(new_target) = redirect[ti] {
                        *true_bb = new_target;
                        changed = true;
                    }
                }
                let fi = false_bb.0 as usize;
                if fi < redirect.len() {
                    if let Some(new_target) = redirect[fi] {
                        *false_bb = new_target;
                        changed = true;
                    }
                }
            }
            Terminator::Switch { default, cases, .. } => {
                let di = default.0 as usize;
                if di < redirect.len() {
                    if let Some(new_target) = redirect[di] {
                        *default = new_target;
                        changed = true;
                    }
                }
                for (_, bb) in cases {
                    let ci = bb.0 as usize;
                    if ci < redirect.len() {
                        if let Some(new_target) = redirect[ci] {
                            *bb = new_target;
                            changed = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    changed
}

/// Remove dead (unreachable) blocks via BFS from the entry.
fn remove_dead_blocks(func: &mut IrFunction) -> bool {
    if func.blocks.is_empty() {
        return false;
    }

    // BFS reachability
    let mut reachable = vec![false; func.blocks.len()];
    let mut queue = VecDeque::new();
    reachable[0] = true;
    queue.push_back(BlockId(0));

    // Also mark blocks referenced by LabelAddr as reachable
    for block in &func.blocks {
        for inst in &block.insts {
            if let Instruction::LabelAddr { block: bid, .. } = inst {
                let idx = bid.0 as usize;
                if idx < reachable.len() && !reachable[idx] {
                    reachable[idx] = true;
                    queue.push_back(*bid);
                }
            }
        }
    }

    while let Some(bid) = queue.pop_front() {
        let succs = func.blocks[bid.0 as usize].terminator.successors();
        for succ in succs {
            let si = succ.0 as usize;
            if si < reachable.len() && !reachable[si] {
                reachable[si] = true;
                queue.push_back(succ);
            }
        }
    }

    // Check if any block is unreachable
    let any_dead = reachable.iter().any(|&r| !r);
    if !any_dead {
        return false;
    }

    // Collect live block indices and build remapping
    let dead_ids: HashSet<BlockId> = reachable
        .iter()
        .enumerate()
        .filter(|(_, &r)| !r)
        .map(|(i, _)| BlockId(i as u32))
        .collect();

    // Remove phi entries from dead predecessors
    for block in &mut func.blocks {
        for inst in &mut block.insts {
            if let Instruction::Phi { incoming, .. } = inst {
                incoming.retain(|(bb, _)| !dead_ids.contains(bb));
            }
        }
    }

    // Replace dead blocks with empty blocks (we can't easily remove them
    // since BlockId is an index). Instead, mark them with Unreachable terminator
    // and clear their instructions.
    let mut changed = false;
    for (i, r) in reachable.iter().enumerate() {
        if !r {
            // Only mark changed if the block wasn't already dead
            if !func.blocks[i].insts.is_empty()
                || !matches!(func.blocks[i].terminator, Terminator::Unreachable)
            {
                func.blocks[i].insts.clear();
                func.blocks[i].terminator = Terminator::Unreachable;
                func.blocks[i].preds.clear();
                changed = true;
            }
        }
    }

    changed
}

/// Simplify trivial phi nodes (single incoming or all-same values).
fn simplify_trivial_phis(func: &mut IrFunction) -> bool {
    let mut changed = false;

    for bi in 0..func.blocks.len() {
        for ii in 0..func.blocks[bi].insts.len() {
            if let Instruction::Phi { result, incoming, .. } = &func.blocks[bi].insts[ii] {
                let result = *result;

                // Single incoming edge
                if incoming.len() == 1 {
                    let src = incoming[0].1.clone();
                    func.blocks[bi].insts[ii] = Instruction::Copy { result, src };
                    changed = true;
                    continue;
                }

                // All incoming values identical (excluding self-references)
                if incoming.len() > 1 {
                    let mut unique_val: Option<&Operand> = None;
                    let mut all_same = true;
                    for (_, val) in incoming {
                        // Skip self-references
                        if let Operand::Value(v) = val {
                            if *v == result {
                                continue;
                            }
                        }
                        match unique_val {
                            None => unique_val = Some(val),
                            Some(prev) => {
                                if prev != val {
                                    all_same = false;
                                    break;
                                }
                            }
                        }
                    }
                    if all_same {
                        if let Some(val) = unique_val {
                            let src = val.clone();
                            func.blocks[bi].insts[ii] = Instruction::Copy { result, src };
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    changed
}

/// Merge single-predecessor blocks. When block A ends with an unconditional branch
/// to block B and B has exactly one predecessor (A), fuse B into A.
fn merge_single_pred_blocks(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Rebuild predecessors
    func.compute_predecessors();

    // Iterate and merge
    let mut merged = true;
    while merged {
        merged = false;
        func.compute_predecessors();

        for bi in 0..func.blocks.len() {
            if let Terminator::Br { target } = func.blocks[bi].terminator {
                let ti = target.0 as usize;
                if ti < func.blocks.len()
                    && ti != bi // don't merge self-loop
                    && func.blocks[ti].preds.len() == 1
                    && func.blocks[ti].preds[0] == BlockId(bi as u32)
                {
                    // Check: target block must not be referenced by LabelAddr
                    let has_label_ref = func.blocks.iter().any(|b| {
                        b.insts.iter().any(|inst| {
                            matches!(inst, Instruction::LabelAddr { block, .. } if *block == target)
                        })
                    });
                    if has_label_ref {
                        continue;
                    }

                    // Merge: move B's instructions and terminator into A
                    let b_insts = std::mem::take(&mut func.blocks[ti].insts);
                    let b_term = func.blocks[ti].terminator.clone();

                    func.blocks[bi].insts.extend(b_insts);
                    func.blocks[bi].terminator = b_term;

                    // Clear B
                    func.blocks[ti].terminator = Terminator::Unreachable;
                    func.blocks[ti].preds.clear();

                    changed = true;
                    merged = true;
                    break; // Restart after merge
                }
            }
        }
    }

    changed
}

/// Resolve a condition operand to a constant if it's an immediate constant value.
fn resolve_cond_const(op: &Operand) -> Option<i64> {
    match op {
        Operand::Const(ConstValue::I8(v)) => Some(*v as i64),
        Operand::Const(ConstValue::I16(v)) => Some(*v as i64),
        Operand::Const(ConstValue::I32(v)) => Some(*v as i64),
        Operand::Const(ConstValue::I64(v)) => Some(*v),
        Operand::Const(ConstValue::U8(v)) => Some(*v as i64),
        Operand::Const(ConstValue::U16(v)) => Some(*v as i64),
        Operand::Const(ConstValue::U32(v)) => Some(*v as i64),
        Operand::Const(ConstValue::U64(v)) => Some(*v as i64),
        Operand::Const(ConstValue::NullPtr) => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fold_constant_condbr() {
        let mut f = IrFunction::new("test", IrType::Void, Linkage::External);
        let b0 = f.create_block("entry");
        let b1 = f.create_block("then");
        let b2 = f.create_block("else");

        f.block_mut(b0).set_terminator(Terminator::CondBr {
            cond: Operand::Const(ConstValue::I8(1)),
            true_bb: b1,
            false_bb: b2,
        });
        f.block_mut(b1).set_terminator(Terminator::Ret { value: None });
        f.block_mut(b2).set_terminator(Terminator::Ret { value: None });

        assert!(cfg_simplify(&mut f));
        // After folding and merging, b0 should eventually reach a Ret
        // (either still Br to b1, or merged with b1 into Ret directly)
        match &f.block(b0).terminator {
            Terminator::Br { target } => assert_eq!(*target, b1),
            Terminator::Ret { .. } => {} // merged
            other => panic!("Unexpected terminator: {:?}", other),
        }
    }

    #[test]
    fn test_remove_dead_block() {
        let mut f = IrFunction::new("test", IrType::Void, Linkage::External);
        let b0 = f.create_block("entry");
        let b1 = f.create_block("dead");
        let b2 = f.create_block("exit");

        f.block_mut(b0).set_terminator(Terminator::Br { target: b2 });
        f.block_mut(b1).set_terminator(Terminator::Ret { value: None }); // unreachable
        f.block_mut(b2).set_terminator(Terminator::Ret { value: None });

        assert!(cfg_simplify(&mut f));
        // b1 should be cleared
        assert!(f.block(b1).insts.is_empty());
        assert!(matches!(f.block(b1).terminator, Terminator::Unreachable));
    }

    #[test]
    fn test_trivial_phi() {
        let mut f = IrFunction::new("test", IrType::I32, Linkage::External);
        let b0 = f.create_block("entry");
        let v0 = f.alloc_value();
        let v1 = f.alloc_value();

        f.block_mut(b0).push(Instruction::BinOp {
            result: v0,
            op: BinOpKind::Add,
            lhs: Operand::Const(ConstValue::I32(1)),
            rhs: Operand::Const(ConstValue::I32(2)),
            ty: IrType::I32,
        });
        f.block_mut(b0).push(Instruction::Phi {
            result: v1,
            ty: IrType::I32,
            incoming: vec![(BlockId(0), Operand::Value(v0))],
        });
        f.block_mut(b0).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v1)),
        });

        assert!(cfg_simplify(&mut f));
        // The phi should be simplified to a Copy
        assert!(matches!(f.block(b0).insts[1], Instruction::Copy { .. }));
    }

    #[test]
