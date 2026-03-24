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
fn try_convert_diamond(
    func: &mut IrFunction,
    cond_bi: usize,
    true_bb: BlockId,
    false_bb: BlockId,
    merge_bi: usize,
    cond: &Operand,
) -> bool {
    let ti = true_bb.0 as usize;
    let fi = false_bb.0 as usize;

    // Check arm sizes and side effects.
    if !is_arm_convertible(&func.blocks[ti].insts) {
        return false;
    }
    if !is_arm_convertible(&func.blocks[fi].insts) {
        return false;
    }

    // Reject 128-bit types.
    for inst in &func.blocks[ti].insts {
        if let Some(ty) = result_type(inst) {
            if is_wide_type(&ty) {
                return false;
            }
        }
    }
    for inst in &func.blocks[fi].insts {
        if let Some(ty) = result_type(inst) {
            if is_wide_type(&ty) {
                return false;
            }
        }
    }

    // Collect instructions from both arms.
    let true_insts = func.blocks[ti].insts.clone();
    let false_insts = func.blocks[fi].insts.clone();

    // Convert phis in the merge block to selects.
    let merge_block = &func.blocks[merge_bi];
    let mut new_insts = Vec::new();

    for inst in &merge_block.insts {
        if let Instruction::Phi { result, ty, incoming } = inst {
            // Find incoming values from true and false arms.
            let true_val = incoming.iter()
                .find(|(b, _)| *b == true_bb)
                .map(|(_, op)| op.clone());
            let false_val = incoming.iter()
                .find(|(b, _)| *b == false_bb)
                .map(|(_, op)| op.clone());

            if let (Some(tv), Some(fv)) = (true_val, false_val) {
                new_insts.push(Instruction::Select {
                    result: *result,
                    cond: cond.clone(),
                    true_val: tv,
                    false_val: fv,
                    ty: ty.clone(),
                });
            } else {
                return false; // Can't convert this phi.
            }
        }
    }

    // If we got here, do the conversion:
    // 1. Move arm instructions into the cond block.
    let cond_block = &mut func.blocks[cond_bi];
    cond_block.insts.extend(true_insts);
    cond_block.insts.extend(false_insts);
    cond_block.insts.extend(new_insts);

    // 2. Set cond block terminator to branch directly to merge.
    let merge_bid = BlockId(merge_bi as u32);
    func.blocks[cond_bi].terminator = Terminator::Br { target: merge_bid };

    // 3. Clear the dead arm blocks.
    func.blocks[ti].insts.clear();
    func.blocks[ti].terminator = Terminator::Unreachable;
    func.blocks[fi].insts.clear();
    func.blocks[fi].terminator = Terminator::Unreachable;

    // 4. Remove phis in merge block that were converted.
    func.blocks[merge_bi].insts.retain(|inst| {
        !matches!(inst, Instruction::Phi { .. })
    });

    true
}

/// Try converting a triangle pattern.
fn try_convert_triangle(
    func: &mut IrFunction,
    preds: &[Vec<usize>],
    cond_bi: usize,
    true_bb: BlockId,
    false_bb: BlockId,
    cond: &Operand,
) -> bool {
    let ti = true_bb.0 as usize;
    let fi = false_bb.0 as usize;

    // Pattern: cond → true_bb, cond → false_bb, true_bb → false_bb
    // So false_bb serves as the merge.
    if ti < func.blocks.len() {
        if let Terminator::Br { target } = &func.blocks[ti].terminator {
            if *target == false_bb && preds.get(ti).map_or(false, |p| p.len() == 1) {
                if is_arm_convertible(&func.blocks[ti].insts)
                    && !func.blocks[ti].insts.iter().any(|i| result_type(i).map_or(false, |t| is_wide_type(&t)))
                {
                    // Convert phis in false_bb
                    let phis: Vec<_> = func.blocks[fi].insts.iter()
                        .filter(|i| matches!(i, Instruction::Phi { .. }))
                        .cloned()
                        .collect();

                    let mut selects = Vec::new();
                    for inst in &phis {
                        if let Instruction::Phi { result, ty, incoming } = inst {
                            let from_true = incoming.iter()
                                .find(|(b, _)| *b == true_bb)
                                .map(|(_, op)| op.clone());
                            let from_cond = incoming.iter()
                                .find(|(b, _)| b.0 as usize == cond_bi)
                                .map(|(_, op)| op.clone());

                            if let (Some(tv), Some(fv)) = (from_true, from_cond) {
                                selects.push(Instruction::Select {
                                    result: *result,
                                    cond: cond.clone(),
                                    true_val: tv,
                                    false_val: fv,
                                    ty: ty.clone(),
                                });
                            } else {
                                return false;
                            }
                        }
                    }

                    // Perform conversion
                    let arm_insts = func.blocks[ti].insts.clone();
                    func.blocks[cond_bi].insts.extend(arm_insts);
                    func.blocks[cond_bi].insts.extend(selects);
                    func.blocks[cond_bi].terminator = Terminator::Br { target: false_bb };

                    func.blocks[ti].insts.clear();
                    func.blocks[ti].terminator = Terminator::Unreachable;

                    func.blocks[fi].insts.retain(|i| !matches!(i, Instruction::Phi { .. }));

                    return true;
                }
            }
        }
    }

    false
}

/// Check if an arm's instructions are convertible (pure, small).
fn is_arm_convertible(insts: &[Instruction]) -> bool {
    if insts.len() > MAX_ARM_INSTRS {
        return false;
    }
    for inst in insts {
        if inst.has_side_effects() {
            return false;
        }
        // Also reject phis in the arm (shouldn't happen in single-pred block).
        if matches!(inst, Instruction::Phi { .. }) {
            return false;
        }
    }
    true
}

/// Get the result type of an instruction if it produces a value.
fn result_type(inst: &Instruction) -> Option<IrType> {
    if inst.result().is_some() {
        let ty = inst.result_type();
        if ty.is_void() { None } else { Some(ty) }
    } else {
        None
    }
}

/// Check if a type is too wide for select (I128/U128/F128).
fn is_wide_type(ty: &IrType) -> bool {
    matches!(ty, IrType::I128 | IrType::U128)
}

