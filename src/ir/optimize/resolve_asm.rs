// ir/optimize/resolve_asm.rs — Post-inline assembly symbol resolution.
//
// After inlining, some inline assembly operands may reference GlobalAddr
// values that were cloned from the callee. This pass scans InlineAsm
// instructions and ensures their operands point to valid global symbols.
//
// For each InlineAsm operand that is a value, trace it through
// GlobalAddr + GEP chains to produce a concrete symbol name. If the
// operand already references a valid global, leave it alone.

use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::*;
use std::collections::HashMap;

/// Run assembly symbol resolution on a function. Returns true if changes were made.
pub fn resolve_asm(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Build a map from value -> defining instruction for tracing.
    let defs = build_def_map(func);

    for bi in 0..func.blocks.len() {
        for ii in 0..func.blocks[bi].insts.len() {
            let inst = &func.blocks[bi].insts[ii];

            if let Instruction::InlineAsm { operands, .. } = inst {
                // Check if any operand references a value that can be resolved.
                let mut new_operands: Option<Vec<(Operand, IrType)>> = None;

                for (oi, (op, ty)) in operands.iter().enumerate() {
                    if let Operand::Value(v) = op {
                        // Try to trace through GlobalAddr/GEP chains.
                        if let Some(resolved) = trace_to_global(*v, &defs, func) {
                            if new_operands.is_none() {
                                new_operands = Some(operands.clone());
                            }
                            new_operands.as_mut().unwrap()[oi] = (resolved, ty.clone());
                        }
                    }
                }

                if let Some(new_ops) = new_operands {
                    // Replace the operands.
                    if let Instruction::InlineAsm {
                        result, template, constraints, operands: _,
                        has_side_effects, align_stack,
                    } = &func.blocks[bi].insts[ii]
                    {
                        func.blocks[bi].insts[ii] = Instruction::InlineAsm {
                            result: *result,
                            template: template.clone(),
                            constraints: constraints.clone(),
                            operands: new_ops,
                            has_side_effects: *has_side_effects,
                            align_stack: *align_stack,
                        };
                        changed = true;
                    }
                }
            }
        }
    }

    changed
}

/// Trace a value through its definition chain, looking for a GlobalAddr
/// or a GEP based on a GlobalAddr.
fn trace_to_global(
    val: ValueId,
    defs: &HashMap<ValueId, (usize, usize)>,
    func: &IrFunction,
) -> Option<Operand> {
    let mut current = val;
    let mut depth = 0;
    const MAX_DEPTH: usize = 16;

    loop {
        if depth >= MAX_DEPTH {
            return None;
        }
        depth += 1;

        let (bi, ii) = defs.get(&current)?;
        let inst = func.blocks.get(*bi)?.insts.get(*ii)?;

        match inst {
            Instruction::GlobalAddr { name, .. } => {
                return Some(Operand::Global(name.clone()));
            }
            Instruction::GetElementPtr { base, .. } => {
                // Follow the base pointer.
                match base {
                    Operand::Value(v) => current = *v,
                    Operand::Global(g) => return Some(Operand::Global(g.clone())),
                    _ => return None,
                }
            }
            Instruction::Copy { src, .. } => {
                match src {
                    Operand::Value(v) => current = *v,
                    Operand::Global(g) => return Some(Operand::Global(g.clone())),
                    _ => return None,
                }
            }
            Instruction::Cast { src, .. } => {
                // Follow through bitcasts/pointer casts.
                match src {
                    Operand::Value(v) => current = *v,
                    Operand::Global(g) => return Some(Operand::Global(g.clone())),
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
}

fn build_def_map(func: &IrFunction) -> HashMap<ValueId, (usize, usize)> {
    let mut map = HashMap::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.insts.iter().enumerate() {
            if let Some(v) = inst.result() {
                map.insert(v, (bi, ii));
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::instruction::Terminator;
    use crate::ir::module::IrFunction;

    #[test]
    fn test_resolve_global_addr() {
        let mut f = IrFunction::new("test", IrType::Void, Linkage::External);
        let b = f.create_block("entry");

        let g_addr = f.alloc_value();
        let asm_result = f.alloc_value();

        f.block_mut(b).push(Instruction::GlobalAddr {
            result: g_addr,
            name: "my_global".to_string(),
        });
        f.block_mut(b).push(Instruction::InlineAsm {
            result: asm_result,
            template: "mov $0, %%rax".to_string(),
            constraints: "r".to_string(),
            operands: vec![(Operand::Value(g_addr), IrType::Ptr)],
            has_side_effects: true,
            align_stack: false,
        });
        f.block_mut(b).set_terminator(Terminator::Ret { value: None });

        let changed = resolve_asm(&mut f);
        assert!(changed);

        // The asm operand should now reference the global directly.
        match &f.blocks[0].insts[1] {
            Instruction::InlineAsm { operands, .. } => {
                assert!(matches!(&operands[0].0, Operand::Global(g) if g == "my_global"));
            }
            _ => panic!("Expected InlineAsm"),
        }
    }

    #[test]
    fn test_resolve_through_copy() {
        let mut f = IrFunction::new("test", IrType::Void, Linkage::External);
        let b = f.create_block("entry");

        let g_addr = f.alloc_value();
        let copy_v = f.alloc_value();
        let asm_result = f.alloc_value();

        f.block_mut(b).push(Instruction::GlobalAddr {
            result: g_addr,
            name: "sym".to_string(),
        });
        f.block_mut(b).push(Instruction::Copy {
            result: copy_v,
            src: Operand::Value(g_addr),
        });
        f.block_mut(b).push(Instruction::InlineAsm {
            result: asm_result,
            template: "nop".to_string(),
            constraints: "r".to_string(),
            operands: vec![(Operand::Value(copy_v), IrType::Ptr)],
            has_side_effects: true,
            align_stack: false,
        });
        f.block_mut(b).set_terminator(Terminator::Ret { value: None });

        let changed = resolve_asm(&mut f);
        assert!(changed);

        match &f.blocks[0].insts[2] {
            Instruction::InlineAsm { operands, .. } => {
                assert!(matches!(&operands[0].0, Operand::Global(g) if g == "sym"));
            }
            _ => panic!("Expected InlineAsm"),
        }
    }

    #[test]
    fn test_no_change_already_resolved() {
        let mut f = IrFunction::new("test", IrType::Void, Linkage::External);
        let b = f.create_block("entry");
        let asm_result = f.alloc_value();

        f.block_mut(b).push(Instruction::InlineAsm {
            result: asm_result,
            template: "nop".to_string(),
            constraints: "".to_string(),
            operands: vec![(Operand::Global("already".to_string()), IrType::Ptr)],
            has_side_effects: true,
            align_stack: false,
        });
        f.block_mut(b).set_terminator(Terminator::Ret { value: None });

        let changed = resolve_asm(&mut f);
        assert!(!changed);
    }
}
