// ir/display.rs — Textual dump of the IR for debugging.
//
// Produces output similar to LLVM IR syntax, e.g.:
//   define i32 @main(i32 %0, ptr %1) {
//   entry:
//     %2 = alloca i32, align 4
//     store i32 %0, ptr %2
//     %3 = load i32, ptr %2
//     ret i32 %3
//   }

use std::fmt;
use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::module::*;
use crate::ir::types::*;

/// Format an entire IrModule as human-readable IR text.
impl fmt::Display for IrModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "; ModuleID = '{}'", self.source_file)?;
        writeln!(f)?;

        // Extern declarations
        for ext in &self.externs {
            write!(f, "declare {} @{}(", ext.ret_ty, ext.name)?;
            for (i, p) in ext.params.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", p)?;
            }
            if ext.is_variadic {
                if !ext.params.is_empty() {
                    write!(f, ", ")?;
                }
                write!(f, "...")?;
            }
            writeln!(f, ")")?;
        }
        if !self.externs.is_empty() {
            writeln!(f)?;
        }

        // Global variables
        for gv in &self.globals {
            write!(f, "@{} = ", gv.name)?;
            match gv.linkage {
                Linkage::Internal => write!(f, "internal ")?,
                Linkage::External => write!(f, "global ")?,
                Linkage::Weak => write!(f, "weak ")?,
                Linkage::Common => write!(f, "common ")?,
                Linkage::Private => write!(f, "private ")?,

            }
            if gv.is_const {
                write!(f, "constant ")?;
            }
            write!(f, "{}", gv.ty)?;
            if let Some(init) = &gv.init {
                write!(f, " ")?;
                display_global_init(f, init)?;
            }
            if gv.align > 0 {
                write!(f, ", align {}", gv.align)?;
            }
            writeln!(f)?;
        }
        if !self.globals.is_empty() {
            writeln!(f)?;
        }

        // String literals
        for (label, bytes) in &self.string_literals {
            write!(f, "@{} = private unnamed_addr constant [{} x i8] c\"",
                label, bytes.len())?;
            for &b in bytes {
                if b == b'\\' {
                    write!(f, "\\\\")?;
                } else if b == b'"' {
                    write!(f, "\\\"")?;
                } else if (0x20..0x7f).contains(&b) && b != b'\\' {
                    write!(f, "{}", b as char)?;
                } else {
                    write!(f, "\\{:02x}", b)?;
                }
            }
            writeln!(f, "\"")?;
        }
        if !self.string_literals.is_empty() {
            writeln!(f)?;
        }

        // Function definitions
        for func in &self.functions {
            display_function(f, func)?;
            writeln!(f)?;
        }

        Ok(())
    }
}

fn display_global_init(f: &mut fmt::Formatter<'_>, init: &GlobalInit) -> fmt::Result {
    match init {
        GlobalInit::Integer(v) => write!(f, "{}", v),
        GlobalInit::Float(v) => write!(f, "{:e}", v),
        GlobalInit::String(bytes) => {
            write!(f, "c\"")?;
            for &b in bytes {
                if (0x20..0x7f).contains(&b) && b != b'\\' && b != b'"' {
                    write!(f, "{}", b as char)?;
                } else {
                    write!(f, "\\{:02x}", b)?;
                }
            }
            write!(f, "\"")
        }
        GlobalInit::Address { symbol, offset } => {
            write!(f, "@{}", symbol)?;
            if *offset != 0 {
                write!(f, " + {}", offset)?;
            }
            Ok(())
        }
        GlobalInit::Compound(parts) => {
            write!(f, "{{ ")?;
            for (i, (off, sub)) in parts.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "/*+{}*/ ", off)?;
                display_global_init(f, sub)?;
            }
            write!(f, " }}")
        }
        GlobalInit::ZeroFill(n) => write!(f, "zeroinitializer({})", n),
        GlobalInit::LabelDiff { pos, neg } => write!(f, "{} - {}", pos, neg),
    }
}

fn display_function(f: &mut fmt::Formatter<'_>, func: &IrFunction) -> fmt::Result {
    // Linkage prefix
    match func.linkage {
        Linkage::Internal => write!(f, "define internal ")?,
        _ => write!(f, "define ")?,
    }

    // Return type and name
    write!(f, "{} @{}(", func.ret_ty, func.name)?;

    // Parameters
    for (i, p) in func.params.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{} %{}", p.ty, p.value.0)?;
    }
    if func.is_variadic {
        if !func.params.is_empty() {
            write!(f, ", ")?;
        }
        write!(f, "...")?;
    }
    writeln!(f, ") {{")?;

    // Blocks
    for bb in &func.blocks {
        writeln!(f, "{}:", bb.label)?;
        for inst in &bb.insts {
            write!(f, "    ")?;
            display_instruction(f, inst)?;
            writeln!(f)?;
        }
        write!(f, "    ")?;
        display_terminator(f, &bb.terminator)?;
        writeln!(f)?;
        writeln!(f)?;
    }

    write!(f, "}}")
}

fn display_instruction(f: &mut fmt::Formatter<'_>, inst: &Instruction) -> fmt::Result {
    match inst {
        Instruction::Alloca { result, ty, align } => {
            write!(f, "%{} = alloca {}, align {}", result.0, ty, align)
        }
        Instruction::DynAlloca {
            result, ty, count, ..
        } => {
            write!(f, "%{} = dynalloca {}, {}", result.0, ty, count)
        }
        Instruction::Store { addr, value, ty } => {
            write!(f, "store {} {}, ptr {}", ty, value, addr)
        }
        Instruction::Load { result, addr, ty } => {
            write!(f, "%{} = load {}, ptr {}", result.0, ty, addr)
        }
        Instruction::BinOp {
            result,
            op,
            lhs,
            rhs,
            ty,
        } => {
            write!(f, "%{} = {} {} {}, {}", result.0, op, ty, lhs, rhs)
        }
        Instruction::UnaryOp {
            result,
            op,
            operand,
            ty,
        } => {
            write!(f, "%{} = {} {} {}", result.0, op, ty, operand)
        }
        Instruction::Icmp {
            result,
            pred,
            lhs,
            rhs,
            ty,
        } => {
            write!(
                f,
                "%{} = icmp {} {} {}, {}",
                result.0, pred, ty, lhs, rhs
            )
        }
        Instruction::Fcmp {
            result,
            pred,
            lhs,
            rhs,
            ty,
        } => {
            write!(
                f,
                "%{} = fcmp {} {} {}, {}",
                result.0, pred, ty, lhs, rhs
            )
        }
        Instruction::Cast {
            result,
            kind,
            src,
            src_ty,
            dst_ty,
        } => {
            write!(
                f,
                "%{} = {} {} {} to {}",
                result.0, kind, src_ty, src, dst_ty
            )
        }
        Instruction::Call {
            result,
            callee,
            args,
            ret_ty,
            ..
        } => {
            if *ret_ty != IrType::Void {
                write!(f, "%{} = ", result.0)?;
            }
            write!(f, "call {} @{}(", ret_ty, callee)?;
            for (i, (op, ty)) in args.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{} {}", ty, op)?;
            }
            write!(f, ")")
        }
        Instruction::CallIndirect {
            result,
            func_ptr,
            args,
            ret_ty,
            ..
        } => {
            if *ret_ty != IrType::Void {
                write!(f, "%{} = ", result.0)?;
            }
            write!(f, "call {} {}(", ret_ty, func_ptr)?;
            for (i, (op, ty)) in args.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{} {}", ty, op)?;
            }
            write!(f, ")")
        }
        Instruction::GetElementPtr {
            result,
            base,
            offset,
            elem_ty,
        } => {
            write!(
                f,
                "%{} = getelementptr {}, ptr {}, {}",
                result.0, elem_ty, base, offset
            )
        }
        Instruction::GlobalAddr { result, name } => {
            write!(f, "%{} = globaladdr @{}", result.0, name)
        }
        Instruction::LabelAddr { result, block } => {
            write!(f, "%{} = labeladdr bb{}", result.0, block.0)
        }
        Instruction::Select {
            result,
            cond,
            true_val,
            false_val,
            ty,
        } => {
            write!(
                f,
                "%{} = select i1 {}, {} {}, {} {}",
                result.0, cond, ty, true_val, ty, false_val
            )
        }
        Instruction::Copy { result, src } => {
            write!(f, "%{} = copy {}", result.0, src)
        }
        Instruction::Phi {
            result,
            ty,
            incoming,
        } => {
            write!(f, "%{} = phi {} ", result.0, ty)?;
            for (i, (bb, val)) in incoming.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "[{}, bb{}]", val, bb.0)?;
            }
            Ok(())
        }
        Instruction::AtomicLoad {
            result,
            addr,
            ty,
            ordering,
        } => {
            write!(
                f,
                "%{} = atomic_load {} ptr {}, {:?}",
                result.0, ty, addr, ordering
            )
        }
        Instruction::AtomicStore {
            addr,
            value,
            ty,
            ordering,
        } => {
            write!(
                f,
                "atomic_store {} {}, ptr {}, {:?}",
                ty, value, addr, ordering
            )
        }
        Instruction::AtomicRmw {
            result,
            op,
            addr,
            value,
            ty,
            ordering,
        } => {
            write!(
                f,
                "%{} = atomicrmw {:?} {} ptr {}, {}, {:?}",
                result.0, op, ty, addr, value, ordering
            )
        }
        Instruction::AtomicCmpxchg {
            result,
            addr,
            expected,
            desired,
            ty,
            success_ordering,
            failure_ordering,
        } => {
            write!(
                f,
                "%{} = cmpxchg ptr {}, {} {}, {} {}, {:?} {:?}",
                result.0, addr, ty, expected, ty, desired,
                success_ordering, failure_ordering
            )
        }
        Instruction::StackRestore { saved_sp } => {
            write!(f, "stackrestore {}", saved_sp)
        }
        Instruction::InlineAsm {
            result,
            template,
            constraints,
            operands,
            ..
        } => {
            write!(
                f,
                "%{} = asm \"{}\" \"{}\"(",
                result.0, template, constraints
            )?;
            for (i, (op, ty)) in operands.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{} {}", ty, op)?;
            }
            write!(f, ")")
        }
        Instruction::Nop => write!(f, "nop"),
    }
}

fn display_terminator(f: &mut fmt::Formatter<'_>, term: &Terminator) -> fmt::Result {
    match term {
        Terminator::Ret { value: Some(v) } => write!(f, "ret {}", v),
        Terminator::Ret { value: None } => write!(f, "ret void"),
        Terminator::Br { target } => write!(f, "br bb{}", target.0),
        Terminator::CondBr {
            cond,
            true_bb,
            false_bb,
        } => {
            write!(
                f,
                "br i1 {}, bb{}, bb{}",
                cond, true_bb.0, false_bb.0
            )
        }
        Terminator::Switch {
            discr,
            ty,
            default,
            cases,
        } => {
            write!(f, "switch {} {}, bb{} [", ty, discr, default.0)?;
            for (val, bb) in cases {
                write!(f, " {}: bb{}", val, bb.0)?;
            }
            write!(f, " ]")
        }
        Terminator::IndirectBr { addr, targets } => {
            write!(f, "indirectbr {}, [", addr)?;
            for (i, t) in targets.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "bb{}", t.0)?;
            }
            write!(f, "]")
        }
        Terminator::Unreachable => write!(f, "unreachable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::instruction::*;

    #[test]
    fn test_display_empty_module() {
        let m = IrModule::new("empty.c");
        let s = format!("{}", m);
        assert!(s.contains("; ModuleID = 'empty.c'"));
    }

    #[test]
    fn test_display_simple_function() {
        let mut m = IrModule::new("test.c");
        let mut f = IrFunction::new("add", IrType::I32, Linkage::External);
        let a = f.add_param("a", IrType::I32);
        let b = f.add_param("b", IrType::I32);
        let entry = f.create_block("entry");
        let result = f.alloc_value();
        f.block_mut(entry).push(Instruction::BinOp {
            result,
            op: BinOpKind::Add,
            lhs: Operand::Value(a),
            rhs: Operand::Value(b),
            ty: IrType::I32,
        });
        f.block_mut(entry).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(result)),
        });
        m.add_function(f);

        let s = format!("{}", m);
        assert!(s.contains("define i32 @add(i32 %0, i32 %1)"));
        assert!(s.contains("%2 = add i32 %0, %1"));
        assert!(s.contains("ret %2"));
    }

    #[test]
    fn test_display_extern() {
        let mut m = IrModule::new("test.c");
        m.add_extern(ExternFunc {
            name: "printf".into(),
            ret_ty: IrType::I32,
            params: vec![IrType::Ptr],
            is_variadic: true,
        });
        let s = format!("{}", m);
        assert!(s.contains("declare i32 @printf(ptr, ...)"));
    }

    #[test]
    fn test_display_global() {
        let mut m = IrModule::new("test.c");
        m.add_global(GlobalVariable {
            name: "x".into(),
            ty: IrType::I32,
            init: Some(GlobalInit::Integer(42)),
            linkage: Linkage::External,
            visibility: Visibility::Default,
            section: None,
            align: 4,
            is_const: false,
            is_tls: false,
        });
        let s = format!("{}", m);
        assert!(s.contains("@x = global i32 42, align 4"));
    }

    #[test]
    fn test_display_phi() {
        let mut f = IrFunction::new("phi_test", IrType::I32, Linkage::External);
        let entry = f.create_block("entry");
        let bb1 = f.create_block("left");
        let bb2 = f.create_block("right");
        let merge = f.create_block("merge");

        f.block_mut(entry).set_terminator(Terminator::Br { target: bb1 });
        f.block_mut(bb1).set_terminator(Terminator::Br { target: merge });
        f.block_mut(bb2).set_terminator(Terminator::Br { target: merge });

        let phi_val = f.alloc_value();
        f.block_mut(merge).push(Instruction::Phi {
            result: phi_val,
            ty: IrType::I32,
            incoming: vec![
                (bb1, Operand::Const(ConstValue::I32(1))),
                (bb2, Operand::Const(ConstValue::I32(2))),
            ],
        });
        f.block_mut(merge).set_terminator(Terminator::Ret {
            value: Some(Operand::Value(phi_val)),
        });

        let mut m = IrModule::new("test.c");
        m.add_function(f);

        let s = format!("{}", m);
        assert!(s.contains("phi i32 [1, bb1], [2, bb2]"));
    }
}
