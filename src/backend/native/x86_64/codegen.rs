// backend/native/x86_64/codegen.rs — x86-64 ArchCodegen implementation.
//
// Implements the ArchCodegen trait for x86-64 (System V AMD64 ABI).
// Produces AT&T syntax assembly text.

use crate::backend::native::traits::ArchCodegen;
use crate::backend::native::state::{CodegenState, ValueLocation};
#[allow(unused_imports)]
use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::*;
use crate::ir::types::*;

/// x86-64 code generator.
pub struct X86_64Codegen;

// System V AMD64 ABI register conventions:
// Integer args: rdi, rsi, rdx, rcx, r8, r9
// Float args: xmm0-xmm7
// Return: rax (integer), xmm0 (float)
// Callee-saved: rbx, rbp, r12, r13, r14, r15
// Caller-saved: rax, rcx, rdx, rsi, rdi, r8, r9, r10, r11

const INT_ARG_REGS: &[&str] = &["rdi", "rsi", "rdx", "rcx", "r8", "r9"];
const CALLEE_SAVED: &[&str] = &["rbx", "r12", "r13", "r14", "r15"];
const CALLER_SAVED: &[&str] = &["rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11"];

impl X86_64Codegen {
    pub fn new() -> Self {
        Self
    }

    /// Get the suffix for an IrType (b, w, l, q).
    fn suffix(ty: &IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 => "b",
            IrType::I16 | IrType::U16 => "w",
            IrType::I32 | IrType::U32 | IrType::F32 => "l",
            IrType::I64 | IrType::U64 | IrType::Ptr | IrType::F64 => "q",
            _ => "q",
        }
    }

    /// Get the register name for a given size (e.g., rax → eax for 32-bit).
    fn reg_for_size(reg: &str, ty: &IrType) -> String {
        let bits = match ty {
            IrType::I8 | IrType::U8 => 8,
            IrType::I16 | IrType::U16 => 16,
            IrType::I32 | IrType::U32 | IrType::F32 => 32,
            _ => 64,
        };
        match (reg, bits) {
            ("rax", 8) => "al".into(),
            ("rax", 16) => "ax".into(),
            ("rax", 32) => "eax".into(),
            ("rax", _) => "rax".into(),
            ("rbx", 8) => "bl".into(),
            ("rbx", 16) => "bx".into(),
            ("rbx", 32) => "ebx".into(),
            ("rbx", _) => "rbx".into(),
            ("rcx", 8) => "cl".into(),
            ("rcx", 16) => "cx".into(),
            ("rcx", 32) => "ecx".into(),
            ("rcx", _) => "rcx".into(),
            ("rdx", 8) => "dl".into(),
            ("rdx", 16) => "dx".into(),
            ("rdx", 32) => "edx".into(),
            ("rdx", _) => "rdx".into(),
            ("rsi", 8) => "sil".into(),
            ("rsi", 16) => "si".into(),
            ("rsi", 32) => "esi".into(),
            ("rsi", _) => "rsi".into(),
            ("rdi", 8) => "dil".into(),
            ("rdi", 16) => "di".into(),
            ("rdi", 32) => "edi".into(),
            ("rdi", _) => "rdi".into(),
            ("r8", 8) => "r8b".into(),
            ("r8", 16) => "r8w".into(),
            ("r8", 32) => "r8d".into(),
            ("r8", _) => "r8".into(),
            ("r9", 8) => "r9b".into(),
            ("r9", 16) => "r9w".into(),
            ("r9", 32) => "r9d".into(),
            ("r9", _) => "r9".into(),
            ("r10", 8) => "r10b".into(),
            ("r10", 16) => "r10w".into(),
            ("r10", 32) => "r10d".into(),
            ("r10", _) => "r10".into(),
            ("r11", 8) => "r11b".into(),
            ("r11", 16) => "r11w".into(),
            ("r11", 32) => "r11d".into(),
            ("r11", _) => "r11".into(),
            ("r12", 32) => "r12d".into(),
            ("r12", _) => "r12".into(),
            ("r13", 32) => "r13d".into(),
            ("r13", _) => "r13".into(),
            ("r14", 32) => "r14d".into(),
            ("r14", _) => "r14".into(),
            ("r15", 32) => "r15d".into(),
            ("r15", _) => "r15".into(),
            _ => reg.into(),
        }
    }

    /// Format an operand for AT&T syntax.
    fn format_operand(&self, state: &CodegenState, op: &Operand, ty: &IrType) -> String {
        match op {
            Operand::Value(vid) => {
                match state.get_value_location(*vid) {
                    ValueLocation::Reg(r) => {
                        format!("%{}", Self::reg_for_size(r, ty))
                    }
                    ValueLocation::Stack(off) => {
                        format!("{}(%rbp)", off)
                    }
                    ValueLocation::Const(c) => {
                        format!("${}", const_to_i64(c))
                    }
                    ValueLocation::Global(name) => {
                        format!("{}(%rip)", name)
                    }
                    ValueLocation::Unassigned => {
                        format!("/* unassigned %{} */", vid.0)
                    }
                }
            }
            Operand::Const(c) => {
                format!("${}", const_to_i64(c))
            }
            Operand::Global(name) => {
                format!("{}(%rip)", name)
            }
            Operand::Label(bid) => {
                format!(".LBB_{}", bid.0)
            }
        }
    }
}

impl ArchCodegen for X86_64Codegen {
    fn emit_file_header(&self, state: &mut CodegenState, module: &IrModule) {
        state.emit_directive(&format!(".file\t\"{}\"", module.source_file));
        state.emit_directive(".text");
    }

    fn emit_file_footer(&self, state: &mut CodegenState, _module: &IrModule) {
        state.emit_directive(".section\t.note.GNU-stack,\"\",@progbits");
    }

    fn emit_global(&self, state: &mut CodegenState, name: &str, ty: &IrType,
                   init: &GlobalInit, align: u32, linkage: Linkage) {
        state.emit_directive(".data");
        if linkage != Linkage::Internal && linkage != Linkage::Private {
            state.emit_directive(&format!(".globl\t{}", name));
        }
        if align > 0 {
            state.emit_directive(&format!(".align\t{}", align));
        }
        state.emit_directive(&format!(".type\t{}, @object", name));
        state.emit_label(name);

        self.emit_init_data(state, init, ty);
        state.emit_line("");
    }

    fn emit_string_literal(&self, state: &mut CodegenState, label: &str, bytes: &[u8]) {
        state.emit_directive(".section\t.rodata");
        state.emit_label(label);
        // Emit as .byte directives
        let bytes_str: Vec<String> = bytes.iter().map(|b| format!("{}", b)).collect();
        for chunk in bytes_str.chunks(16) {
            state.emit_directive(&format!(".byte\t{}", chunk.join(", ")));
        }
    }

    fn emit_bss(&self, state: &mut CodegenState, name: &str, size: u64, align: u32, linkage: Linkage) {
        if linkage == Linkage::Common {
            state.emit_directive(&format!(".comm\t{},{},{}", name, size, align));
        } else {
            state.emit_directive(".bss");
            if linkage != Linkage::Internal && linkage != Linkage::Private {
                state.emit_directive(&format!(".globl\t{}", name));
            }
            if align > 0 {
                state.emit_directive(&format!(".align\t{}", align));
            }
            state.emit_directive(&format!(".type\t{}, @object", name));
            state.emit_label(name);
            state.emit_directive(&format!(".zero\t{}", size));
        }
    }

    fn emit_function_prologue(&self, state: &mut CodegenState, func: &IrFunction) {
        state.emit_directive(".text");
        if func.linkage != Linkage::Internal && func.linkage != Linkage::Private {
            state.emit_directive(&format!(".globl\t{}", func.name));
        }
        state.emit_directive(&format!(".type\t{}, @function", func.name));
        state.emit_label(&func.name);

        // Standard frame setup
        state.emit_inst("pushq\t%rbp");
        state.emit_inst("movq\t%rsp, %rbp");

        // Save callee-saved registers that we use
        for reg in &state.callee_saved_used.clone() {
            state.emit_inst(&format!("pushq\t%{}", reg));
        }

        // Allocate stack frame
        if state.frame_size > 0 {
            state.emit_inst(&format!("subq\t${}, %rsp", state.frame_size));
        }

        // Store parameters from registers to stack slots
        // (handled by IR store instructions — no explicit prologue stores needed)
    }

    fn emit_function_epilogue(&self, state: &mut CodegenState, func: &IrFunction) {
        // Epilogue label
        state.emit_label(&format!(".Lret_{}", func.name));

        // Restore callee-saved registers (reverse order)
        for reg in state.callee_saved_used.clone().iter().rev() {
            state.emit_inst(&format!("popq\t%{}", reg));
        }

        state.emit_inst("leave");
        state.emit_inst("ret");

        state.emit_directive(&format!(
            ".size\t{}, .-{}",
            func.name, func.name
        ));
        state.emit_line("");
    }

    fn compute_stack_layout(&self, state: &mut CodegenState, func: &IrFunction) {
        // Initialize free registers
        state.free_regs = CALLER_SAVED
            .iter()
            .chain(CALLEE_SAVED.iter())
            .map(|s| s.to_string())
            .collect();

        // Map function parameters to their ABI register locations.
        // Parameters are allocated stack slots and the prologue copies them there.
        for (i, param) in func.params.iter().enumerate() {
            if i < INT_ARG_REGS.len() {
                // Parameter's value (%0, %1, ...) lives in the ABI register initially
                state.set_value_location(
                    param.value,
                    ValueLocation::Reg(INT_ARG_REGS[i].to_string()),
                );
                // Remove the param register from the free list so regalloc doesn't reuse it
                state.free_regs.retain(|r| r != INT_ARG_REGS[i]);
            }
        }

        // First pass: allocate stack slots for alloca instructions
        for bb in &func.blocks {
            for inst in &bb.insts {
                if let Instruction::Alloca { result, ty, align } = inst {
                    let size = ty_size(ty) as u32;
                    let slot = state.alloc_stack_slot(size, *align);
                    state.map_alloca(*result, slot);
                    state.set_value_location(
                        *result,
                        ValueLocation::Stack(state.stack_slots[slot].offset),
                    );
                }
            }
        }

        // Run register allocation (skipping values already assigned)
        let mut intervals = crate::backend::native::regalloc::compute_liveness(func);
        // Filter out alloca results and parameter values — they already have locations
        intervals.retain(|iv| {
            matches!(state.get_value_location(iv.value), ValueLocation::Unassigned)
        });
        crate::backend::native::regalloc::allocate_registers(
            state,
            &intervals,
            CALLEE_SAVED,
            CALLER_SAVED,
        );

        // After regalloc, re-add parameter registers to free list
        // (they are only needed briefly in the prologue for spilling)
        for (i, _param) in func.params.iter().enumerate() {
            if i < INT_ARG_REGS.len() {
                if !state.free_regs.contains(&INT_ARG_REGS[i].to_string()) {
                    state.free_regs.push(INT_ARG_REGS[i].to_string());
                }
            }
        }

        // Finalize frame size
        state.finalize_frame();
    }

    fn emit_block_label(&self, state: &mut CodegenState, func: &IrFunction, block: &BasicBlock) {
        // Don't emit label for entry block (it follows the prologue)
        if block.id != BlockId(0) {
            state.emit_label(&self.block_label(func, block.id));
        }
    }

    fn emit_alloca(&self, state: &mut CodegenState, result: ValueId, _ty: &IrType, _align: u32) {
        // Allocas are laid out during compute_stack_layout.
        // The result is already associated with a stack slot.
        // Nothing to emit here.
        let _ = state.alloca_offset(result);
    }

    fn emit_store(&self, state: &mut CodegenState, addr: &Operand, value: &Operand, ty: &IrType) {
        let sf = Self::suffix(ty);
        let val_str = self.format_operand(state, value, ty);
        let addr_str = self.format_addr(state, addr);

        // Need to go through a register for memory-to-memory moves
        if is_memory_operand(&val_str) && is_memory_operand(&addr_str) {
            let tmp = Self::reg_for_size("rax", ty);
            state.emit_inst(&format!("mov{}\t{}, %{}", sf, val_str, tmp));
            state.emit_inst(&format!("mov{}\t%{}, {}", sf, tmp, addr_str));
        } else {
            state.emit_inst(&format!("mov{}\t{}, {}", sf, val_str, addr_str));
        }
    }

    fn emit_load(&self, state: &mut CodegenState, result: ValueId, addr: &Operand, ty: &IrType) {
        let sf = Self::suffix(ty);
        let addr_str = self.format_addr(state, addr);
        let dst = self.get_or_alloc_reg(state, result, ty);
        state.emit_inst(&format!("mov{}\t{}, {}", sf, addr_str, dst));
    }

    fn emit_binop(&self, state: &mut CodegenState, result: ValueId, op: BinOpKind,
                  lhs: &Operand, rhs: &Operand, ty: &IrType) {
        let sf = Self::suffix(ty);
        let lhs_str = self.format_operand(state, lhs, ty);
        let rhs_str = self.format_operand(state, rhs, ty);
        let dst = self.get_or_alloc_reg(state, result, ty);

        match op {
            BinOpKind::Add | BinOpKind::FAdd => {
                // add is commutative: if dst == rhs, just add lhs
                if dst == rhs_str {
                    state.emit_inst(&format!("add{}\t{}, {}", sf, lhs_str, dst));
                } else {
                    if dst != lhs_str {
                        state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                    }
                    state.emit_inst(&format!("add{}\t{}, {}", sf, rhs_str, dst));
                }
            }
            BinOpKind::Sub | BinOpKind::FSub => {
                if dst == rhs_str && dst != lhs_str {
                    // dst contains rhs; need temp: tmp = lhs; tmp -= rhs; mov tmp, dst
                    let tmp = format!("%{}", Self::reg_for_size("rax", ty));
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, tmp));
                    state.emit_inst(&format!("sub{}\t{}, {}", sf, dst, tmp));
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, tmp, dst));
                } else {
                    if dst != lhs_str {
                        state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                    }
                    state.emit_inst(&format!("sub{}\t{}, {}", sf, rhs_str, dst));
                }
            }
            BinOpKind::Mul | BinOpKind::FMul => {
                // imul is commutative
                if dst == rhs_str {
                    state.emit_inst(&format!("imul{}\t{}, {}", sf, lhs_str, dst));
                } else {
                    if dst != lhs_str {
                        state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                    }
                    state.emit_inst(&format!("imul{}\t{}, {}", sf, rhs_str, dst));
                }
            }
            BinOpKind::SDiv | BinOpKind::UDiv | BinOpKind::FDiv => {
                // idiv/div uses rax:rdx pair
                let rax = format!("%{}", Self::reg_for_size("rax", ty));
                state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, rax));
                if op == BinOpKind::SDiv {
                    match ty {
                        IrType::I32 | IrType::U32 => state.emit_inst("cltd"),
                        IrType::I64 | IrType::U64 => state.emit_inst("cqto"),
                        _ => state.emit_inst("cltd"),
                    }
                } else {
                    state.emit_inst(&format!("xor{}\t%{}, %{}", sf,
                        Self::reg_for_size("rdx", ty),
                        Self::reg_for_size("rdx", ty)));
                }
                // Need rhs in a register for div
                let divisor = self.operand_to_reg(state, rhs, ty, "rcx");
                state.emit_inst(&format!("idiv{}\t{}", sf, divisor));
                if dst != rax {
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, rax, dst));
                }
            }
            BinOpKind::SRem | BinOpKind::URem | BinOpKind::FRem => {
                let rax = format!("%{}", Self::reg_for_size("rax", ty));
                let rdx = format!("%{}", Self::reg_for_size("rdx", ty));
                state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, rax));
                if op == BinOpKind::SRem {
                    match ty {
                        IrType::I32 | IrType::U32 => state.emit_inst("cltd"),
                        _ => state.emit_inst("cqto"),
                    }
                } else {
                    state.emit_inst(&format!("xor{}\t{}, {}", sf, rdx, rdx));
                }
                let divisor = self.operand_to_reg(state, rhs, ty, "rcx");
                state.emit_inst(&format!("idiv{}\t{}", sf, divisor));
                // Remainder is in rdx
                if dst != rdx {
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, rdx, dst));
                }
            }
            BinOpKind::And => {
                if dst == rhs_str {
                    state.emit_inst(&format!("and{}\t{}, {}", sf, lhs_str, dst));
                } else {
                    if dst != lhs_str {
                        state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                    }
                    state.emit_inst(&format!("and{}\t{}, {}", sf, rhs_str, dst));
                }
            }
            BinOpKind::Or => {
                if dst == rhs_str {
                    state.emit_inst(&format!("or{}\t{}, {}", sf, lhs_str, dst));
                } else {
                    if dst != lhs_str {
                        state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                    }
                    state.emit_inst(&format!("or{}\t{}, {}", sf, rhs_str, dst));
                }
            }
            BinOpKind::Xor => {
                if dst == rhs_str {
                    state.emit_inst(&format!("xor{}\t{}, {}", sf, lhs_str, dst));
                } else {
                    if dst != lhs_str {
                        state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                    }
                    state.emit_inst(&format!("xor{}\t{}, {}", sf, rhs_str, dst));
                }
            }
            BinOpKind::Shl => {
                if dst != lhs_str {
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                }
                // Shift amount must be in %cl or immediate
                let shift = self.shift_operand(state, rhs, ty);
                state.emit_inst(&format!("shl{}\t{}, {}", sf, shift, dst));
            }
            BinOpKind::LShr => {
                if dst != lhs_str {
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                }
                let shift = self.shift_operand(state, rhs, ty);
                state.emit_inst(&format!("shr{}\t{}, {}", sf, shift, dst));
            }
            BinOpKind::AShr => {
                if dst != lhs_str {
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, lhs_str, dst));
                }
                let shift = self.shift_operand(state, rhs, ty);
                state.emit_inst(&format!("sar{}\t{}, {}", sf, shift, dst));
            }
        }
    }

    fn emit_unaryop(&self, state: &mut CodegenState, result: ValueId, op: UnaryOpKind,
                    operand: &Operand, ty: &IrType) {
        let sf = Self::suffix(ty);
        let src = self.format_operand(state, operand, ty);
        let dst = self.get_or_alloc_reg(state, result, ty);

        match op {
            UnaryOpKind::Neg | UnaryOpKind::FNeg => {
                if dst != src {
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, src, dst));
                }
                state.emit_inst(&format!("neg{}\t{}", sf, dst));
            }
            UnaryOpKind::BitNot => {
                if dst != src {
                    state.emit_inst(&format!("mov{}\t{}, {}", sf, src, dst));
                }
                state.emit_inst(&format!("not{}\t{}", sf, dst));
            }
            UnaryOpKind::LogNot => {
                // Logical NOT: result = (src == 0) ? 1 : 0
                let src_reg = self.operand_to_reg(state, operand, ty, "rax");
                state.emit_inst(&format!("test{}\t{}, {}", sf, src_reg, src_reg));
                state.emit_inst(&format!("sete\t{}", dst));
            }
        }
    }

    fn emit_icmp(&self, state: &mut CodegenState, result: ValueId, pred: IcmpPred,
                 lhs: &Operand, rhs: &Operand, ty: &IrType) {
        let sf = Self::suffix(ty);
        let _lhs_str = self.format_operand(state, lhs, ty);
        let rhs_str = self.format_operand(state, rhs, ty);

        // cmp rhs, lhs (AT&T order: compare is reversed)
        // Need both operands in register-compatible forms
        let lhs_reg = self.operand_to_reg(state, lhs, ty, "rax");
        state.emit_inst(&format!("cmp{}\t{}, {}", sf, rhs_str, lhs_reg));

        let dst_reg = self.get_or_alloc_reg(state, result, &IrType::I8);
        let set_cc = match pred {
            IcmpPred::Eq => "sete",
            IcmpPred::Ne => "setne",
            IcmpPred::Slt => "setl",
            IcmpPred::Sgt => "setg",
            IcmpPred::Sle => "setle",
            IcmpPred::Sge => "setge",
            IcmpPred::Ult => "setb",
            IcmpPred::Ugt => "seta",
            IcmpPred::Ule => "setbe",
            IcmpPred::Uge => "setae",
        };
        state.emit_inst(&format!("{}\t{}", set_cc, dst_reg));
    }

    fn emit_fcmp(&self, state: &mut CodegenState, result: ValueId, pred: FcmpPred,
                 lhs: &Operand, rhs: &Operand, ty: &IrType) {
        // SSE comparison
        let cmp_suffix = if *ty == IrType::F32 { "ss" } else { "sd" };
        let lhs_str = self.format_operand(state, lhs, ty);
        let rhs_str = self.format_operand(state, rhs, ty);

        state.emit_inst(&format!("ucomi{}\t{}, {}", cmp_suffix, rhs_str, lhs_str));

        let dst_reg = self.get_or_alloc_reg(state, result, &IrType::I8);
        let set_cc = match pred {
            FcmpPred::Oeq | FcmpPred::Ueq => "sete",
            FcmpPred::One | FcmpPred::Une => "setne",
            FcmpPred::Olt | FcmpPred::Ult => "setb",
            FcmpPred::Ogt | FcmpPred::Ugt => "seta",
            FcmpPred::Ole | FcmpPred::Ule => "setbe",
            FcmpPred::Oge | FcmpPred::Uge => "setae",
            FcmpPred::Ord => "setnp",
            FcmpPred::Uno => "setp",
        };
        state.emit_inst(&format!("{}\t{}", set_cc, dst_reg));
    }

    fn emit_cast(&self, state: &mut CodegenState, result: ValueId, kind: CastKind,
                 src: &Operand, src_ty: &IrType, dst_ty: &IrType) {
        let src_str = self.format_operand(state, src, src_ty);
        let dst = self.get_or_alloc_reg(state, result, dst_ty);

        match kind {
            CastKind::ZExt => {
                state.emit_inst(&format!("movz{}{}\t{}, {}",
                    Self::suffix(src_ty), Self::suffix(dst_ty), src_str, dst));
            }
            CastKind::SExt => {
                state.emit_inst(&format!("movs{}{}\t{}, {}",
                    Self::suffix(src_ty), Self::suffix(dst_ty), src_str, dst));
            }
            CastKind::Trunc => {
                // Just move — the upper bits are ignored
                let dst_small = self.get_or_alloc_reg(state, result, dst_ty);
                state.emit_inst(&format!("mov{}\t{}, {}", Self::suffix(dst_ty), src_str, dst_small));
            }
            CastKind::SIToFP => {
                let dst_ss = if *dst_ty == IrType::F32 { "ss" } else { "sd" };
                state.emit_inst(&format!("cvtsi2{}{}\t{}, {}",
                    dst_ss, Self::suffix(src_ty), src_str, dst));
            }
            CastKind::UIToFP => {
                // Unsigned int to float — use indirect conversion for large values
                state.emit_inst(&format!("# utof: cvtsi2sd{}\t{}, {}", Self::suffix(src_ty), src_str, dst));
            }
            CastKind::FPToSI => {
                let src_ss = if *src_ty == IrType::F32 { "ss" } else { "sd" };
                state.emit_inst(&format!("cvtt{}2si{}\t{}, {}",
                    src_ss, Self::suffix(dst_ty), src_str, dst));
            }
            CastKind::FPToUI => {
                state.emit_inst(&format!("# ftou: cvttsd2si{}\t{}, {}", Self::suffix(dst_ty), src_str, dst));
            }
            CastKind::FPExt => {
                state.emit_inst(&format!("cvtss2sd\t{}, {}", src_str, dst));
            }
            CastKind::FPTrunc => {
                state.emit_inst(&format!("cvtsd2ss\t{}, {}", src_str, dst));
            }
            CastKind::PtrToInt | CastKind::IntToPtr | CastKind::Bitcast => {
                if src_str != dst {
                    state.emit_inst(&format!("movq\t{}, {}", src_str, dst));
                }
            }
            // No AddrSpaceCast — last variant is Bitcast above
        }
    }

    fn emit_call(&self, state: &mut CodegenState, result: ValueId, callee: &str,
                 args: &[(Operand, IrType)], ret_ty: &IrType, _is_variadic: bool) {
        // Pass arguments in registers
        for (i, (op, ty)) in args.iter().enumerate() {
            if i < INT_ARG_REGS.len() {
                let sf = Self::suffix(ty);
                let reg = Self::reg_for_size(INT_ARG_REGS[i], ty);
                let val = self.format_operand(state, op, ty);
                state.emit_inst(&format!("mov{}\t{}, %{}", sf, val, reg));
            } else {
                // Push on stack (in reverse order for System V ABI)
                let val = self.format_operand(state, op, ty);
                state.emit_inst(&format!("pushq\t{}", val));
            }
        }

        // Ensure 16-byte stack alignment before call
        // (Simplified — real implementation would track exact alignment)
        state.emit_inst(&format!("call\t{}", callee));

        // Clean up stack args
        let stack_args = if args.len() > INT_ARG_REGS.len() {
            args.len() - INT_ARG_REGS.len()
        } else {
            0
        };
        if stack_args > 0 {
            state.emit_inst(&format!("addq\t${}, %rsp", stack_args * 8));
        }

        // Move return value to result location
        if *ret_ty != IrType::Void {
            let dst = self.get_or_alloc_reg(state, result, ret_ty);
            let sf = Self::suffix(ret_ty);
            let rax = Self::reg_for_size("rax", ret_ty);
            if dst != format!("%{}", rax) {
                state.emit_inst(&format!("mov{}\t%{}, {}", sf, rax, dst));
            }
        }
    }

    fn emit_call_indirect(&self, state: &mut CodegenState, result: ValueId,
                          func_ptr: &Operand, args: &[(Operand, IrType)],
                          ret_ty: &IrType, _is_variadic: bool) {
        // Same as direct call but use *%reg for the callee
        for (i, (op, ty)) in args.iter().enumerate() {
            if i < INT_ARG_REGS.len() {
                let sf = Self::suffix(ty);
                let reg = Self::reg_for_size(INT_ARG_REGS[i], ty);
                let val = self.format_operand(state, op, ty);
                state.emit_inst(&format!("mov{}\t{}, %{}", sf, val, reg));
            }
        }

        let ptr = self.format_operand(state, func_ptr, &IrType::Ptr);
        // Move pointer to r11 (caller-saved, not used for args)
        state.emit_inst(&format!("movq\t{}, %r11", ptr));
        state.emit_inst("call\t*%r11");

        if *ret_ty != IrType::Void {
            let dst = self.get_or_alloc_reg(state, result, ret_ty);
            let sf = Self::suffix(ret_ty);
            let rax = Self::reg_for_size("rax", ret_ty);
            if dst != format!("%{}", rax) {
                state.emit_inst(&format!("mov{}\t%{}, {}", sf, rax, dst));
            }
        }
    }

    fn emit_gep(&self, state: &mut CodegenState, result: ValueId,
                base: &Operand, offset: &Operand, elem_ty: &IrType) {
        let base_str = self.format_operand(state, base, &IrType::Ptr);
        let dst = self.get_or_alloc_reg(state, result, &IrType::Ptr);

        // Load base address
        state.emit_inst(&format!("leaq\t{}, {}", base_str, dst));

        // Add offset * elem_size
        let elem_size = ty_size(elem_ty);
        if elem_size > 0 {
            let off_str = self.format_operand(state, offset, &IrType::I64);
            if elem_size == 1 {
                state.emit_inst(&format!("addq\t{}, {}", off_str, dst));
            } else {
                // offset * elem_size
                let tmp = "%rax";
                state.emit_inst(&format!("movq\t{}, {}", off_str, tmp));
                state.emit_inst(&format!("imulq\t${}, {}", elem_size, tmp));
                state.emit_inst(&format!("addq\t{}, {}", tmp, dst));
            }
        }
    }

    fn emit_global_addr(&self, state: &mut CodegenState, result: ValueId, name: &str) {
        let dst = self.get_or_alloc_reg(state, result, &IrType::Ptr);
        state.emit_inst(&format!("leaq\t{}(%rip), {}", name, dst));
    }

    fn emit_select(&self, state: &mut CodegenState, result: ValueId,
                   cond: &Operand, true_val: &Operand, false_val: &Operand, ty: &IrType) {
        let sf = Self::suffix(ty);
        let cond_str = self.format_operand(state, cond, &IrType::I8);
        let _true_str = self.format_operand(state, true_val, ty);
        let false_str = self.format_operand(state, false_val, ty);
        let dst = self.get_or_alloc_reg(state, result, ty);

        // Test condition
        state.emit_inst(&format!("testb\t{}, {}", cond_str, cond_str));
        // Move false value first, then conditionally overwrite with true
        state.emit_inst(&format!("mov{}\t{}, {}", sf, false_str, dst));
        let true_reg = self.operand_to_reg(state, true_val, ty, "rcx");
        state.emit_inst(&format!("cmovne{}\t{}, {}", sf, true_reg, dst));
    }

    fn emit_phi(&self, state: &mut CodegenState, result: ValueId, ty: &IrType,
                _incoming: &[(BlockId, Operand)]) {
        // Phi nodes are lowered to parallel copies during phi elimination.
        // For the naive approach, the register allocator should handle phi resolution.
        // Here we just ensure the result has a location.
        let _ = self.get_or_alloc_reg(state, result, ty);
    }

    fn emit_ret(&self, state: &mut CodegenState, value: &Option<Operand>) {
        if let Some(val) = value {
            let ty = &IrType::I64; // assume i64 for now
            let val_str = self.format_operand(state, val, ty);
            let rax = Self::reg_for_size("rax", ty);
            if val_str != format!("%{}", rax) {
                state.emit_inst(&format!("mov{}\t{}, %{}", Self::suffix(ty), val_str, rax));
            }
        }
        state.emit_inst(&format!("jmp\t.Lret_{}", state.func_name));
    }

    fn emit_br(&self, state: &mut CodegenState, target: BlockId) {
        state.emit_inst(&format!("jmp\t.LBB_{}", target.0));
    }

    fn emit_cond_br(&self, state: &mut CodegenState, cond: &Operand,
                    true_bb: BlockId, false_bb: BlockId) {
        let cond_str = self.format_operand(state, cond, &IrType::I8);
        state.emit_inst(&format!("testb\t{}, {}", cond_str, cond_str));
        state.emit_inst(&format!("jne\t.LBB_{}", true_bb.0));
        state.emit_inst(&format!("jmp\t.LBB_{}", false_bb.0));
    }

    fn emit_switch(&self, state: &mut CodegenState, discr: &Operand, ty: &IrType,
                   default: BlockId, cases: &[(i64, BlockId)]) {
        let discr_str = self.format_operand(state, discr, ty);
        let sf = Self::suffix(ty);

        for (val, bb) in cases {
            state.emit_inst(&format!("cmp{}\t${}, {}", sf, val, discr_str));
            state.emit_inst(&format!("je\t.LBB_{}", bb.0));
        }
        state.emit_inst(&format!("jmp\t.LBB_{}", default.0));
    }

    fn value_location(&self, state: &CodegenState, vid: ValueId) -> String {
        match state.get_value_location(vid) {
            ValueLocation::Reg(r) => format!("%{}", r),
            ValueLocation::Stack(off) => format!("{}(%rbp)", off),
            ValueLocation::Const(c) => format!("${}", const_to_i64(c)),
            ValueLocation::Global(name) => format!("{}(%rip)", name),
            ValueLocation::Unassigned => format!("/* unassigned %{} */", vid.0),
        }
    }

    fn materialize_operand(&self, state: &mut CodegenState, op: &Operand, ty: &IrType) -> String {
        self.format_operand(state, op, ty)
    }
}

// ── X86_64 helper methods ─────────────────────────────────────────────

impl X86_64Codegen {
    fn emit_init_data(&self, state: &mut CodegenState, init: &GlobalInit, ty: &IrType) {
        match init {
            GlobalInit::Integer(v) => {
                let dir = match ty {
                    IrType::I8 | IrType::U8 => format!(".byte\t{}", *v as u8),
                    IrType::I16 | IrType::U16 => format!(".short\t{}", *v as u16),
                    IrType::I32 | IrType::U32 => format!(".long\t{}", *v as u32),
                    _ => format!(".quad\t{}", v),
                };
                state.emit_directive(&dir);
            }
            GlobalInit::Float(v) => {
                match ty {
                    IrType::F32 => {
                        let bits = (*v as f32).to_bits();
                        state.emit_directive(&format!(".long\t{}\t# float {}", bits, v));
                    }
                    _ => {
                        let bits = v.to_bits();
                        state.emit_directive(&format!(".quad\t{}\t# double {}", bits, v));
                    }
                }
            }
            GlobalInit::String(bytes) => {
                let escaped: Vec<String> = bytes.iter().map(|b| format!("{}", b)).collect();
                for chunk in escaped.chunks(16) {
                    state.emit_directive(&format!(".byte\t{}", chunk.join(", ")));
                }
            }
            GlobalInit::ZeroFill(n) => {
                if *n > 0 {
                    state.emit_directive(&format!(".zero\t{}", n));
                }
            }
            GlobalInit::Address { symbol, offset } => {
                if *offset != 0 {
                    state.emit_directive(&format!(".quad\t{}+{}", symbol, offset));
                } else {
                    state.emit_directive(&format!(".quad\t{}", symbol));
                }
            }
            GlobalInit::Compound(parts) => {
                for (_, sub) in parts {
                    self.emit_init_data(state, sub, ty);
                }
            }
            GlobalInit::LabelDiff { pos, neg } => {
                state.emit_directive(&format!(".long\t{}-{}", pos, neg));
            }
        }
    }

    /// Get or allocate a register for a result value.
    fn get_or_alloc_reg(&self, state: &mut CodegenState, vid: ValueId, ty: &IrType) -> String {
        match state.get_value_location(vid) {
            ValueLocation::Reg(r) => {
                format!("%{}", Self::reg_for_size(r, ty))
            }
            ValueLocation::Stack(off) => {
                format!("{}(%rbp)", off)
            }
            _ => {
                // Try to allocate a register
                if let Some(reg) = state.alloc_reg(vid) {
                    let sized = Self::reg_for_size(&reg, ty);
                    state.set_value_location(vid, ValueLocation::Reg(reg));
                    format!("%{}", sized)
                } else {
                    // Spill to stack
                    let slot = state.alloc_stack_slot(8, 8);
                    let off = state.stack_slots[slot].offset;
                    state.set_value_location(vid, ValueLocation::Stack(off));
                    format!("{}(%rbp)", off)
                }
            }
        }
    }

    /// Format an address operand (for store/load).
    fn format_addr(&self, state: &CodegenState, op: &Operand) -> String {
        match op {
            Operand::Value(vid) => {
                match state.get_value_location(*vid) {
                    ValueLocation::Reg(r) => format!("(%{})", r),
                    ValueLocation::Stack(off) => format!("{}(%rbp)", off),
                    ValueLocation::Global(name) => format!("{}(%rip)", name),
                    _ => format!("/* addr? #{} */", vid.0),
                }
            }
            Operand::Global(name) => format!("{}(%rip)", name),
            _ => format!("/* addr? */"),
        }
    }

    /// Move an operand into a specific register if it isn't already there.
    fn operand_to_reg(&self, state: &mut CodegenState, op: &Operand, ty: &IrType, hint: &str) -> String {
        let op_str = self.format_operand(state, op, ty);
        let reg = format!("%{}", Self::reg_for_size(hint, ty));
        if op_str != reg {
            state.emit_inst(&format!("mov{}\t{}, {}", Self::suffix(ty), op_str, reg));
        }
        reg
    }

    /// Format a shift operand (must be %cl or immediate).
    fn shift_operand(&self, state: &mut CodegenState, op: &Operand, _ty: &IrType) -> String {
        match op {
            Operand::Const(c) => format!("${}", const_to_i64(c)),
            _ => {
                // Move to rcx, use cl
                let val = self.format_operand(state, op, &IrType::I8);
                state.emit_inst(&format!("movb\t{}, %cl", val));
                "%cl".into()
            }
        }
    }
}

// ── Utility functions ─────────────────────────────────────────────────

fn const_to_i64(c: &ConstValue) -> i64 {
    match c {
        ConstValue::I8(v) => *v as i64,
        ConstValue::I16(v) => *v as i64,
        ConstValue::I32(v) => *v as i64,
        ConstValue::I64(v) => *v,
        ConstValue::U8(v) => *v as i64,
        ConstValue::U16(v) => *v as i64,
        ConstValue::U32(v) => *v as i64,
        ConstValue::U64(v) => *v as i64,
        ConstValue::F32(v) => *v as i64,
        ConstValue::F64(v) => *v as i64,
        ConstValue::NullPtr => 0,
        _ => 0,
    }
}

fn ty_size(ty: &IrType) -> u64 {
    match ty {
        IrType::I8 | IrType::U8 => 1,
        IrType::I16 | IrType::U16 => 2,
        IrType::I32 | IrType::U32 | IrType::F32 => 4,
        IrType::I64 | IrType::U64 | IrType::F64 | IrType::Ptr => 8,
        IrType::I128 | IrType::U128 | IrType::F128 => 16,
        IrType::Void => 0,
        IrType::Array(elem, count) => ty_size(elem) * count,
        IrType::Struct(fields) => {
            fields.iter().map(|f| ty_size(f)).sum()
        }
    }
}

fn is_memory_operand(s: &str) -> bool {
    s.contains("(%") || s.contains("(%rip)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::module::*;
    use crate::target::Target;
    use crate::backend::native::generation::generate_asm;

    #[test]
    fn test_x86_64_codegen_empty_function() {
        let mut module = IrModule::new("test.c");
        let mut func = IrFunction::new("empty", IrType::Void, Linkage::External);
        func.create_block("entry");
        func.block_mut(BlockId(0)).set_terminator(Terminator::Ret { value: None });
        module.add_function(func);

        let codegen = X86_64Codegen::new();
        let asm = generate_asm(&codegen, &module, Target::X86_64);

        assert!(asm.contains(".globl\tempty"));
        assert!(asm.contains("empty:"));
        assert!(asm.contains("pushq\t%rbp"));
        assert!(asm.contains("movq\t%rsp, %rbp"));
        assert!(asm.contains("leave"));
        assert!(asm.contains("ret"));
    }

    #[test]
    fn test_x86_64_codegen_return_constant() {
        let mut module = IrModule::new("test.c");
        let mut func = IrFunction::new("answer", IrType::I32, Linkage::External);
        let entry = func.create_block("entry");
        func.block_mut(entry).set_terminator(Terminator::Ret {
            value: Some(Operand::Const(ConstValue::I32(42))),
        });
        module.add_function(func);

        let codegen = X86_64Codegen::new();
        let asm = generate_asm(&codegen, &module, Target::X86_64);

        assert!(asm.contains("answer:"));
        assert!(asm.contains("$42"));
    }

    #[test]
    fn test_x86_64_string_literal() {
        let mut module = IrModule::new("test.c");
        module.intern_string(b"Hello\0".to_vec());

        let codegen = X86_64Codegen::new();
        let asm = generate_asm(&codegen, &module, Target::X86_64);

        assert!(asm.contains(".rodata"));
        assert!(asm.contains(".LC0:"));
    }

    #[test]
    fn test_x86_64_global_variable() {
        let mut module = IrModule::new("test.c");
        module.add_global(GlobalVariable {
            name: "counter".into(),
            ty: IrType::I32,
            init: Some(GlobalInit::Integer(42)),
            linkage: Linkage::External,
            visibility: Visibility::Default,
            section: None,
            align: 4,
            is_const: false,
            is_tls: false,
        });

        let codegen = X86_64Codegen::new();
        let asm = generate_asm(&codegen, &module, Target::X86_64);

        assert!(asm.contains(".globl\tcounter"));
        assert!(asm.contains("counter:"));
        assert!(asm.contains(".long\t42"));
    }

    #[test]
    fn test_reg_for_size() {
        assert_eq!(X86_64Codegen::reg_for_size("rax", &IrType::I8), "al");
        assert_eq!(X86_64Codegen::reg_for_size("rax", &IrType::I16), "ax");
        assert_eq!(X86_64Codegen::reg_for_size("rax", &IrType::I32), "eax");
        assert_eq!(X86_64Codegen::reg_for_size("rax", &IrType::I64), "rax");
        assert_eq!(X86_64Codegen::reg_for_size("r8", &IrType::I32), "r8d");
        assert_eq!(X86_64Codegen::reg_for_size("r8", &IrType::I8), "r8b");
    }
}
