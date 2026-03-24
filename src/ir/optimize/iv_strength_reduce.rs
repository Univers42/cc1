// ir/optimize/iv_strength_reduce.rs — Induction variable strength reduction.
//
// Detects induction variables in natural loops and replaces expensive
// derived expressions (multiplications of the IV) with cheaper additions
// using a "strength-reduced" shadow variable.
//
// Uses a shared CfgAnalysis for loop/dominator information.

use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::IrFunction;
use crate::ir::optimize::loop_analysis::CfgAnalysis;
use crate::ir::types::*;
use std::collections::{HashMap, HashSet};

/// Run IV strength reduction using a pre-computed CfgAnalysis.
pub fn iv_strength_reduce(func: &mut IrFunction, cfg: &CfgAnalysis) -> bool {
    let mut changed = false;

    // For each natural loop in the function.
    for lp in &cfg.loops {
        changed |= process_loop(func, lp, cfg);
    }

    changed
}

/// Process a single natural loop.
fn process_loop(
    func: &mut IrFunction,
    lp: &crate::ir::optimize::loop_analysis::NaturalLoop,
    _cfg: &CfgAnalysis,
) -> bool {
    let preheader = match lp.preheader {
        Some(p) => p,
        None => return false,
    };

    let mut changed = false;

    // Find basic induction variables (BIVs).
    // A BIV is a value defined in the loop whose only modification is
    // `iv = iv + c` or `iv = iv - c` where c is loop-invariant.
    let bivs = find_basic_ivs(func, lp);

    // For each BIV, find derived expressions and strength-reduce them.
    for biv in &bivs {
        changed |= reduce_derived(func, biv, lp, preheader);
    }

    changed
}

#[derive(Debug)]
struct BasicIV {
    /// The phi node that defines the IV.
    phi_val: ValueId,
    /// The value after increment (phi input from inside the loop).
    inc_val: ValueId,
    /// The step constant.
    step: i64,
    /// The initial value operand (from outside the loop).
    init: Operand,
    /// The type of the IV.
    ty: IrType,
    /// Block and instruction index of the increment.
    inc_block: usize,
    inc_idx: usize,
}

/// Find all basic induction variables in the loop.
fn find_basic_ivs(
    func: &IrFunction,
    lp: &crate::ir::optimize::loop_analysis::NaturalLoop,
) -> Vec<BasicIV> {
    let mut bivs = Vec::new();
    let header_idx = lp.header.0 as usize;

    if header_idx >= func.blocks.len() {
        return bivs;
    }

    // Look at phi nodes in the header.
    for inst in &func.blocks[header_idx].insts {
        if let Instruction::Phi { result, ty, incoming } = inst {
            // Need exactly two incoming edges: one from outside (init),
            // one from inside the loop (increment).
            if incoming.len() != 2 {
                continue;
            }

            let mut init_op: Option<Operand> = None;
            let mut loop_op: Option<(BlockId, Operand)> = None;

            for (blk, op) in incoming {
                if lp.body.contains(blk) {
                    loop_op = Some((*blk, op.clone()));
                } else {
                    init_op = Some(op.clone());
                }
            }

            let (loop_blk, loop_val_op) = match loop_op {
                Some(x) => x,
                None => continue,
            };
            let init = match init_op {
                Some(x) => x,
                None => continue,
            };

            // The loop operand must be a Value.
            let inc_val = match &loop_val_op {
                Operand::Value(v) => *v,
                _ => continue,
            };

            // Find the increment instruction.
            let (step, inc_bi, inc_ii) = match find_increment(func, inc_val, *result, lp) {
                Some(x) => x,
                None => continue,
            };

            // Only handle integer types.
            if !ty.is_integer() {
                continue;
            }

            bivs.push(BasicIV {
                phi_val: *result,
                inc_val,
                step,
                init,
                ty: ty.clone(),
                inc_block: inc_bi,
                inc_idx: inc_ii,
            });
        }
    }

    bivs
}

/// Find the increment instruction for a potential BIV.
/// Returns (step_constant, block_index, instruction_index).
fn find_increment(
    func: &IrFunction,
    inc_val: ValueId,
    phi_val: ValueId,
    lp: &crate::ir::optimize::loop_analysis::NaturalLoop,
) -> Option<(i64, usize, usize)> {
    for bi in 0..func.blocks.len() {
        let bid = BlockId(bi as u32);
        if !lp.body.contains(&bid) {
            continue;
        }

        for (ii, inst) in func.blocks[bi].insts.iter().enumerate() {
            if let Instruction::BinOp { result, op, lhs, rhs, .. } = inst {
                if *result != inc_val {
                    continue;
                }

                match op {
                    BinOpKind::Add => {
                        // iv + c
                        if let Operand::Value(v) = lhs {
                            if *v == phi_val {
                                if let Some(c) = const_to_i64(rhs) {
                                    return Some((c, bi, ii));
                                }
                            }
                        }
                        // c + iv
                        if let Operand::Value(v) = rhs {
                            if *v == phi_val {
                                if let Some(c) = const_to_i64(lhs) {
                                    return Some((c, bi, ii));
                                }
                            }
                        }
                    }
                    BinOpKind::Sub => {
                        // iv - c
                        if let Operand::Value(v) = lhs {
                            if *v == phi_val {
                                if let Some(c) = const_to_i64(rhs) {
                                    return Some((-c, bi, ii));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Find derived expressions of the form `mul iv, c` and reduce them
/// to incremental additions.
fn reduce_derived(
    func: &mut IrFunction,
    biv: &BasicIV,
    lp: &crate::ir::optimize::loop_analysis::NaturalLoop,
    preheader: BlockId,
) -> bool {
    let mut changed = false;

    // Find all `mul biv.phi_val, c` or `mul biv.inc_val, c` in the loop.
    let mut muls: Vec<(usize, usize, ValueId, i64, IrType)> = Vec::new();

    for bi in 0..func.blocks.len() {
        let bid = BlockId(bi as u32);
        if !lp.body.contains(&bid) {
            continue;
        }

        for (ii, inst) in func.blocks[bi].insts.iter().enumerate() {
            if let Instruction::BinOp { result, op: BinOpKind::Mul, lhs, rhs, ty } = inst {
                // mul iv, c
                if let Operand::Value(v) = lhs {
                    if *v == biv.phi_val || *v == biv.inc_val {
                        if let Some(c) = const_to_i64(rhs) {
                            muls.push((bi, ii, *result, c, ty.clone()));
                        }
                    }
                }
                // mul c, iv
                if let Operand::Value(v) = rhs {
                    if *v == biv.phi_val || *v == biv.inc_val {
                        if let Some(c) = const_to_i64(lhs) {
                            muls.push((bi, ii, *result, c, ty.clone()));
                        }
                    }
                }
            }
        }
    }

    for (mul_bi, mul_ii, mul_result, factor, ty) in muls {
        // Create a shadow IV:
        // preheader: shadow_init = init * factor
        // header: shadow_phi = phi [preheader: shadow_init, latch: shadow_inc]
        // after_biv_inc: shadow_inc = shadow_phi + step * factor

        let shadow_init = func.alloc_value();
        let shadow_phi = func.alloc_value();
        let shadow_inc = func.alloc_value();

        let step_times_factor = biv.step * factor;

        // Insert init computation in preheader.
        let pre_idx = preheader.0 as usize;
        if pre_idx >= func.blocks.len() {
            continue;
        }

        // Insert before the terminator.
        func.blocks[pre_idx].insts.push(Instruction::BinOp {
            result: shadow_init,
            op: BinOpKind::Mul,
            lhs: biv.init.clone(),
            rhs: Operand::Const(i64_to_const(factor, &ty)),
            ty: ty.clone(),
        });

        // Insert phi in header.
        let header_idx = lp.header.0 as usize;
        let latch = if !lp.latches.is_empty() {
            lp.latches[0]
        } else {
            continue;
        };

        func.blocks[header_idx].insts.insert(0, Instruction::Phi {
            result: shadow_phi,
            ty: ty.clone(),
            incoming: vec![
                (preheader, Operand::Value(shadow_init)),
                (latch, Operand::Value(shadow_inc)),
            ],
        });

        // Insert shadow increment after the BIV increment.
        let inc_pos = biv.inc_idx + 1;
        let inc_bi = biv.inc_block;
        if inc_bi < func.blocks.len() && inc_pos <= func.blocks[inc_bi].insts.len() {
            func.blocks[inc_bi].insts.insert(inc_pos, Instruction::BinOp {
                result: shadow_inc,
                op: BinOpKind::Add,
                lhs: Operand::Value(shadow_phi),
                rhs: Operand::Const(i64_to_const(step_times_factor, &ty)),
                ty: ty.clone(),
            });
        }

        // Replace the multiplication with a copy of the shadow phi.
        if mul_bi < func.blocks.len() && mul_ii < func.blocks[mul_bi].insts.len() {
            func.blocks[mul_bi].insts[mul_ii] = Instruction::Copy {
                result: mul_result,
                src: Operand::Value(shadow_phi),
            };
            changed = true;
        }
    }

    changed
}

fn const_to_i64(op: &Operand) -> Option<i64> {
    match op {
        Operand::Const(ConstValue::I8(v)) => Some(*v as i64),
        Operand::Const(ConstValue::U8(v)) => Some(*v as i64),
        Operand::Const(ConstValue::I16(v)) => Some(*v as i64),
        Operand::Const(ConstValue::U16(v)) => Some(*v as i64),
        Operand::Const(ConstValue::I32(v)) => Some(*v as i64),
        Operand::Const(ConstValue::U32(v)) => Some(*v as i64),
        Operand::Const(ConstValue::I64(v)) => Some(*v),
        Operand::Const(ConstValue::U64(v)) => Some(*v as i64),
        _ => None,
    }
}

fn i64_to_const(v: i64, ty: &IrType) -> ConstValue {
    match ty {
        IrType::I8 => ConstValue::I8(v as i8),
        IrType::U8 => ConstValue::U8(v as u8),
        IrType::I16 => ConstValue::I16(v as i16),
        IrType::U16 => ConstValue::U16(v as u16),
        IrType::I32 => ConstValue::I32(v as i32),
        IrType::U32 => ConstValue::U32(v as u32),
        IrType::I64 => ConstValue::I64(v),
        IrType::U64 => ConstValue::U64(v as u64),
        _ => ConstValue::I64(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::instruction::Terminator;
    use crate::ir::module::IrFunction;

    #[test]
    fn test_find_basic_ivs_simple() {
        let mut f = IrFunction::new("test", IrType::I32, Linkage::External);
        let preheader = f.create_block("preheader");
        let header = f.create_block("header");
        let body = f.create_block("body");
        let exit = f.create_block("exit");

        let iv = f.alloc_value();
        let iv_inc = f.alloc_value();
        let cmp = f.alloc_value();

        // preheader → header
        f.block_mut(preheader).set_terminator(Terminator::Br { target: header });

        // header: iv = phi [preheader: 0, body: iv_inc]
        f.block_mut(header).push(Instruction::Phi {
            result: iv,
            ty: IrType::I32,
            incoming: vec![
                (preheader, Operand::Const(ConstValue::I32(0))),
                (body, Operand::Value(iv_inc)),
            ],
        });
        f.block_mut(header).push(Instruction::Icmp {
            result: cmp,
            pred: IcmpPred::Slt,
            lhs: Operand::Value(iv),
            rhs: Operand::Const(ConstValue::I32(100)),
            ty: IrType::I32,
        });
        f.block_mut(header).set_terminator(Terminator::CondBr {
            cond: Operand::Value(cmp),
            true_bb: body,
            false_bb: exit,
        });

        // body: iv_inc = iv + 1; br header
        f.block_mut(body).push(Instruction::BinOp {
            result: iv_inc,
            op: BinOpKind::Add,
            lhs: Operand::Value(iv),
            rhs: Operand::Const(ConstValue::I32(1)),
            ty: IrType::I32,
        });
        f.block_mut(body).set_terminator(Terminator::Br { target: header });

        // exit
        f.block_mut(exit).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(iv)),
        });

        let body_set: HashSet<BlockId> = vec![header, body].into_iter().collect();
        let lp = crate::ir::optimize::loop_analysis::NaturalLoop {
            header,
            body: body_set,
            preheader: Some(preheader),
            latches: vec![body],
        };

        let bivs = find_basic_ivs(&f, &lp);
        assert_eq!(bivs.len(), 1);
        assert_eq!(bivs[0].phi_val, iv);
        assert_eq!(bivs[0].step, 1);
    }
}
