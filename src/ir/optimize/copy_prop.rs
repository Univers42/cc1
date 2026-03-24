// ir/optimize/copy_prop.rs — Copy propagation with path compression.
//
// Replaces uses of Copy destinations with the Copy's source operand,
// transitively following chains. Uses a flat Vec<Option<Operand>> for O(1)
// lookups and union-find-style path compression.

use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::*;

/// Maximum chain depth to prevent pathological cases.
const MAX_CHAIN_DEPTH: usize = 64;

/// Run copy propagation on a single function. Returns true if any changes were made.
pub fn copy_prop(func: &mut IrFunction) -> bool {
    let num_values = func.value_count() as usize;
    // Build copy map: value_id -> what it copies from
    let mut copy_map: Vec<Option<Operand>> = vec![None; num_values];

    // First pass: find all Copy instructions and build the map.
    // Multi-def detection: if a value is defined by Copy in multiple blocks,
    // we mark it as None (ambiguous) and don't propagate.
    let mut def_count: Vec<u8> = vec![0; num_values];

    for block in &func.blocks {
        for inst in &block.insts {
            if let Instruction::Copy { result, src } = inst {
                let idx = result.0 as usize;
                if idx < num_values {
                    if def_count[idx] == 0 {
                        copy_map[idx] = Some(src.clone());
                    } else {
                        // Multi-def: don't propagate
                        copy_map[idx] = None;
                    }
                    def_count[idx] = def_count[idx].saturating_add(1);
                }
            }
        }
    }

    // Also gather copies from Phi nodes that have a single incoming value
    // (or all incoming values are the same). We handle these as copies too.
    for block in &func.blocks {
        for inst in &block.insts {
            if let Instruction::Phi { result, incoming, .. } = inst {
                if incoming.len() == 1 {
                    let idx = result.0 as usize;
                    if idx < num_values && def_count[idx] == 0 {
                        copy_map[idx] = Some(incoming[0].1.clone());
                        def_count[idx] = 1;
                    }
                }
            }
        }
    }

    // Resolve chains with path compression.
    // resolve(v) follows the chain v -> src -> src2 -> ... until we reach
    // a non-copy value, then updates all intermediate entries.
    fn resolve(copy_map: &mut [Option<Operand>], val: ValueId) -> Operand {
        let idx = val.0 as usize;
        if idx >= copy_map.len() {
            return Operand::Value(val);
        }
        match &copy_map[idx] {
            None => Operand::Value(val),
            Some(Operand::Const(c)) => Operand::Const(c.clone()),
            Some(Operand::Global(g)) => Operand::Global(g.clone()),
            Some(Operand::Label(b)) => Operand::Label(*b),
            Some(Operand::Value(next)) => {
                let next_val = *next;
                if next_val == val {
                    return Operand::Value(val); // self-reference
                }
                // Follow chain iteratively
                let mut chain = vec![idx];
                let mut current = next_val;
                let mut depth = 0;
                loop {
                    if depth >= MAX_CHAIN_DEPTH {
                        break;
                    }
                    let cur_idx = current.0 as usize;
                    if cur_idx >= copy_map.len() {
                        break;
                    }
                    match &copy_map[cur_idx] {
                        Some(Operand::Value(v)) if *v != current => {
                            chain.push(cur_idx);
                            current = *v;
                            depth += 1;
                        }
                        Some(Operand::Const(_))
                        | Some(Operand::Global(_))
                        | Some(Operand::Label(_)) => {
                            let resolved = copy_map[cur_idx].clone().unwrap();
                            // Path compression: update all entries in chain
                            for &ci in &chain {
                                copy_map[ci] = Some(resolved.clone());
                            }
                            return resolved;
                        }
                        _ => break,
                    }
                }
                let resolved = Operand::Value(current);
                // Path compression
                for &ci in &chain {
                    copy_map[ci] = Some(resolved.clone());
                }
                resolved
            }
        }
    }

    // Second pass: replace operands in all instructions and terminators.
    let mut changed = false;

    for block in &mut func.blocks {
        for inst in &mut block.insts {
            inst.for_each_operand_mut(|op| {
                if let Operand::Value(v) = op {
                    let resolved = resolve(&mut copy_map, *v);
                    if resolved != *op {
                        *op = resolved;
                        changed = true;
                    }
                }
            });
        }
        block.terminator.for_each_operand_mut(|op| {
            if let Operand::Value(v) = op {
                let resolved = resolve(&mut copy_map, *v);
                if resolved != *op {
                    *op = resolved;
                    changed = true;
                }
            }
        });
    }

    changed
}

