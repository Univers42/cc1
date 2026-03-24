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
fn count_instructions(func: &IrFunction) -> usize {
    func.blocks.iter().map(|b| b.insts.len()).sum()
}

/// Check if a function is eligible for inlining (no excluded instructions).
fn is_eligible_for_inlining(func: &IrFunction) -> bool {
    for block in &func.blocks {
        for inst in &block.insts {
            match inst {
                Instruction::DynAlloca { .. }
                | Instruction::StackRestore { .. } => return false,
                Instruction::InlineAsm { .. } => {
                    // Allow inline asm, but could restrict in future
                }
                _ => {}
            }
        }
        // Check for indirect branches
        if matches!(block.terminator, Terminator::IndirectBr { .. }) {
            return false;
        }
    }
    true
}

/// Inline a specific call site. Returns true on success.
fn inline_call_site(
    module: &mut IrModule,
    caller_idx: usize,
    call_bi: usize,
    call_ii: usize,
    callee_idx: usize,
) -> bool {
    // Clone the callee's blocks (we need ownership).
    let callee = module.functions[callee_idx].clone();
    let caller = &mut module.functions[caller_idx];

    // Extract the call instruction.
    let call_inst = &caller.blocks[call_bi].insts[call_ii];
    let (call_result, call_args, call_ret_ty) = match call_inst {
        Instruction::Call { result, args, ret_ty, .. } => {
            (*result, args.clone(), ret_ty.clone())
        }
        _ => return false,
    };

    // Allocate new ValueIds and BlockIds for cloned callee.
    let value_offset = caller.value_count();
    let block_offset = caller.block_count() as u32;

    let remap_value = |v: ValueId| -> ValueId {
        ValueId(v.0 + value_offset)
    };
    let remap_block = |b: BlockId| -> BlockId {
        BlockId(b.0 + block_offset)
    };

    let remap_operand = |op: &Operand| -> Operand {
        match op {
            Operand::Value(v) => Operand::Value(remap_value(*v)),
            Operand::Const(c) => Operand::Const(c.clone()),
            Operand::Global(g) => Operand::Global(g.clone()),
            Operand::Label(b) => Operand::Label(remap_block(*b)),
        }
    };

    // Create a merge block for the return value.
    let merge_bid = caller.create_block("inline.merge");

    // Clone and remap callee blocks.
    let mut cloned_blocks = Vec::new();
    for block in &callee.blocks {
        let new_bid = caller.create_block(&format!("inline.{}", block.label));
        let nb = caller.block_mut(new_bid);

        // Remap instructions
        for inst in &block.insts {
            let new_inst = remap_instruction(inst, &remap_operand, &remap_value, &remap_block);
            nb.insts.push(new_inst);
        }

        // Remap terminator
        nb.terminator = match &block.terminator {
            Terminator::Ret { value } => {
                // Return → branch to merge block
                Terminator::Br { target: merge_bid }
            }
            Terminator::Br { target } => {
                Terminator::Br { target: remap_block(*target) }
            }
            Terminator::CondBr { cond, true_bb, false_bb } => {
                Terminator::CondBr {
                    cond: remap_operand(cond),
                    true_bb: remap_block(*true_bb),
                    false_bb: remap_block(*false_bb),
                }
            }
            Terminator::Switch { discr, ty, default, cases } => {
                Terminator::Switch {
                    discr: remap_operand(discr),
                    ty: ty.clone(),
                    default: remap_block(*default),
                    cases: cases.iter().map(|(v, b)| (*v, remap_block(*b))).collect(),
                }
            }
            other => other.clone(),
        };

        cloned_blocks.push(new_bid);
    }

    // Wire arguments: insert stores for each parameter's alloca.
    // For simplicity, we create Copy instructions from args to param values.
    let callee_entry = if !cloned_blocks.is_empty() {
        cloned_blocks[0]
    } else {
        return false;
    };

    // Insert argument copies at the beginning of the callee entry
    let mut arg_copies = Vec::new();
    for (i, param) in callee.params.iter().enumerate() {
        if i < call_args.len() {
            let remapped_param = remap_value(param.value);
            arg_copies.push(Instruction::Copy {
                result: remapped_param,
                src: call_args[i].0.clone(),
            });
        }
    }
    // Prepend to callee entry
    let entry_insts = &mut caller.block_mut(callee_entry).insts;
    let old_insts = std::mem::take(entry_insts);
    *entry_insts = arg_copies;
    entry_insts.extend(old_insts);

    // Handle return value: collect return values for phi in merge block.
    let mut return_values: Vec<(BlockId, Operand)> = Vec::new();
    for (i, block) in callee.blocks.iter().enumerate() {
        if let Terminator::Ret { value } = &block.terminator {
            let from_block = cloned_blocks[i];
            let ret_op = match value {
                Some(op) => remap_operand(op),
                None => Operand::Const(ConstValue::Undef),
            };
            return_values.push((from_block, ret_op));
        }
    }

    // Create phi or copy for return value in merge block.
    if !call_ret_ty.is_void() && !return_values.is_empty() {
        let ret_val = if return_values.len() == 1 {
            Instruction::Copy {
                result: call_result,
                src: return_values[0].1.clone(),
            }
        } else {
            Instruction::Phi {
                result: call_result,
                ty: call_ret_ty.clone(),
                incoming: return_values,
            }
        };
        caller.block_mut(merge_bid).insts.push(ret_val);
    }

    // Split the caller block at the call site.
    // Instructions after the call go into the merge block.
    let split_idx = call_bi.min(caller.blocks.len() - 1);
    let after_call = caller.blocks[split_idx]
        .insts
        .split_off(call_ii + 1);
    let old_term = caller.blocks[call_bi].terminator.clone();
    caller.block_mut(merge_bid).insts.extend(after_call);
    caller.block_mut(merge_bid).terminator = old_term;

    // Replace the call instruction with a branch to the callee entry.
    caller.blocks[call_bi].insts.truncate(call_ii);
    caller.blocks[call_bi].terminator = Terminator::Br { target: callee_entry };

    // Bump the value counter to account for cloned values.
    let max_callee_value = callee.value_count();
    for _ in 0..(max_callee_value + callee.params.len() as u32 + 1) {
        caller.alloc_value();
    }

    true
}

/// Remap an instruction's values and blocks.
fn remap_instruction(
    inst: &Instruction,
    remap_op: &dyn Fn(&Operand) -> Operand,
    remap_val: &dyn Fn(ValueId) -> ValueId,
    remap_blk: &dyn Fn(BlockId) -> BlockId,
) -> Instruction {
    match inst {
        Instruction::Alloca { result, ty, align } => Instruction::Alloca {
            result: remap_val(*result),
            ty: ty.clone(),
            align: *align,
        },
        Instruction::DynAlloca { result, ty, count } => Instruction::DynAlloca {
            result: remap_val(*result),
            ty: ty.clone(),
            count: remap_op(count),
        },
        Instruction::Store { addr, value, ty } => Instruction::Store {
            addr: remap_op(addr),
            value: remap_op(value),
            ty: ty.clone(),
        },
        Instruction::Load { result, addr, ty } => Instruction::Load {
            result: remap_val(*result),
            addr: remap_op(addr),
            ty: ty.clone(),
        },
        Instruction::BinOp { result, op, lhs, rhs, ty } => Instruction::BinOp {
            result: remap_val(*result),
            op: *op,
            lhs: remap_op(lhs),
            rhs: remap_op(rhs),
            ty: ty.clone(),
        },
        Instruction::UnaryOp { result, op, operand, ty } => Instruction::UnaryOp {
            result: remap_val(*result),
            op: *op,
            operand: remap_op(operand),
            ty: ty.clone(),
        },
        Instruction::Icmp { result, pred, lhs, rhs, ty } => Instruction::Icmp {
            result: remap_val(*result),
            pred: *pred,
            lhs: remap_op(lhs),
            rhs: remap_op(rhs),
            ty: ty.clone(),
        },
        Instruction::Fcmp { result, pred, lhs, rhs, ty } => Instruction::Fcmp {
            result: remap_val(*result),
            pred: *pred,
            lhs: remap_op(lhs),
            rhs: remap_op(rhs),
            ty: ty.clone(),
        },
        Instruction::Cast { result, kind, src, src_ty, dst_ty } => Instruction::Cast {
            result: remap_val(*result),
            kind: *kind,
            src: remap_op(src),
            src_ty: src_ty.clone(),
            dst_ty: dst_ty.clone(),
        },
        Instruction::Call { result, callee, args, ret_ty, is_variadic } => Instruction::Call {
            result: remap_val(*result),
            callee: callee.clone(),
            args: args.iter().map(|(op, ty)| (remap_op(op), ty.clone())).collect(),
            ret_ty: ret_ty.clone(),
            is_variadic: *is_variadic,
        },
        Instruction::CallIndirect { result, func_ptr, args, ret_ty, is_variadic } => {
            Instruction::CallIndirect {
                result: remap_val(*result),
                func_ptr: remap_op(func_ptr),
                args: args.iter().map(|(op, ty)| (remap_op(op), ty.clone())).collect(),
                ret_ty: ret_ty.clone(),
                is_variadic: *is_variadic,
            }
        }
        Instruction::GetElementPtr { result, base, offset, elem_ty } => {
            Instruction::GetElementPtr {
                result: remap_val(*result),
                base: remap_op(base),
                offset: remap_op(offset),
                elem_ty: elem_ty.clone(),
            }
        }
        Instruction::GlobalAddr { result, name } => Instruction::GlobalAddr {
            result: remap_val(*result),
            name: name.clone(),
        },
        Instruction::LabelAddr { result, block } => Instruction::LabelAddr {
            result: remap_val(*result),
            block: remap_blk(*block),
        },
        Instruction::Select { result, cond, true_val, false_val, ty } => Instruction::Select {
            result: remap_val(*result),
            cond: remap_op(cond),
            true_val: remap_op(true_val),
            false_val: remap_op(false_val),
            ty: ty.clone(),
        },
        Instruction::Copy { result, src } => Instruction::Copy {
            result: remap_val(*result),
            src: remap_op(src),
        },
        Instruction::Phi { result, ty, incoming } => Instruction::Phi {
            result: remap_val(*result),
            ty: ty.clone(),
            incoming: incoming
                .iter()
                .map(|(b, op)| (remap_blk(*b), remap_op(op)))
                .collect(),
        },
        Instruction::AtomicLoad { result, addr, ty, ordering } => Instruction::AtomicLoad {
            result: remap_val(*result),
            addr: remap_op(addr),
            ty: ty.clone(),
            ordering: *ordering,
        },
        Instruction::AtomicStore { addr, value, ty, ordering } => Instruction::AtomicStore {
            addr: remap_op(addr),
            value: remap_op(value),
            ty: ty.clone(),
            ordering: *ordering,
        },
        Instruction::AtomicRmw { result, op, addr, value, ty, ordering } => {
            Instruction::AtomicRmw {
                result: remap_val(*result),
                op: *op,
                addr: remap_op(addr),
                value: remap_op(value),
                ty: ty.clone(),
                ordering: *ordering,
            }
        }
        Instruction::AtomicCmpxchg {
            result, addr, expected, desired, ty,
            success_ordering, failure_ordering,
        } => Instruction::AtomicCmpxchg {
            result: remap_val(*result),
            addr: remap_op(addr),
            expected: remap_op(expected),
            desired: remap_op(desired),
            ty: ty.clone(),
            success_ordering: *success_ordering,
            failure_ordering: *failure_ordering,
        },
        Instruction::StackRestore { saved_sp } => Instruction::StackRestore {
            saved_sp: remap_op(saved_sp),
        },
        Instruction::InlineAsm {
            result, template, constraints, operands,
            has_side_effects, align_stack,
        } => Instruction::InlineAsm {
            result: remap_val(*result),
            template: template.clone(),
            constraints: constraints.clone(),
            operands: operands.iter().map(|(op, ty)| (remap_op(op), ty.clone())).collect(),
            has_side_effects: *has_side_effects,
            align_stack: *align_stack,
        },
        Instruction::Nop => Instruction::Nop,
    }
}

