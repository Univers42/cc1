// ir/optimize/dce.rs — Dead code elimination.
//
// Removes instructions whose results are never used. Uses a use-count-based
// worklist algorithm with O(n) complexity.

use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::*;
use std::collections::{HashMap, VecDeque};

/// Run dead code elimination on a single function. Returns true if any changes.
pub fn dce(func: &mut IrFunction) -> bool {
    let num_values = func.value_count() as usize;

    // Step 1: Build use counts
    let mut use_count: Vec<u32> = vec![0; num_values];

    for block in &func.blocks {
        for inst in &block.insts {
            // Count uses of operands, but exclude self-referencing phi edges.
            let self_val = inst.result();
            inst.for_each_operand(|op| {
                if let Operand::Value(v) = op {
                    // Exclude self-references in phis
                    if Some(*v) != self_val {
                        let idx = v.0 as usize;
                        if idx < num_values {
                            use_count[idx] = use_count[idx].saturating_add(1);
                        }
                    }
                }
            });
        }
        // Count uses in terminator
        block.terminator.for_each_operand(|op| {
            if let Operand::Value(v) = op {
                let idx = v.0 as usize;
                if idx < num_values {
                    use_count[idx] = use_count[idx].saturating_add(1);
                }
            }
        });
    }

    // Step 2: Build definition map (ValueId -> (block_idx, inst_idx))
    // Skip side-effecting instructions.
    let mut def_map: HashMap<ValueId, (usize, usize)> = HashMap::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.insts.iter().enumerate() {
            if let Some(result) = inst.result() {
                if !inst.has_side_effects() {
                    def_map.insert(result, (bi, ii));
                }
            }
        }
    }

    // Step 3: Seed worklist with zero-use non-side-effecting instructions.
    let mut worklist: VecDeque<ValueId> = VecDeque::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.insts.iter().enumerate() {
            if let Some(result) = inst.result() {
                if !inst.has_side_effects() && use_count[result.0 as usize] == 0 {
                    worklist.push_back(result);
                }
            }
        }
    }

    // Step 4: Process worklist.
    let mut dead: Vec<(usize, usize)> = Vec::new();

    while let Some(val) = worklist.pop_front() {
        if let Some(&(bi, ii)) = def_map.get(&val) {
            // Mark as dead
            dead.push((bi, ii));

            // Decrement operand use counts
            let inst = &func.blocks[bi].insts[ii];
            inst.for_each_operand(|op| {
                if let Operand::Value(v) = op {
                    let idx = v.0 as usize;
                    if idx < num_values && use_count[idx] > 0 {
                        use_count[idx] -= 1;
                        if use_count[idx] == 0 {
                            if def_map.contains_key(v) {
                                worklist.push_back(*v);
                            }
                        }
                    }
                }
            });
        }
    }

    // Step 5: Sweep dead instructions.
    if dead.is_empty() {
        return false;
    }

    let dead_set: std::collections::HashSet<(usize, usize)> = dead.into_iter().collect();
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        let mut ii = 0;
        block.insts.retain(|inst| {
            let keep = !dead_set.contains(&(bi, ii));
            ii += 1;
            keep
        });
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::instruction::Terminator;

    #[test]
    fn test_dce_unused() {
        let mut f = IrFunction::new("test", IrType::I32, Linkage::External);
        let b = f.create_block("entry");

        let v0 = f.alloc_value(); // used
        let v1 = f.alloc_value(); // unused (dead)

        f.block_mut(b).push(Instruction::BinOp {
            result: v0,
            op: BinOpKind::Add,
            lhs: Operand::Const(ConstValue::I32(1)),
            rhs: Operand::Const(ConstValue::I32(2)),
            ty: IrType::I32,
        });
        f.block_mut(b).push(Instruction::BinOp {
            result: v1,
            op: BinOpKind::Mul,
            lhs: Operand::Const(ConstValue::I32(3)),
            rhs: Operand::Const(ConstValue::I32(4)),
            ty: IrType::I32,
        });
        f.block_mut(b).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v0)),
        });

        assert!(dce(&mut f));
        // Only v0's instruction should remain
        assert_eq!(f.block(b).insts.len(), 1);
    }

    #[test]
    fn test_dce_chain() {
        let mut f = IrFunction::new("test", IrType::I32, Linkage::External);
        let b = f.create_block("entry");

        let v0 = f.alloc_value(); // dead
        let v1 = f.alloc_value(); // dead (uses v0)
        let v2 = f.alloc_value(); // live

        f.block_mut(b).push(Instruction::BinOp {
            result: v0,
            op: BinOpKind::Add,
            lhs: Operand::Const(ConstValue::I32(1)),
            rhs: Operand::Const(ConstValue::I32(2)),
            ty: IrType::I32,
        });
        f.block_mut(b).push(Instruction::BinOp {
            result: v1,
            op: BinOpKind::Mul,
            lhs: Operand::Value(v0),
            rhs: Operand::Const(ConstValue::I32(3)),
            ty: IrType::I32,
        });
        f.block_mut(b).push(Instruction::BinOp {
            result: v2,
            op: BinOpKind::Add,
            lhs: Operand::Const(ConstValue::I32(42)),
            rhs: Operand::Const(ConstValue::I32(0)),
            ty: IrType::I32,
        });
        f.block_mut(b).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v2)),
        });

        assert!(dce(&mut f));
        // Only v2's instruction should remain (v0 and v1 are dead chain)
        assert_eq!(f.block(b).insts.len(), 1);
    }

    #[test]
