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
pub fn assemble_source_file_builtin(
    driver: &Driver,
    input: &str,
) -> Result<Vec<u8>, String> {
    let source = if super::file_types::is_assembly_with_cpp(input) {
        // .S files need C preprocessing first.
        // For now, just read directly. TODO: run built-in preprocessor with
        // __ASSEMBLER__ defined and assembly-mode tokenization enabled.
        let raw = std::fs::read_to_string(input)
            .map_err(|e| format!("cannot read '{}': {}", input, e))?;

        // Debug: dump preprocessed assembly if CCC_ASM_DEBUG is set.
        if std::env::var("CCC_ASM_DEBUG").is_ok() {
            let stem = std::path::Path::new(input)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let debug_path = format!("/tmp/asm_debug_{}.s", stem);
            let _ = std::fs::write(&debug_path, &raw);
            eprintln!("cc1: dumped preprocessed assembly to {}", debug_path);
        }

        raw
    } else {
        std::fs::read_to_string(input)
            .map_err(|e| format!("cannot read '{}': {}", input, e))?
    };

    // Route to the architecture-specific builtin assembler.
    match driver.target {
        crate::target::Target::X86_64 => {
            use crate::backend::native::x86_64::assembler::X86_64Assembler;
            let mut asm = X86_64Assembler::new();
            Ok(asm.assemble(&source))
        }
        crate::target::Target::I386 => {
            Err("i386 builtin assembler not yet implemented".into())
        }
    }
}

/// Assemble a source file using GCC as the external assembler.
#[allow(dead_code)]
pub fn assemble_source_file_gcc(
    driver: &Driver,
    input: &str,
    output: &str,
) -> Result<(), String> {
    let mut cmd = Command::new("gcc");
    cmd.arg("-c");
    cmd.arg("-o").arg(output);

    // Forward target.
    match driver.target {
        crate::target::Target::I386 => {
            cmd.arg("-m32");
        }
        crate::target::Target::X86_64 => {
            cmd.arg("-m64");
        }
    }

    // Forward include paths (for .S preprocessing).
    for p in &driver.include_paths {
        cmd.arg(format!("-I{}", p));
    }
    for p in &driver.isystem_include_paths {
        cmd.arg("-isystem").arg(p);
    }

    // Forward defines.
    for d in &driver.defines {
        if let Some(ref val) = d.value {
            cmd.arg(format!("-D{}={}", d.name, val));
        } else {
            cmd.arg(format!("-D{}", d.name));
        }
    }

    // Forward undefines.
    for u in &driver.undef_macros {
        cmd.arg(format!("-U{}", u));
    }

    if driver.nostdinc {
        cmd.arg("-nostdinc");
    }
    if driver.undef_all {
        cmd.arg("-undef");
    }

    // Forward force-include files.
    for f in &driver.force_includes {
        cmd.arg("-include").arg(f);
    }

    // Forward explicit language override.
    if let Some(ref lang) = driver.explicit_language {
        cmd.arg("-x").arg(lang);
    }

    // Forward RISC-V assembler flags.
    cmd.args(build_asm_extra_args(driver));

    // Forward extra assembler args.
    for a in &driver.assembler_extra_args {
        cmd.arg(format!("-Wa,{}", a));
    }

    cmd.arg(input);

    if driver.verbose {
        eprintln!("cc1: assembling with gcc: {:?}", cmd);
    }

    let status = cmd
        .status()
        .map_err(|e| format!("failed to invoke gcc for assembly: {}", e))?;

    if !status.success() {
        return Err(format!(
            "gcc assembly failed with exit code {}",
            status.code().unwrap_or(-1)
        ));
    }

    Ok(())
}

// ── Assembler argument construction ────────────────────────────────────

/// Build RISC-V-specific assembler flags.
pub fn build_asm_extra_args(driver: &Driver) -> Vec<String> {
    let mut args = Vec::new();

    // RISC-V: -mabi, -march, -mno-relax, -fno-pic
    if let Some(ref abi) = driver.riscv_abi {
        args.push(format!("-Wa,-mabi={}", abi));
    }
    if let Some(ref march) = driver.riscv_march {
        args.push(format!("-Wa,-march={}", march));
    }
    if driver.riscv_no_relax {
        args.push("-Wa,-mno-relax".into());
    }
    if !driver.pic {
        args.push("-Wa,-fno-pic".into());
    }

    args
}

// ── Linker argument construction ───────────────────────────────────────

/// Build the ordered list of linker arguments.
///
/// Returns (flags, positional_items) where:
///   - flags: order-independent flags (-nostdlib, -shared, -static, -L paths)
///   - positional_items: ordered object files, -l flags, -Wl, pass-through
#[allow(dead_code)]
pub fn build_linker_args(driver: &Driver) -> (Vec<String>, Vec<String>) {
    let mut flags = Vec::new();
    let mut items = Vec::new();

    // Order-independent flags.
    if driver.relocatable {
        flags.push("-nostdlib".into());
    }
    if driver.shared_lib {
        flags.push("-shared".into());
    }
    if driver.static_link {
        flags.push("-static".into());
    }
    if driver.nostdlib {
        flags.push("-nostdlib".into());
    }
    for p in &driver.linker_paths {
        flags.push(format!("-L{}", p));
    }

    // Positional items from linker_ordered_items.
    for item in &driver.linker_ordered_items {
        items.push(item.clone());
    }

    (flags, items)
}

// ── Dependency file generation ─────────────────────────────────────────

/// Write a Make-compatible dependency file.
///
/// Format: `target: source\n`
/// Currently minimal — lists only the source file as a dependency,
/// not included headers. Sufficient for Linux kernel's fixdep processing.
#[allow(dead_code)]
pub fn write_dep_file(
    dep_path: &str,
    target: &str,
    source: &str,
) -> Result<(), String> {
    let content = format!("{}: {}\n", target, source);
    std::fs::write(dep_path, &content)
        .map_err(|e| format!("cannot write dependency file '{}': {}", dep_path, e))
}

/// Derive the dependency file path from the output path.
#[allow(dead_code)]
pub fn derive_dep_path(output: &str) -> String {
    if let Some(dot) = output.rfind('.') {
        format!("{}.d", &output[..dot])
    } else {
        format!("{}.d", output)
    }
}

// ── Environment variable helpers ───────────────────────────────────────

/// Check if CCC_KEEP_ASM is set (preserve intermediate .s files).
#[allow(dead_code)]
pub fn keep_asm_files() -> bool {
    std::env::var("CCC_KEEP_ASM").is_ok()
}

/// Check if CCC_ASM_DEBUG is set (dump preprocessed assembly).
#[allow(dead_code)]
pub fn asm_debug() -> bool {
    std::env::var("CCC_ASM_DEBUG").is_ok()
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::pipeline::Driver;

    #[test]
    fn test_build_asm_extra_args_empty() {
        let d = Driver::new();
        assert!(build_asm_extra_args(&d).is_empty() || build_asm_extra_args(&d).contains(&"-Wa,-fno-pic".into()));
    }

    #[test]
    fn test_build_asm_extra_args_riscv() {
        let mut d = Driver::new();
        d.riscv_abi = Some("lp64".into());
        d.riscv_march = Some("rv64imac".into());
        d.riscv_no_relax = true;
        let args = build_asm_extra_args(&d);
        assert!(args.contains(&"-Wa,-mabi=lp64".into()));
        assert!(args.contains(&"-Wa,-march=rv64imac".into()));
        assert!(args.contains(&"-Wa,-mno-relax".into()));
    }

    #[test]
    fn test_build_linker_args() {
        let mut d = Driver::new();
        d.static_link = true;
        d.linker_paths.push("/usr/lib".into());
        d.linker_ordered_items.push("-lfoo".into());
        d.linker_ordered_items.push("bar.o".into());

        let (flags, items) = build_linker_args(&d);
        assert!(flags.iter().any(|f| f == "-static"));
        assert!(flags.iter().any(|f| f == "-L/usr/lib"));
        assert_eq!(items, vec!["-lfoo", "bar.o"]);
    }

    #[test]
    fn test_derive_dep_path() {
        assert_eq!(derive_dep_path("foo.o"), "foo.d");
        assert_eq!(derive_dep_path("a/b/c.o"), "a/b/c.d");
        assert_eq!(derive_dep_path("noext"), "noext.d");
    }

    #[test]
    fn test_write_dep_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("test_dep.d");
        let path_str = path.to_str().unwrap();
        write_dep_file(path_str, "foo.o", "foo.c").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "foo.o: foo.c\n");
        let _ = std::fs::remove_file(&path);
    }
}
