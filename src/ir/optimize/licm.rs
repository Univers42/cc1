// ir/optimize/licm.rs — Loop-Invariant Code Motion.
//
// Identifies natural loops and hoists loop-invariant instructions
// to preheader blocks. An instruction is loop-invariant if all its
// operands are constants, defined outside the loop, or themselves
// loop-invariant (computed to a fixpoint).

use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::*;
use super::loop_analysis::CfgAnalysis;
use std::collections::HashSet;

/// Run LICM on a single function using shared CFG analysis.
/// Returns true if any instructions were hoisted.
pub fn licm(func: &mut IrFunction, cfg: &CfgAnalysis) -> bool {
    let mut changed = false;

    for nl in &cfg.loops {
        let preheader = match nl.preheader {
            Some(p) => p,
            None => continue, // Can't hoist without a preheader
        };

        // Collect all value definitions inside the loop.
        let mut loop_defs: HashSet<ValueId> = HashSet::new();
        for &bid in &nl.body {
            let bi = bid.0 as usize;
            if bi < func.blocks.len() {
                for inst in &func.blocks[bi] .insts {
                    if let Some(r) = inst.result() {
                        loop_defs.insert(r);
                    }
                }
            }
        }

        // Find store targets inside the loop (for load hoisting safety).
        let mut stores_in_loop: HashSet<ValueId> = HashSet::new();
        let mut has_calls = false;
        for &bid in &nl.body {
            let bi = bid.0 as usize;
            if bi < func.blocks.len() {
                for inst in &func.blocks[bi].insts {
                    match inst {
                        Instruction::Store { addr: Operand::Value(v), .. } => {
                            stores_in_loop.insert(*v);
                        }
                        Instruction::Call { .. } | Instruction::CallIndirect { .. } => {
                            has_calls = true;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Compute loop-invariant instructions to fixpoint.
        let mut invariant: HashSet<ValueId> = HashSet::new();
        loop {
            let mut progress = false;
            for &bid in &nl.body {
                let bi = bid.0 as usize;
                if bi >= func.blocks.len() { continue; }
                for inst in &func.blocks[bi].insts {
                    let result = match inst.result() {
                        Some(r) => r,
                        None => continue,
                    };
                    if invariant.contains(&result) {
                        continue;
                    }
                    if !is_hoistable(inst, &loop_defs, &invariant, &stores_in_loop, has_calls) {
                        continue;
                    }
                    invariant.insert(result);
                    progress = true;
                }
            }
            if !progress { break; }
        }

        if invariant.is_empty() {
            continue;
        }

        // Collect instructions to hoist (in topological order).
        let mut to_hoist: Vec<Instruction> = Vec::new();
        for &bid in &nl.body {
            let bi = bid.0 as usize;
            if bi >= func.blocks.len() { continue; }
            let block = &mut func.blocks[bi];
            let mut kept = Vec::new();
            for inst in block.insts.drain(..) {
                if let Some(r) = inst.result() {
                    if invariant.contains(&r) {
                        to_hoist.push(inst);
                        continue;
                    }
                }
                kept.push(inst);
            }
            block.insts = kept;
        }

        if !to_hoist.is_empty() {
            // Insert hoisted instructions before the preheader's terminator.
            let phi = preheader.0 as usize;
            if phi < func.blocks.len() {
                // Insert at the end of instructions (before terminator).
                for inst in to_hoist {
                    func.blocks[phi].insts.push(inst);
                }
                changed = true;
            }
        }
    }

    changed
}

/// Check if an instruction is safe and profitable to hoist.
fn is_hoistable(
    inst: &Instruction,
    loop_defs: &HashSet<ValueId>,
    invariant: &HashSet<ValueId>,
    _stores_in_loop: &HashSet<ValueId>,
    _has_calls: bool,
) -> bool {
    // Only pure instructions are hoistable.
    if !inst.is_pure() {
        // Special case: loads from non-address-taken allocas could be hoisted
        // but we skip this for safety. Division/remainder are excluded (can trap).
        return false;
    }

    // Division and remainder can trap, do not hoist.
    if let Instruction::BinOp { op, .. } = inst {
        if matches!(op, BinOpKind::SDiv | BinOpKind::UDiv | BinOpKind::SRem | BinOpKind::URem) {
            return false;
        }
    }

    // All operands must be either:
    // - Constants
    // - Defined outside the loop
    // - Already proven loop-invariant
    let mut all_invariant = true;
    inst.for_each_operand(|op| {
        if let Operand::Value(v) = op {
            if loop_defs.contains(v) && !invariant.contains(v) {
                all_invariant = false;
            }
        }
    });

    all_invariant
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::instruction::Terminator;
    use crate::ir::types::*;

    #[test]
    fn test_licm_hoist_constant() {
        let mut f = IrFunction::new("test", IrType::Void, Linkage::External);
        let b0 = f.create_block("preheader");
        let b1 = f.create_block("header");
        let b2 = f.create_block("body");
        let b3 = f.create_block("exit");

        let cond = f.alloc_value();
        let v0 = f.alloc_value(); // loop-invariant: add of two constants

        f.block_mut(b0).set_terminator(Terminator::Br { target: b1 });
        f.block_mut(b1).set_terminator(Terminator::CondBr {
            cond: Operand::Value(cond),
            true_bb: b2,
            false_bb: b3,
        });
        f.block_mut(b2).push(Instruction::BinOp {
            result: v0,
            op: BinOpKind::Add,
            lhs: Operand::Const(ConstValue::I32(1)),
            rhs: Operand::Const(ConstValue::I32(2)),
            ty: IrType::I32,
        });
        f.block_mut(b2).set_terminator(Terminator::Br { target: b1 });
        f.block_mut(b3).set_terminator(Terminator::Ret { value: None });

        let cfg = CfgAnalysis::build(&f);
        let changed = licm(&mut f, &cfg);
        assert!(changed);
        // The instruction should be moved to the preheader
        assert!(!f.block(b0).insts.is_empty());
        assert!(f.block(b2).insts.is_empty());
    }
}
