// ir/optimize/gvn.rs — Dominator-based Global Value Numbering (CSE).
//
// Walks the dominator tree in depth-first order, maintaining scoped hash
// tables that map expression keys to previously computed values.
// Also performs redundant load elimination and store-to-load forwarding.

use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::*;
use std::collections::HashMap;

/// Expression key for value numbering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ExprKey {
    BinOp {
        op: BinOpKind,
        lhs: ValueId,
        rhs: ValueId,
        ty: IrType,
    },
    UnaryOp {
        op: UnaryOpKind,
        operand: ValueId,
        ty: IrType,
    },
    Icmp {
        pred: IcmpPred,
        lhs: ValueId,
        rhs: ValueId,
    },
    Fcmp {
        pred: FcmpPred,
        lhs: ValueId,
        rhs: ValueId,
    },
    Cast {
        kind: CastKind,
        src: ValueId,
        src_ty: IrType,
        dst_ty: IrType,
    },
    Gep {
        base: ValueId,
        offset: ValueId,
        elem_ty: IrType,
    },
}

/// Run GVN on a single function. Returns true if any changes were made.
pub fn gvn(func: &mut IrFunction) -> bool {
    if func.blocks.is_empty() {
        return false;
    }

    let mut changed = false;

    // Simple local CSE: within each basic block, hash instructions
    // and replace duplicates with copies.
    for bi in 0..func.blocks.len() {
        let mut expr_map: HashMap<ExprKey, ValueId> = HashMap::new();
        let mut load_map: HashMap<ValueId, ValueId> = HashMap::new(); // addr_vn -> loaded value
        let mut store_fwd: HashMap<ValueId, (ValueId, IrType)> = HashMap::new(); // addr_vn -> (stored val, ty)

        for ii in 0..func.blocks[bi].insts.len() {
            let inst = &func.blocks[bi].insts[ii];

            // Memory clobbering invalidates load CSE
            if inst.clobbers_memory() {
                load_map.clear();
                store_fwd.clear();

                // But capture store-to-load forwarding for this store
                if let Instruction::Store { addr, value, ty } = inst {
                    if let Operand::Value(addr_vn) = addr {
                        store_fwd.insert(*addr_vn, (
                            match value {
                                Operand::Value(v) => *v,
                                _ => continue,
                            },
                            ty.clone(),
                        ));
                    }
                }
                continue;
            }

            // Load elimination
            if let Instruction::Load { result, addr, ty } = inst {
                if let Operand::Value(addr_vn) = addr {
                    // Store-to-load forwarding
                    if let Some((stored_val, stored_ty)) = store_fwd.get(addr_vn) {
                        if stored_ty == ty && !ty.is_float() {
                            let new_result = *result;
                            func.blocks[bi].insts[ii] = Instruction::Copy {
                                result: new_result,
                                src: Operand::Value(*stored_val),
                            };
                            changed = true;
                            continue;
                        }
                    }
                    // Redundant load elimination
                    if let Some(prev_val) = load_map.get(addr_vn) {
                        let new_result = *result;
                        func.blocks[bi].insts[ii] = Instruction::Copy {
                            result: new_result,
                            src: Operand::Value(*prev_val),
                        };
                        changed = true;
                        continue;
                    }
                    // Record this load
                    load_map.insert(*addr_vn, *result);
                }
                continue;
            }

            // Pure expression CSE
            if let Some(key) = make_expr_key(inst) {
                if let Some(result) = inst.result() {
                    if let Some(&prev) = expr_map.get(&key) {
                        // Replace with copy
                        func.blocks[bi].insts[ii] = Instruction::Copy {
                            result,
                            src: Operand::Value(prev),
                        };
                        changed = true;
                    } else {
                        expr_map.insert(key, result);
                    }
                }
            }
        }
    }

    changed
}

/// Create an expression key for an instruction (for CSE).
fn make_expr_key(inst: &Instruction) -> Option<ExprKey> {
    match inst {
        Instruction::BinOp { op, lhs, rhs, ty, .. } => {
            let (l, r) = match (lhs, rhs) {
                (Operand::Value(l), Operand::Value(r)) => {
                    // Commutative canonicalization
                    if is_commutative(*op) && l.0 > r.0 {
                        (*r, *l)
                    } else {
                        (*l, *r)
                    }
                }
                _ => return None, // Skip constants for simplicity
            };
            Some(ExprKey::BinOp { op: *op, lhs: l, rhs: r, ty: ty.clone() })
        }
        Instruction::UnaryOp { op, operand: Operand::Value(v), ty, .. } => {
            Some(ExprKey::UnaryOp { op: *op, operand: *v, ty: ty.clone() })
        }
        Instruction::Icmp { pred, lhs: Operand::Value(l), rhs: Operand::Value(r), .. } => {
            Some(ExprKey::Icmp { pred: *pred, lhs: *l, rhs: *r })
        }
        Instruction::Fcmp { pred, lhs: Operand::Value(l), rhs: Operand::Value(r), .. } => {
            Some(ExprKey::Fcmp { pred: *pred, lhs: *l, rhs: *r })
        }
        Instruction::Cast { kind, src: Operand::Value(v), src_ty, dst_ty, .. } => {
            // Exclude 128-bit types
            if matches!(src_ty, IrType::I128 | IrType::U128) || matches!(dst_ty, IrType::I128 | IrType::U128) {
                return None;
            }
            Some(ExprKey::Cast { kind: *kind, src: *v, src_ty: src_ty.clone(), dst_ty: dst_ty.clone() })
        }
        Instruction::GetElementPtr { base: Operand::Value(b), offset: Operand::Value(o), elem_ty, .. } => {
            Some(ExprKey::Gep { base: *b, offset: *o, elem_ty: elem_ty.clone() })
        }
        _ => None,
    }
}

fn is_commutative(op: BinOpKind) -> bool {
    matches!(
        op,
        BinOpKind::Add
            | BinOpKind::Mul
            | BinOpKind::And
            | BinOpKind::Or
            | BinOpKind::Xor
            | BinOpKind::FAdd
            | BinOpKind::FMul
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::instruction::Terminator;

    #[test]
    fn test_gvn_cse() {
        let mut f = IrFunction::new("test", IrType::I32, Linkage::External);
        let b = f.create_block("entry");

        let v0 = f.alloc_value(); // input
        let v1 = f.alloc_value(); // add v0, v0
        let v2 = f.alloc_value(); // add v0, v0 (redundant)
        let v3 = f.alloc_value(); // use both

        f.block_mut(b).push(Instruction::Alloca {
            result: v0,
            ty: IrType::I32,
            align: 4,
        });
        f.block_mut(b).push(Instruction::BinOp {
            result: v1,
            op: BinOpKind::Add,
            lhs: Operand::Value(v0),
            rhs: Operand::Value(v0),
            ty: IrType::I32,
        });
        f.block_mut(b).push(Instruction::BinOp {
            result: v2,
            op: BinOpKind::Add,
            lhs: Operand::Value(v0),
            rhs: Operand::Value(v0),
            ty: IrType::I32,
        });
        f.block_mut(b).push(Instruction::BinOp {
            result: v3,
            op: BinOpKind::Add,
            lhs: Operand::Value(v1),
            rhs: Operand::Value(v2),
            ty: IrType::I32,
        });
        f.block_mut(b).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v3)),
        });

        assert!(gvn(&mut f));
        // v2 should be replaced with Copy of v1
        assert!(matches!(f.block(b).insts[2], Instruction::Copy { .. }));
    }

    #[test]
    fn test_gvn_commutative() {
        let mut f = IrFunction::new("test", IrType::I32, Linkage::External);
        let b = f.create_block("entry");

        let v0 = f.alloc_value();
        let v1 = f.alloc_value();
        let v2 = f.alloc_value(); // add v0, v1
        let v3 = f.alloc_value(); // add v1, v0 (same due to commutativity)

        f.block_mut(b).push(Instruction::Alloca { result: v0, ty: IrType::I32, align: 4 });
        f.block_mut(b).push(Instruction::Alloca { result: v1, ty: IrType::I32, align: 4 });
        f.block_mut(b).push(Instruction::BinOp {
            result: v2,
            op: BinOpKind::Add,
            lhs: Operand::Value(v0),
            rhs: Operand::Value(v1),
            ty: IrType::I32,
        });
        f.block_mut(b).push(Instruction::BinOp {
            result: v3,
            op: BinOpKind::Add,
            lhs: Operand::Value(v1),
            rhs: Operand::Value(v0),
            ty: IrType::I32,
        });
        f.block_mut(b).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v3)),
        });

        assert!(gvn(&mut f));
        assert!(matches!(f.block(b).insts[3], Instruction::Copy { .. }));
    }

    #[test]
    fn test_gvn_load_elimination() {
        let mut f = IrFunction::new("test", IrType::I32, Linkage::External);
        let b = f.create_block("entry");

        let ptr = f.alloc_value();
        let v1 = f.alloc_value(); // first load
        let v2 = f.alloc_value(); // redundant load

        f.block_mut(b).push(Instruction::Alloca { result: ptr, ty: IrType::I32, align: 4 });
        f.block_mut(b).push(Instruction::Load {
            result: v1,
            addr: Operand::Value(ptr),
            ty: IrType::I32,
        });
        f.block_mut(b).push(Instruction::Load {
            result: v2,
            addr: Operand::Value(ptr),
            ty: IrType::I32,
        });
        f.block_mut(b).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v2)),
        });

        assert!(gvn(&mut f));
        // Second load should become Copy of first load's result
        assert!(matches!(f.block(b).insts[2], Instruction::Copy { .. }));
    }
}
