// backend/native/mod.rs — Native backend subsystem.
//
// This module contains the architecture-independent backend infrastructure:
//   - ArchCodegen trait (architecture-specific code generation interface)
//   - CodegenState (shared per-function state: stack, registers, output)
//   - Generation driver (walks IR, dispatches to ArchCodegen)
//   - Stack layout and register allocator
//   - ELF assembler and linker infrastructure

pub mod traits;
pub mod state;
pub mod generation;
pub mod regalloc;
pub mod elf;
pub mod linker;
pub mod x86_64;
