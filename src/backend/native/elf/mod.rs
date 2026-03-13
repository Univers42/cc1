// backend/native/elf/mod.rs — ELF binary format infrastructure.
//
// Provides types and utilities for:
// - ELF headers, sections, segments, symbols
// - Relocations
// - Object file (.o) emission (assembler output)
// - Executable linking

pub mod types;
pub mod writer;

pub use types::*;
