// ir/mod.rs — SSA Intermediate Representation for the native backend.
//
// The IR sits between the C89 AST and the architecture-specific code generators.
// It is a typed, SSA-form representation with explicit basic blocks, phi nodes,
// and terminators. All values are referenced by `ValueId` handles.

pub mod types;
pub mod module;
pub mod instruction;
pub mod display;
pub mod lower;
pub mod optimize;

pub use types::*;
pub use module::*;
pub use instruction::*;
