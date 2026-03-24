// ir/optimize/inline.rs — Function inlining with tiered size heuristics.
//
// Substitutes callee function bodies into call sites. Runs before the main
// optimization loop. After inlining, gnu_inline extern inline functions
// are converted to declarations.

use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::{IrFunction, IrModule};
use crate::ir::types::*;
use std::collections::HashMap;

/// Maximum instruction count for "tiny" functions (always inline).
const TINY_INSTR_LIMIT: usize = 5;
const TINY_BLOCK_LIMIT: usize = 1;

/// Maximum instruction count for "small" / static inline functions.
const SMALL_INSTR_LIMIT: usize = 20;
const SMALL_BLOCK_LIMIT: usize = 3;

/// Maximum instruction count for normal static functions.
const NORMAL_STATIC_INSTR_LIMIT: usize = 30;
const NORMAL_STATIC_BLOCK_LIMIT: usize = 4;

/// Per-caller budget limits.
const CALLER_BUDGET_INSTRS: usize = 200;
const CALLER_BUDGET_TOTAL: usize = 800;
const CALLER_HARD_CAP: usize = 500;
const CALLER_ABSOLUTE_CAP: usize = 1000;

/// Maximum inlining rounds per caller.
const MAX_ROUNDS: usize = 200;

/// Run the inlining pass on the module.
pub fn run_inline(module: &mut IrModule, _timing: bool) {
    // Build a map of function names to function indices.
    let name_to_idx: HashMap<String, usize> = module
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.clone(), i))
        .collect();

    // Phase 1: Main inlining loop.
    // Process each caller function.
    let num_funcs = module.functions.len();
    for caller_idx in 0..num_funcs {
        if module.functions[caller_idx].blocks.is_empty() {
            continue; // Declaration only
        }

        let mut inlined_instrs = 0usize;
        let mut total_inlined = 0usize;

        for _round in 0..MAX_ROUNDS {
            let mut did_inline = false;

            // Scan for call sites in the caller.
            let caller = &module.functions[caller_idx];
            let mut call_sites: Vec<(usize, usize, String)> = Vec::new(); // (block, inst, callee_name)

            for (bi, block) in caller.blocks.iter().enumerate() {
                for (ii, inst) in block.insts.iter().enumerate() {
                    if let Instruction::Call { callee, .. } = inst {
                        if let Some(&callee_idx) = name_to_idx.get(callee) {
                            if callee_idx != caller_idx { // no self-recursion
                                call_sites.push((bi, ii, callee.clone()));
                            }
                        }
                    }
                }
            }

            if call_sites.is_empty() {
                break;
            }

            // Try to inline each call site (process in reverse order
            // to maintain instruction indices).
            for (bi, ii, callee_name) in call_sites.into_iter().rev() {
                let callee_idx = match name_to_idx.get(&callee_name) {
                    Some(&i) => i,
                    None => continue,
                };

                // Check eligibility
                let callee = &module.functions[callee_idx];
                if callee.blocks.is_empty() {
                    continue; // Declaration only
                }
                if callee.is_variadic {
                    continue;
                }
                if !is_eligible_for_inlining(callee) {
                    continue;
                }

                // Check size heuristics
                let callee_instrs = count_instructions(callee);
                let callee_blocks = callee.blocks.len();
                let is_internal = matches!(callee.linkage, Linkage::Internal | Linkage::Private);

                let should_inline = if callee_instrs <= TINY_INSTR_LIMIT && callee_blocks <= TINY_BLOCK_LIMIT {
                    true // Tiny: always inline
                } else if callee_instrs <= SMALL_INSTR_LIMIT && callee_blocks <= SMALL_BLOCK_LIMIT {
                    inlined_instrs < CALLER_BUDGET_INSTRS && total_inlined < CALLER_BUDGET_TOTAL
                } else if is_internal && callee_instrs <= NORMAL_STATIC_INSTR_LIMIT && callee_blocks <= NORMAL_STATIC_BLOCK_LIMIT {
                    inlined_instrs < CALLER_BUDGET_INSTRS && total_inlined < CALLER_BUDGET_TOTAL
                } else {
                    false
                };

                if !should_inline {
                    continue;
                }

                // Budget check
                let caller_instrs = count_instructions(&module.functions[caller_idx]);
                if caller_instrs > CALLER_HARD_CAP {
                    continue;
                }
                if total_inlined + callee_instrs > CALLER_ABSOLUTE_CAP {
                    continue;
                }

                // Perform the inline
                if inline_call_site(module, caller_idx, bi, ii, callee_idx) {
                    inlined_instrs += callee_instrs;
                    total_inlined += callee_instrs;
                    did_inline = true;
                }
            }

            if !did_inline {
                break;
            }
        }
    }
}

/// Count total instructions in a function.
