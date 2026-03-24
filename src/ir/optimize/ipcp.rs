// ir/optimize/ipcp.rs — Interprocedural constant propagation.
//
// Three optimizations:
// 1. Constant return propagation: if a function always returns the same
//    constant, replace all Call uses with that constant.
// 2. Dead call elimination: if a function is pure (no side effects,
//    no address taken) and its return value is unused, remove the call.
// 3. Constant argument propagation: if a function parameter always
//    receives the same constant across all call sites, replace uses
//    of that parameter in the callee body.

use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::IrModule;
use crate::ir::types::*;
use std::collections::{HashMap, HashSet};

/// Run IPCP on the module. Returns true if any changes were made.
pub fn ipcp(module: &mut IrModule) -> bool {
    let mut changed = false;

    changed |= constant_return_propagation(module);
    changed |= dead_call_elimination(module);
    changed |= constant_argument_propagation(module);

    changed
}

/// If a function always returns the same constant, replace call results.
fn constant_return_propagation(module: &mut IrModule) -> bool {
    let mut changed = false;

    // Collect functions that return a single constant.
    let mut const_returns: HashMap<String, ConstValue> = HashMap::new();

    for func in &module.functions {
        if func.blocks.is_empty() {
            continue; // Declaration
        }
        if func.linkage == Linkage::External {
            continue; // External linkage may be overridden
        }

        let mut ret_const: Option<ConstValue> = None;
        let mut is_uniform = true;

        for block in &func.blocks {
            if let Terminator::Ret { value: Some(op) } = &block.terminator {
                if let Operand::Const(c) = op {
                    match &ret_const {
                        None => ret_const = Some(c.clone()),
                        Some(prev) => {
                            if prev != c {
                                is_uniform = false;
                                break;
                            }
                        }
                    }
                } else {
                    is_uniform = false;
                    break;
                }
            }
        }

        if is_uniform {
            if let Some(c) = ret_const {
                const_returns.insert(func.name.clone(), c);
            }
        }
    }

    if const_returns.is_empty() {
        return false;
    }

    // Replace Call instructions with copies of the constant.
    for func in &mut module.functions {
        for block in &mut func.blocks {
            for inst in &mut block.insts {
                if let Instruction::Call { result, callee, ret_ty, .. } = inst {
                    if let Some(cv) = const_returns.get(callee.as_str()) {
                        if !ret_ty.is_void() {
                            // Replace with Copy of constant.
                            *inst = Instruction::Copy {
                                result: *result,
                                src: Operand::Const(cv.clone()),
                            };
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    changed
}

/// Remove calls to side-effect-free functions when the result is unused.
fn dead_call_elimination(module: &mut IrModule) -> bool {
    // Identify pure / side-effect-free functions.
    let mut pure_funcs: HashSet<String> = HashSet::new();

    for func in &module.functions {
        if func.blocks.is_empty() {
            continue;
        }
        if is_function_pure(func) {
            pure_funcs.insert(func.name.clone());
        }
    }

    if pure_funcs.is_empty() {
        return false;
    }

    // Collect address-taken functions (they cannot be DCE'd).
    let mut addr_taken: HashSet<String> = HashSet::new();
    for func in &module.functions {
        for block in &func.blocks {
            for inst in &block.insts {
                if let Instruction::GlobalAddr { name, .. } = inst {
                    addr_taken.insert(name.clone());
                }
            }
        }
    }

    let mut changed = false;

    // Build use counts for values, then remove dead calls.
    for fi in 0..module.functions.len() {
        let use_counts = build_use_counts(&module.functions[fi]);
        let func = &mut module.functions[fi];

        for block in &mut func.blocks {
            block.insts.retain(|inst| {
                if let Instruction::Call { result, callee, .. } = inst {
                    if pure_funcs.contains(callee.as_str())
                        && !addr_taken.contains(callee.as_str())
                    {
                        let uses = use_counts.get(result).copied().unwrap_or(0);
                        if uses == 0 {
                            changed = true;
                            return false; // Remove
                        }
                    }
                }
                true
            });
        }
    }

    changed
}

/// If a parameter receives the same constant at all call sites, propagate it.
fn constant_argument_propagation(module: &mut IrModule) -> bool {
    let mut changed = false;

    // For each function, analyze all call sites.
    let names: Vec<String> = module.functions.iter().map(|f| f.name.clone()).collect();

    for target_name in &names {
        // Find the target function.
        let target_idx = match module.functions.iter().position(|f| &f.name == target_name) {
            Some(i) => i,
            None => continue,
        };

        let func = &module.functions[target_idx];
        if func.blocks.is_empty() || func.params.is_empty() {
            continue;
        }
        if func.linkage == Linkage::External {
            continue; // Could be called from outside
        }

        let num_params = func.params.len();
        let param_values: Vec<ValueId> = func.params.iter().map(|p| p.value).collect();

        // Collect constant values for each parameter across all call sites.
        let mut param_consts: Vec<Option<ConstValue>> = vec![None; num_params];
        let mut param_varies: Vec<bool> = vec![false; num_params];

        for (fi, caller) in module.functions.iter().enumerate() {
            for block in &caller.blocks {
                for inst in &block.insts {
                    if let Instruction::Call { callee, args, .. } = inst {
                        if callee == target_name {
                            for (pi, (arg, _)) in args.iter().enumerate() {
                                if pi >= num_params {
                                    break;
                                }
                                if param_varies[pi] {
                                    continue;
                                }
                                if let Operand::Const(c) = arg {
                                    match &param_consts[pi] {
                                        None => param_consts[pi] = Some(c.clone()),
                                        Some(prev) => {
                                            if prev != c {
                                                param_varies[pi] = true;
                                            }
                                        }
                                    }
                                } else {
                                    param_varies[pi] = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Propagate constants into the function body.
        let target = &mut module.functions[target_idx];
        for pi in 0..num_params {
            if param_varies[pi] {
                continue;
            }
            if let Some(cv) = &param_consts[pi] {
                let pval = param_values[pi];
                let const_op = Operand::Const(cv.clone());

                // Replace uses of the parameter value.
                for block in &mut target.blocks {
                    for inst in &mut block.insts {
                        replace_value_in_instruction(inst, pval, &const_op);
                    }
                    replace_value_in_terminator(&mut block.terminator, pval, &const_op);
                }
                changed = true;
            }
        }
    }

    changed
}

/// Check if a function is pure (no side effects: no stores, no calls, no volatile).
fn is_function_pure(func: &crate::ir::module::IrFunction) -> bool {
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.has_side_effects() {
                return false;
            }
        }
    }
    true
}

/// Build a map of ValueId -> use count.
fn build_use_counts(func: &crate::ir::module::IrFunction) -> HashMap<ValueId, usize> {
    let mut counts: HashMap<ValueId, usize> = HashMap::new();

    for block in &func.blocks {
        for inst in &block.insts {
            inst.for_each_operand(|op| {
                if let Operand::Value(v) = op {
                    *counts.entry(*v).or_insert(0) += 1;
                }
            });
        }
        block.terminator.for_each_operand(|op| {
            if let Operand::Value(v) = op {
                *counts.entry(*v).or_insert(0) += 1;
            }
        });
    }

    counts
}

/// Replace all uses of a value in an instruction's operands.
fn replace_value_in_instruction(inst: &mut Instruction, old: ValueId, new_op: &Operand) {
    inst.for_each_operand_mut(|op| {
        if let Operand::Value(v) = op {
            if *v == old {
                *op = new_op.clone();
            }
        }
    });
}

/// Replace all uses of a value in a terminator's operands.
fn replace_value_in_terminator(term: &mut Terminator, old: ValueId, new_op: &Operand) {
    term.for_each_operand_mut(|op| {
        if let Operand::Value(v) = op {
            if *v == old {
                *op = new_op.clone();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::module::{IrFunction, IrModule};

    #[test]
    fn test_constant_return_propagation() {
        let mut module = IrModule::new("test");

        // callee: always returns 42
        let mut callee = IrFunction::new("callee", IrType::I32, Linkage::Internal);
        let b = callee.create_block("entry");
        callee.block_mut(b).set_terminator(Terminator::Ret {
            value: Some(Operand::Const(ConstValue::I32(42))),
        });
        module.functions.push(callee);

        // caller: calls callee
        let mut caller = IrFunction::new("caller", IrType::I32, Linkage::External);
        let b2 = caller.create_block("entry");
        let v = caller.alloc_value();
        caller.block_mut(b2).push(Instruction::Call {
            result: v,
            callee: "callee".to_string(),
            args: vec![],
            ret_ty: IrType::I32,
            is_variadic: false,
        });
        caller.block_mut(b2).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v)),
        });
        module.functions.push(caller);

        let changed = constant_return_propagation(&mut module);
        assert!(changed);

        // The call should be replaced with a copy of 42.
        let caller_fn = &module.functions[1];
        let inst = &caller_fn.blocks[0].insts[0];
        match inst {
            Instruction::Copy { src: Operand::Const(ConstValue::I32(42)), .. } => {}
            other => panic!("Expected Copy of 42, got {:?}", other),
        }
    }

    #[test]
    fn test_dead_call_elimination() {
        let mut module = IrModule::new("test");

        // pure function
        let mut callee = IrFunction::new("pure_fn", IrType::I32, Linkage::Internal);
        let b = callee.create_block("entry");
        let val = callee.alloc_value();
        callee.block_mut(b).push(Instruction::BinOp {
            result: val,
            op: BinOpKind::Add,
            lhs: Operand::Const(ConstValue::I32(1)),
            rhs: Operand::Const(ConstValue::I32(2)),
            ty: IrType::I32,
        });
        callee.block_mut(b).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(val)),
        });
        module.functions.push(callee);

        // caller: calls pure_fn but ignores result
        let mut caller = IrFunction::new("caller", IrType::Void, Linkage::External);
        let b2 = caller.create_block("entry");
        let unused = caller.alloc_value();
        caller.block_mut(b2).push(Instruction::Call {
            result: unused,
            callee: "pure_fn".to_string(),
            args: vec![],
            ret_ty: IrType::I32,
            is_variadic: false,
        });
        caller.block_mut(b2).set_terminator(Terminator::Ret { value: None });
        module.functions.push(caller);

        let changed = dead_call_elimination(&mut module);
        assert!(changed);
        assert!(module.functions[1].blocks[0].insts.is_empty());
    }

    #[test]
    fn test_constant_argument_propagation() {
        let mut module = IrModule::new("test");

        // callee with one param
        let mut callee = IrFunction::new("callee", IrType::I32, Linkage::Internal);
        let b = callee.create_block("entry");
        let param = callee.alloc_value();
        callee.params.push(crate::ir::module::IrParam { name: "x".to_string(), value: param, ty: IrType::I32 });
        let res = callee.alloc_value();
        callee.block_mut(b).push(Instruction::BinOp {
            result: res,
            op: BinOpKind::Add,
            lhs: Operand::Value(param),
            rhs: Operand::Const(ConstValue::I32(1)),
            ty: IrType::I32,
        });
        callee.block_mut(b).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(res)),
        });
        module.functions.push(callee);

        // caller: always passes 10
        let mut caller = IrFunction::new("caller", IrType::I32, Linkage::External);
        let b2 = caller.create_block("entry");
        let v = caller.alloc_value();
        caller.block_mut(b2).push(Instruction::Call {
            result: v,
            callee: "callee".to_string(),
            args: vec![(Operand::Const(ConstValue::I32(10)), IrType::I32)],
            ret_ty: IrType::I32,
            is_variadic: false,
        });
        caller.block_mut(b2).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v)),
        });
        module.functions.push(caller);

        let changed = constant_argument_propagation(&mut module);
        assert!(changed);

        // Check that callee's BinOp now uses const 10 instead of param value.
        let callee_fn = &module.functions[0];
        let inst = &callee_fn.blocks[0].insts[0];
        match inst {
            Instruction::BinOp { lhs: Operand::Const(ConstValue::I32(10)), .. } => {}
            other => panic!("Expected BinOp with const 10, got {:?}", other),
        }
    }
}
