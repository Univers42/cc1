// backend/native/state.rs — Per-function code generation state.
//
// CodegenState holds the assembly output buffer, stack layout information,
// register allocation maps, and label counters shared between the
// architecture-independent driver and the architecture-specific codegen.

use std::collections::HashMap;
use crate::ir::types::*;
use crate::target::Target;

/// Where an IR value lives at runtime.
#[derive(Debug, Clone)]
pub enum ValueLocation {
    /// In a machine register (name, e.g. "rax", "eax", "xmm0").
    Reg(String),
    /// On the stack at [rbp - offset] (or equivalent).
    Stack(i32),
    /// A compile-time constant (never actually materialized until needed).
    Const(ConstValue),
    /// A global symbol address.
    Global(String),
    /// Not yet assigned.
    Unassigned,
}

/// Stack slot for an alloca or spill.
#[derive(Debug, Clone)]
pub struct StackSlot {
    /// Offset from frame pointer (negative on x86).
    pub offset: i32,
    /// Size in bytes.
    pub size: u32,
    /// Alignment in bytes.
    pub align: u32,
}

/// Per-function code generation state.
pub struct CodegenState {
    /// Target architecture.
    pub target: Target,

    // ── Assembly output ──────────────────────────────────────────────
    /// Final assembly text for the current translation unit.
    pub asm: String,

    // ── Stack layout ─────────────────────────────────────────────────
    /// Total frame size (bytes below the frame pointer).
    pub frame_size: u32,
    /// Stack slots for alloca'd variables and spills.
    pub stack_slots: Vec<StackSlot>,
    /// Map: ValueId → StackSlot index (for allocas).
    pub alloca_map: HashMap<u32, usize>,
    /// Next available stack offset (grows negative on x86).
    pub next_stack_offset: i32,

    // ── Value tracking ───────────────────────────────────────────────
    /// Where each IR ValueId currently resides.
    pub value_map: HashMap<u32, ValueLocation>,

    // ── Register allocation ──────────────────────────────────────────
    /// Which registers are currently in use (register name → ValueId).
    pub reg_in_use: HashMap<String, u32>,
    /// Which registers are free (available for allocation).
    pub free_regs: Vec<String>,
    /// Callee-saved registers that we've used (need to save/restore).
    pub callee_saved_used: Vec<String>,

    // ── Label counters ───────────────────────────────────────────────
    /// Next unique label counter.
    pub label_counter: u32,

    // ── Current function info ────────────────────────────────────────
    /// Current function name.
    pub func_name: String,
    /// Whether the current function uses the frame pointer.
    pub uses_frame_pointer: bool,
}

impl CodegenState {
    pub fn new(target: Target) -> Self {
        Self {
            target,
            asm: String::with_capacity(16384),
            frame_size: 0,
            stack_slots: Vec::new(),
            alloca_map: HashMap::new(),
            next_stack_offset: 0,
            value_map: HashMap::new(),
            reg_in_use: HashMap::new(),
            free_regs: Vec::new(),
            callee_saved_used: Vec::new(),
            label_counter: 0,
            func_name: String::new(),
            uses_frame_pointer: true,
        }
    }

    /// Reset per-function state for a new function.
    pub fn begin_function(&mut self, name: &str) {
        self.frame_size = 0;
        self.stack_slots.clear();
        self.alloca_map.clear();
        self.next_stack_offset = 0;
        self.value_map.clear();
        self.reg_in_use.clear();
        self.callee_saved_used.clear();
        self.func_name = name.to_string();
        self.uses_frame_pointer = true;
    }

    /// Allocate a stack slot and return its index.
    pub fn alloc_stack_slot(&mut self, size: u32, align: u32) -> usize {
        // Align the offset
        let align = align.max(1) as i32;
        self.next_stack_offset -= size as i32;
        // Align to boundary (make more negative)
        self.next_stack_offset = self.next_stack_offset & !(align - 1);

        let slot = StackSlot {
            offset: self.next_stack_offset,
            size,
            align: align as u32,
        };
        let idx = self.stack_slots.len();
        self.stack_slots.push(slot);
        idx
    }

    /// Map an alloca ValueId → stack slot.
    pub fn map_alloca(&mut self, vid: ValueId, slot_idx: usize) {
        self.alloca_map.insert(vid.0, slot_idx);
    }

    /// Get the stack offset for a value that was alloca'd.
    pub fn alloca_offset(&self, vid: ValueId) -> Option<i32> {
        self.alloca_map
            .get(&vid.0)
            .map(|&idx| self.stack_slots[idx].offset)
    }

    /// Assign a value to a location.
    pub fn set_value_location(&mut self, vid: ValueId, loc: ValueLocation) {
        self.value_map.insert(vid.0, loc);
    }

    /// Get the current location of a value.
    pub fn get_value_location(&self, vid: ValueId) -> &ValueLocation {
        self.value_map
            .get(&vid.0)
            .unwrap_or(&ValueLocation::Unassigned)
    }

    /// Emit a line of assembly.
    pub fn emit_line(&mut self, line: &str) {
        self.asm.push_str(line);
        self.asm.push('\n');
    }

    /// Emit a labeled line (no indentation).
    pub fn emit_label(&mut self, label: &str) {
        self.asm.push_str(label);
        self.asm.push_str(":\n");
    }

    /// Emit an instruction (with tab indentation).
    pub fn emit_inst(&mut self, inst: &str) {
        self.asm.push('\t');
        self.asm.push_str(inst);
        self.asm.push('\n');
    }

    /// Emit a directive (with tab indentation).
    pub fn emit_directive(&mut self, dir: &str) {
        self.asm.push('\t');
        self.asm.push_str(dir);
        self.asm.push('\n');
    }

    /// Generate a unique label.
    pub fn fresh_label(&mut self, prefix: &str) -> String {
        let n = self.label_counter;
        self.label_counter += 1;
        format!(".L{}_{}", prefix, n)
    }

    /// Finalize the frame size (align to 16 bytes on x86-64).
    pub fn finalize_frame(&mut self) {
        let raw = (-self.next_stack_offset) as u32;
        // Align to 16 bytes
        self.frame_size = (raw + 15) & !15;
    }

    /// Allocate a register from the free list. Returns None if all are in use.
    pub fn alloc_reg(&mut self, vid: ValueId) -> Option<String> {
        if let Some(reg) = self.free_regs.pop() {
            self.reg_in_use.insert(reg.clone(), vid.0);
            Some(reg)
        } else {
            None
        }
    }

    /// Free a register back to the free list.
    pub fn free_reg(&mut self, reg: &str) {
        self.reg_in_use.remove(reg);
        self.free_regs.push(reg.to_string());
    }

    /// Spill a value from a register to the stack. Returns the stack offset.
    pub fn spill(&mut self, reg: &str, size: u32) -> i32 {
        let slot_idx = self.alloc_stack_slot(size, size.min(8));
        let offset = self.stack_slots[slot_idx].offset;
        // The actual spill instruction will be emitted by the arch codegen.
        if let Some(&vid) = self.reg_in_use.get(reg) {
            self.value_map.insert(vid, ValueLocation::Stack(offset));
        }
        self.free_reg(reg);
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stack_slot_allocation() {
        let mut state = CodegenState::new(Target::X86_64);
        state.begin_function("test");

        let s0 = state.alloc_stack_slot(4, 4);
        assert_eq!(state.stack_slots[s0].offset, -4);
        assert_eq!(state.stack_slots[s0].size, 4);

        let s1 = state.alloc_stack_slot(8, 8);
        assert_eq!(state.stack_slots[s1].offset, -16);
    }

    #[test]
    fn test_alloca_mapping() {
        let mut state = CodegenState::new(Target::X86_64);
        state.begin_function("test");

        let slot = state.alloc_stack_slot(4, 4);
        state.map_alloca(ValueId(0), slot);
        assert_eq!(state.alloca_offset(ValueId(0)), Some(-4));
    }

    #[test]
    fn test_value_location() {
        let mut state = CodegenState::new(Target::X86_64);
        state.set_value_location(ValueId(0), ValueLocation::Reg("rax".into()));
        match state.get_value_location(ValueId(0)) {
            ValueLocation::Reg(r) => assert_eq!(r, "rax"),
            _ => panic!("expected Reg"),
        }
    }

    #[test]
    fn test_finalize_frame() {
        let mut state = CodegenState::new(Target::X86_64);
        state.begin_function("test");
        state.alloc_stack_slot(4, 4);  // 4 bytes
        state.alloc_stack_slot(4, 4);  // 8 bytes total
        state.finalize_frame();
        assert_eq!(state.frame_size, 16); // rounded to 16
    }

    #[test]
    fn test_fresh_label() {
        let mut state = CodegenState::new(Target::X86_64);
        let l0 = state.fresh_label("test");
        let l1 = state.fresh_label("test");
        assert_eq!(l0, ".Ltest_0");
        assert_eq!(l1, ".Ltest_1");
    }

    #[test]
    fn test_emit_helpers() {
        let mut state = CodegenState::new(Target::X86_64);
        state.emit_label("main");
        state.emit_inst("pushq %rbp");
        state.emit_inst("movq %rsp, %rbp");
        assert!(state.asm.contains("main:\n"));
        assert!(state.asm.contains("\tpushq %rbp\n"));
    }
}
