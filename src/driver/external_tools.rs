// driver/external_tools.rs — External tool invocation and assembly handling.
//
// Handles GCC -m16 delegation, source .s/.S file assembly (builtin and GCC-backed),
// assembler argument construction, linker argument construction, and dependency
// file generation.
//
// Currently, the compiler uses the builtin assembler and linker exclusively.
// This module provides the infrastructure for optional GCC delegation when
// Cargo features `gcc_assembler` or `gcc_linker` are enabled in the future.

use std::process::Command;

use super::pipeline::Driver;

// ── GCC -m16 delegation ───────────────────────────────────────────────

/// Delegate compilation to GCC for -m16 (16-bit real mode boot code).
///
/// The -m16 flag generates i386 code with .code16gcc prepended for 16-bit
/// real mode execution (used by the Linux kernel at arch/x86/boot/).
///
/// This is a temporary hack — TODO: Remove once i686 code size optimizations
/// bring boot code under 32KB.
#[allow(dead_code)]
pub fn compile_with_gcc_m16(driver: &Driver, input: &str, output: &str) -> Result<(), String> {
    let mut cmd = Command::new("gcc");

    // Forward raw CLI args, stripping -o, -c, -S (we add them back).
    for arg in &driver.raw_args {
        match arg.as_str() {
            "-o" | "-c" | "-S" => continue,
            a if a.starts_with("-o") => continue,
            _ => {
                cmd.arg(arg);
            }
        }
    }

    // Re-add output control.
    cmd.arg("-S"); // We want assembly output.
    cmd.arg("-o").arg(output);
    cmd.arg("-w"); // Suppress GCC warnings.
    cmd.arg(input);

    if driver.verbose {
        eprintln!("cc1: delegating to gcc for -m16: {:?}", cmd);
    }

    let status = cmd
        .status()
        .map_err(|e| format!("failed to invoke gcc: {}", e))?;

    if !status.success() {
        return Err(format!(
            "gcc -m16 compilation failed with exit code {}",
            status.code().unwrap_or(-1)
        ));
    }

    Ok(())
}

// ── Source assembly file handling ──────────────────────────────────────

/// Assemble a source .s/.S file using the builtin assembler.
///
/// For .S files, the built-in C preprocessor is run first (with __ASSEMBLER__
/// defined and assembly-mode tokenization enabled). For .s files, the content
/// is read directly.
#[allow(dead_code)]
