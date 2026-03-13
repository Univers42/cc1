// backend/native/generation.rs — Architecture-independent generation driver.
//
// Walks the IR module and dispatches to the ArchCodegen trait methods.
// This is the main entry point for native code generation.

use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::*;
use crate::ir::types::*;
use crate::backend::native::traits::ArchCodegen;
use crate::backend::native::state::CodegenState;
use crate::target::Target;

/// Generate assembly text from an IR module using the given architecture backend.
pub fn generate_asm(
    arch: &dyn ArchCodegen,
    module: &IrModule,
    target: Target,
) -> String {
    let mut state = CodegenState::new(target);

    // File header
    arch.emit_file_header(&mut state, module);

    // String literals
    for (label, bytes) in &module.string_literals {
        arch.emit_string_literal(&mut state, label, bytes);
    }

    // Global variables
    for gv in &module.globals {
        if let Some(init) = &gv.init {
            match init {
                GlobalInit::ZeroFill(n) if *n > 0 => {
                    arch.emit_bss(&mut state, &gv.name, *n as u64, gv.align, gv.linkage);
                }
                _ => {
                    arch.emit_global(&mut state, &gv.name, &gv.ty, init, gv.align, gv.linkage);
                }
            }
        } else {
            // Extern declaration (no init) — skip, just referenced
        }
    }

    // Functions
    for func in &module.functions {
        generate_function(arch, &mut state, func);
    }

    // File footer
    arch.emit_file_footer(&mut state, module);

    state.asm
}

/// Generate assembly for a single function.
fn generate_function(
    arch: &dyn ArchCodegen,
    state: &mut CodegenState,
    func: &IrFunction,
) {
    state.begin_function(&func.name);

    // Compute stack layout (allocas, spills)
    arch.compute_stack_layout(state, func);

    // Emit function prologue
    arch.emit_function_prologue(state, func);

    // Emit each basic block
    for bb in &func.blocks {
        arch.emit_block_label(state, func, bb);

        // Emit instructions
        for inst in &bb.insts {
            generate_instruction(arch, state, inst);
        }

        // Emit terminator
        generate_terminator(arch, state, &bb.terminator);
    }

    // Emit function epilogue
    arch.emit_function_epilogue(state, func);
}

/// Generate assembly for a single instruction.
fn generate_instruction(
    arch: &dyn ArchCodegen,
    state: &mut CodegenState,
    inst: &Instruction,
) {
    match inst {
        Instruction::Alloca {
            result, ty, align,
        } => {
            arch.emit_alloca(state, *result, ty, *align);
        }
        Instruction::Store { addr, value, ty } => {
            arch.emit_store(state, addr, value, ty);
        }
        Instruction::Load { result, addr, ty } => {
            arch.emit_load(state, *result, addr, ty);
        }
        Instruction::BinOp {
            result, op, lhs, rhs, ty,
        } => {
            arch.emit_binop(state, *result, *op, lhs, rhs, ty);
        }
        Instruction::UnaryOp {
            result, op, operand, ty,
        } => {
            arch.emit_unaryop(state, *result, *op, operand, ty);
        }
        Instruction::Icmp {
            result, pred, lhs, rhs, ty,
        } => {
            arch.emit_icmp(state, *result, *pred, lhs, rhs, ty);
        }
        Instruction::Fcmp {
            result, pred, lhs, rhs, ty,
        } => {
            arch.emit_fcmp(state, *result, *pred, lhs, rhs, ty);
        }
        Instruction::Cast {
            result, kind, src, src_ty, dst_ty,
        } => {
            arch.emit_cast(state, *result, *kind, src, src_ty, dst_ty);
        }
        Instruction::Call {
            result, callee, args, ret_ty, is_variadic,
        } => {
            arch.emit_call(state, *result, callee, args, ret_ty, *is_variadic);
        }
        Instruction::CallIndirect {
            result, func_ptr, args, ret_ty, is_variadic,
        } => {
            arch.emit_call_indirect(state, *result, func_ptr, args, ret_ty, *is_variadic);
        }
        Instruction::GetElementPtr {
            result, base, offset, elem_ty,
        } => {
            arch.emit_gep(state, *result, base, offset, elem_ty);
        }
        Instruction::GlobalAddr { result, name } => {
            arch.emit_global_addr(state, *result, name);
        }
        Instruction::Select {
            result, cond, true_val, false_val, ty,
        } => {
            arch.emit_select(state, *result, cond, true_val, false_val, ty);
        }
        Instruction::Phi {
            result, ty, incoming,
        } => {
            arch.emit_phi(state, *result, ty, incoming);
        }
        Instruction::Copy { result, src } => {
            // Copy is like a move: materialize src into result's location
            let loc = match src {
                Operand::Value(v) => state.get_value_location(*v).clone(),
                Operand::Const(c) => crate::backend::native::state::ValueLocation::Const(c.clone()),
                Operand::Global(n) => crate::backend::native::state::ValueLocation::Global(n.clone()),
                Operand::Label(_) => crate::backend::native::state::ValueLocation::Unassigned,
            };
            state.set_value_location(*result, loc);
        }
        Instruction::Nop
        | Instruction::DynAlloca { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::AtomicLoad { .. }
        | Instruction::AtomicStore { .. }
        | Instruction::AtomicRmw { .. }
        | Instruction::AtomicCmpxchg { .. }
        | Instruction::StackRestore { .. }
        | Instruction::InlineAsm { .. } => {
            // TODO: implement these as needed
            state.emit_inst(&format!("# TODO: {:?}", std::mem::discriminant(inst)));
        }
    }
}

/// Generate assembly for a terminator.
fn generate_terminator(
    arch: &dyn ArchCodegen,
    state: &mut CodegenState,
    term: &Terminator,
) {
    match term {
        Terminator::Ret { value } => {
            arch.emit_ret(state, value);
        }
        Terminator::Br { target } => {
            arch.emit_br(state, *target);
        }
        Terminator::CondBr {
            cond, true_bb, false_bb,
        } => {
            arch.emit_cond_br(state, cond, *true_bb, *false_bb);
        }
        Terminator::Switch {
            discr, ty, default, cases,
        } => {
            arch.emit_switch(state, discr, ty, *default, cases);
        }
        Terminator::Unreachable => {
            state.emit_inst("ud2");
        }
        Terminator::IndirectBr { .. } => {
            state.emit_inst("# TODO: indirectbr");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic smoke test that the driver can be called
    #[test]
    fn test_generate_empty_module() {
        // We'd need a concrete ArchCodegen implementation to actually test this.
        // For now, just verify the module structure compiles.
        let module = IrModule::new("test.c");
        assert!(module.functions.is_empty());
    }
}
