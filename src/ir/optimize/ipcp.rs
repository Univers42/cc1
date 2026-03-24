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
