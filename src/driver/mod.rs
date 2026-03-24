// driver/mod.rs — Module declarations and public re-exports.
//
// The driver is the entry point and orchestrator of the entire compiler.

pub mod pipeline;
pub mod cli;
pub mod external_tools;
pub mod file_types;

pub use pipeline::{Driver, CompileMode};
