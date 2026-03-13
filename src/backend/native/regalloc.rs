// backend/native/regalloc.rs — Linear-scan register allocator.
//
// Phase 1: Assign callee-saved regs to values that span calls.
// Phase 2: Assign caller-saved regs to values that don't span calls.
// Phase 3: Spill remaining values to the stack.

use std::collections::HashMap;
use crate::ir::instruction::Instruction;
use crate::ir::module::IrFunction;
use crate::ir::types::ValueId;
use crate::backend::native::state::{CodegenState, ValueLocation};

/// Liveness interval for an IR value.
#[derive(Debug, Clone)]
pub struct LiveInterval {
    pub value: ValueId,
    /// First use (instruction index in linearized order).
    pub start: u32,
    /// Last use (instruction index in linearized order).
    pub end: u32,
    /// Whether this value is live across a call instruction.
    pub spans_call: bool,
}

/// Compute liveness intervals for all values in a function.
pub fn compute_liveness(func: &IrFunction) -> Vec<LiveInterval> {
    let mut intervals: HashMap<u32, LiveInterval> = HashMap::new();
    let mut inst_idx: u32 = 0;
    let mut call_indices: Vec<u32> = Vec::new();

    // Linearize all instructions across all blocks
    for bb in &func.blocks {
        for inst in &bb.insts {
            // Record if this is a call
            if matches!(inst, Instruction::Call { .. } | Instruction::CallIndirect { .. }) {
                call_indices.push(inst_idx);
            }

            // Record definition
            if let Some(result) = inst.result() {
                intervals.entry(result.0).or_insert(LiveInterval {
                    value: result,
                    start: inst_idx,
                    end: inst_idx,
                    spans_call: false,
                });
            }

            // Record uses
            inst.for_each_value_use(|vid| {
                if let Some(interval) = intervals.get_mut(&vid.0) {
                    interval.end = interval.end.max(inst_idx);
                }
            });

            inst_idx += 1;
        }

        // Process terminator uses
        bb.terminator.for_each_operand(|op| {
            if let crate::ir::types::Operand::Value(vid) = op {
                if let Some(interval) = intervals.get_mut(&vid.0) {
                    interval.end = interval.end.max(inst_idx);
                }
            }
        });
        inst_idx += 1; // count the terminator
    }

    // Mark intervals that span calls
    let mut results: Vec<LiveInterval> = intervals.into_values().collect();
    for interval in &mut results {
        for &ci in &call_indices {
            if interval.start < ci && ci < interval.end {
                interval.spans_call = true;
                break;
            }
        }
    }

    // Sort by start position
    results.sort_by_key(|i| i.start);
    results
}

/// Perform linear-scan register allocation.
///
/// Assigns registers from the given pools:
/// - `callee_saved`: registers preserved across calls (rbx, r12-r15)
/// - `caller_saved`: registers clobbered by calls (rax, rcx, rdx, rsi, rdi, r8-r11)
pub fn allocate_registers(
    state: &mut CodegenState,
    intervals: &[LiveInterval],
    callee_saved: &[&str],
    caller_saved: &[&str],
) {
    let mut free_callee: Vec<String> = callee_saved.iter().map(|s| s.to_string()).collect();
    let mut free_caller: Vec<String> = caller_saved.iter().map(|s| s.to_string()).collect();
    let mut active: Vec<(LiveInterval, String)> = Vec::new();

    for interval in intervals {
        // Expire old intervals
        active.retain(|(ai, reg)| {
            if ai.end <= interval.start {
                // This interval has expired — free its register
                if callee_saved.contains(&reg.as_str()) {
                    free_callee.push(reg.clone());
                } else {
                    free_caller.push(reg.clone());
                }
                false
            } else {
                true
            }
        });

        // Try to allocate a register
        let reg = if interval.spans_call {
            // Prefer callee-saved for call-spanning values
            free_callee.pop().or_else(|| free_caller.pop())
        } else {
            // Prefer caller-saved for non-call-spanning values
            free_caller.pop().or_else(|| free_callee.pop())
        };

        if let Some(reg) = reg {
            // Track callee-saved usage for save/restore
            if callee_saved.contains(&reg.as_str())
                && !state.callee_saved_used.contains(&reg)
            {
                state.callee_saved_used.push(reg.clone());
            }
            state.set_value_location(interval.value, ValueLocation::Reg(reg.clone()));
            active.push((interval.clone(), reg));
        } else {
            // Spill to stack
            let slot_idx = state.alloc_stack_slot(8, 8);
            let offset = state.stack_slots[slot_idx].offset;
            state.set_value_location(interval.value, ValueLocation::Stack(offset));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::module::*;
    use crate::ir::instruction::*;
    use crate::ir::types::*;
    use crate::target::Target;

    #[test]
    fn test_compute_liveness_simple() {
        let mut func = IrFunction::new("test", IrType::I32, Linkage::External);
        let entry = func.create_block("entry");
        let v0 = func.alloc_value();
        let v1 = func.alloc_value();
        let v2 = func.alloc_value();

        func.block_mut(entry).push(Instruction::Alloca {
            result: v0,
            ty: IrType::I32,
            align: 4,
        });
        func.block_mut(entry).push(Instruction::Load {
            result: v1,
            addr: Operand::Value(v0),
            ty: IrType::I32,
        });
        func.block_mut(entry).push(Instruction::BinOp {
            result: v2,
            op: BinOpKind::Add,
            lhs: Operand::Value(v1),
            rhs: Operand::Const(ConstValue::I32(1)),
            ty: IrType::I32,
        });
        func.block_mut(entry).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(v2)),
        });

        let intervals = compute_liveness(&func);
        assert_eq!(intervals.len(), 3);
        // v0 defined at 0, used at 1
        let iv0 = intervals.iter().find(|i| i.value == v0).unwrap();
        assert_eq!(iv0.start, 0);
        assert!(iv0.end >= 1);
    }

    #[test]
    fn test_allocate_registers_simple() {
        let mut state = CodegenState::new(Target::X86_64);
        state.begin_function("test");

        let intervals = vec![
            LiveInterval {
                value: ValueId(0),
                start: 0,
                end: 3,
                spans_call: false,
            },
            LiveInterval {
                value: ValueId(1),
                start: 1,
                end: 2,
                spans_call: false,
            },
        ];

        let callee_saved = &["rbx", "r12", "r13"];
        let caller_saved = &["rax", "rcx", "rdx"];

        allocate_registers(&mut state, &intervals, callee_saved, caller_saved);

        // Both should get registers (we have enough)
        match state.get_value_location(ValueId(0)) {
            ValueLocation::Reg(_) => {}
            _ => panic!("expected Reg"),
        }
        match state.get_value_location(ValueId(1)) {
            ValueLocation::Reg(_) => {}
            _ => panic!("expected Reg"),
        }
    }

    #[test]
    fn test_spill_when_no_regs() {
        let mut state = CodegenState::new(Target::X86_64);
        state.begin_function("test");

        let intervals = vec![
            LiveInterval { value: ValueId(0), start: 0, end: 10, spans_call: false },
            LiveInterval { value: ValueId(1), start: 1, end: 10, spans_call: false },
            LiveInterval { value: ValueId(2), start: 2, end: 10, spans_call: false },
        ];

        // Only 2 registers available
        let callee_saved: &[&str] = &[];
        let caller_saved = &["rax", "rcx"];

        allocate_registers(&mut state, &intervals, callee_saved, caller_saved);

        // Third value should be spilled
        match state.get_value_location(ValueId(2)) {
            ValueLocation::Stack(_) => {}
            _ => panic!("expected Stack spill"),
        }
    }
}
