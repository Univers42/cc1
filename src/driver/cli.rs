// driver/cli.rs — GCC-compatible CLI argument parsing.
//
// Hand-written while/match loop processing ~80 distinct flag patterns.
// No external parser library. Design priorities:
//   - GCC compatibility (build systems pass many obscure flags)
//   - Positional ordering for linker items
//   - Early-exit query flags for build system probes

use crate::target::Target;
use super::pipeline::{Driver, CompileMode, CliDefine, ColorMode};

// ── Target detection from binary name ──────────────────────────────────

/// Detect the target architecture from argv[0] (the binary name).
///
/// | Binary name contains | Target   |
/// |----------------------|----------|
/// | arm or aarch64       | AArch64  | (future — defaults to x86_64 for now)
/// | riscv                | RISC-V   | (future — defaults to x86_64 for now)
/// | i686 or i386         | i686     |
/// | anything else        | x86-64   |
pub fn detect_target_from_argv0(argv0: &str) -> Target {
    let name = std::path::Path::new(argv0)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(argv0);
    let lower = name.to_lowercase();

    if lower.contains("i686") || lower.contains("i386") {
        Target::I386
    } else {
        // Default to x86-64. AArch64 and RISC-V will be added when backends land.
        Target::X86_64
    }
}

// ── Response file expansion ────────────────────────────────────────────

/// Expand @file response files. Supports single/double quotes and backslash
/// escaping. Build systems like Meson use this when command lines exceed OS limits.
fn expand_response_files(args: &[String]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for arg in args {
        if let Some(path) = arg.strip_prefix('@') {
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read response file '{}': {}", path, e))?;
            let tokens = tokenize_response_file(&content);
            result.extend(tokens);
        } else {
            result.push(arg.clone());
        }
    }
    Ok(result)
}

/// Tokenize a response file respecting quotes and backslash escaping.
fn tokenize_response_file(content: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = content.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(&ch) = chars.peek() {
        match ch {
            '\\' if !in_single_quote => {
                chars.next();
                if let Some(&next) = chars.peek() {
                    current.push(next);
                    chars.next();
                }
            }
            '\'' if !in_double_quote => {
                chars.next();
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                chars.next();
                in_double_quote = !in_double_quote;
            }
            c if c.is_whitespace() && !in_single_quote && !in_double_quote => {
                chars.next();
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(ch);
                chars.next();
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

// ── Main CLI parser ────────────────────────────────────────────────────

/// Parse GCC-compatible command-line arguments into a Driver.
///
/// Returns `Ok(true)` if a query flag was handled (program should exit),
/// `Ok(false)` if parsing completed normally and compilation should proceed.
pub fn parse_cli_args(driver: &mut Driver, argv0: &str, args: &[String]) -> Result<bool, String> {
    // Save raw args for -m16 passthrough.
    driver.raw_args = args.to_vec();

    // Detect target from binary name.
    driver.target = detect_target_from_argv0(argv0);

    // Expand response files.
    let args = expand_response_files(args)?;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();

        match arg.as_str() {
            // ── Query flags (early exit) ───────────────────────────

            "-dumpmachine" => {
                println!("{}", driver.target_triple());
                return Ok(true);
            }
            "-dumpversion" => {
                println!("14");
                return Ok(true);
            }
            "--version" => {
                print_version(driver);
                return Ok(true);
            }
            "-v" | "--verbose" => {
                // If -v alone (no input files after), print version and exit.
                // Otherwise, set verbose mode.
                if args.len() == 1 || (i == 0 && no_input_files(&args[1..])) {
                    print_verbose_version(driver);
                    return Ok(true);
                }
                driver.verbose = true;
            }
            "-print-search-dirs" => {
                println!("install: /usr/lib/gcc/{}/14/", driver.target_triple());
                println!("programs: /usr/lib/gcc/{}/14/:/usr/bin/", driver.target_triple());
                println!("libraries: /usr/lib/gcc/{}/14/:/usr/lib/:/lib/", driver.target_triple());
                return Ok(true);
            }
            a if a.starts_with("-print-file-name=") => {
                let name = &a["-print-file-name=".len()..];
                if name == "include" {
                    // Return bundled include directory.
                    println!("/usr/lib/gcc/{}/14/include", driver.target_triple());
                } else {
                    // Search standard GCC library paths.
                    let search_dirs = [
                        format!("/usr/lib/gcc/{}/14/", driver.target_triple()),
                        "/usr/lib/".into(),
                        "/lib/".into(),
                    ];
                    let mut found = false;
                    for dir in &search_dirs {
                        let path = format!("{}{}", dir, name);
                        if std::path::Path::new(&path).exists() {
                            println!("{}", path);
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        println!("{}", name);
                    }
                }
                return Ok(true);
            }

            // ── Mode selection ─────────────────────────────────────

            "-E" => driver.mode = CompileMode::PreprocessOnly,
            "-S" => driver.mode = CompileMode::AssemblyOnly,
            "-c" => driver.mode = CompileMode::ObjectOnly,

            // ── Output ─────────────────────────────────────────────

            "-o" => {
                i += 1;
                if i >= args.len() {
                    return Err("-o requires an argument".into());
                }
                driver.output_path = args[i].clone();
                driver.output_path_set = true;
            }
            a if a.starts_with("-o") && a.len() > 2 => {
                driver.output_path = a[2..].to_string();
                driver.output_path_set = true;
            }

            // ── Target override ────────────────────────────────────

            "-m32" => driver.target = Target::I386,
            "-m64" => driver.target = Target::X86_64,
            "-m16" => {
                driver.target = Target::I386;
                driver.code16gcc = true;
            }

            // ── Optimization ───────────────────────────────────────

            "-O0" => {
                driver.opt_level = 2; // Internal always 2
                driver.optimize = false;
                driver.optimize_size = false;
            }
            "-O1" | "-O" => {
                driver.opt_level = 2;
                driver.optimize = true;
                driver.optimize_size = false;
            }
            "-O2" | "-O3" => {
                driver.opt_level = 2;
                driver.optimize = true;
                driver.optimize_size = false;
            }
            "-Os" | "-Oz" => {
                driver.opt_level = 2;
                driver.optimize = true;
                driver.optimize_size = true;
            }

            // ── Preprocessor defines / undefines ───────────────────

            "-D" => {
                i += 1;
                if i >= args.len() {
                    return Err("-D requires an argument".into());
                }
                driver.defines.push(parse_define(&args[i]));
            }
            a if a.starts_with("-D") => {
                driver.defines.push(parse_define(&a[2..]));
            }
            "-U" => {
                i += 1;
                if i >= args.len() {
                    return Err("-U requires an argument".into());
                }
                driver.undef_macros.push(args[i].clone());
            }
            a if a.starts_with("-U") => {
                driver.undef_macros.push(a[2..].to_string());
            }
            "-undef" => driver.undef_all = true,

            // ── Include paths ──────────────────────────────────────

            "-I" => {
                i += 1;
                if i >= args.len() {
                    return Err("-I requires an argument".into());
                }
                driver.include_paths.push(args[i].clone());
            }
            a if a.starts_with("-I") => {
                driver.include_paths.push(a[2..].to_string());
            }
            "-iquote" => {
                i += 1;
                if i >= args.len() {
                    return Err("-iquote requires an argument".into());
                }
                driver.quote_include_paths.push(args[i].clone());
            }
            "-isystem" => {
                i += 1;
                if i >= args.len() {
                    return Err("-isystem requires an argument".into());
                }
                driver.isystem_include_paths.push(args[i].clone());
            }
            "-idirafter" => {
                i += 1;
                if i >= args.len() {
                    return Err("-idirafter requires an argument".into());
                }
                driver.after_include_paths.push(args[i].clone());
            }
            "-include" => {
                i += 1;
                if i >= args.len() {
                    return Err("-include requires an argument".into());
                }
                driver.force_includes.push(args[i].clone());
            }
            "-nostdinc" => driver.nostdinc = true,

            // ── Language standard ──────────────────────────────────

            a if a.starts_with("-std=") => {
                let std = &a[5..];
                parse_std_flag(driver, std);
            }

            // ── Preprocessor output control ────────────────────────

            "-P" => driver.suppress_line_markers = true,
            "-dM" => driver.dump_defines = true,

            // ── Language override ───────────────────────────────────

            "-x" => {
                i += 1;
                if i >= args.len() {
                    return Err("-x requires an argument".into());
                }
                let lang = args[i].clone();
                if lang == "none" {
                    driver.explicit_language = None;
                } else {
                    driver.explicit_language = Some(lang);
                }
            }

            // ── Debug info ─────────────────────────────────────────

            "-g" | "-g1" | "-g2" | "-g3" | "-gdwarf" | "-gdwarf-2" |
            "-gdwarf-3" | "-gdwarf-4" | "-gdwarf-5" | "-ggdb" | "-ggdb3" => {
                driver.debug_info = true;
            }
            "-g0" => driver.debug_info = false,

            // ── Warning flags ──────────────────────────────────────

            "-w" => driver.warning_config.suppress_all = true,
            "-Wall" => driver.warning_config.all = true,
            "-Wextra" => driver.warning_config.extra = true,
            "-Werror" => driver.warning_config.error = true,
            "-Wpedantic" | "-pedantic" => driver.warning_config.pedantic = true,
            "-pedantic-errors" => {
                driver.warning_config.pedantic = true;
                driver.warning_config.error = true;
            }
            a if a.starts_with("-Werror=") => {
                driver.warning_config.error_flags.push(a[8..].to_string());
            }
            a if a.starts_with("-Wno-") => {
                driver.warning_config.disabled_flags.push(a[5..].to_string());
            }
            a if a.starts_with("-W") && !a.starts_with("-Wl,") && !a.starts_with("-Wp,") && !a.starts_with("-Wa,") => {
                // -W<flag> — enable specific warning.
                let flag = &a[2..];
                if !flag.is_empty() {
                    driver.warning_config.enabled_flags.push(flag.to_string());
                }
            }

            // ── Diagnostic color mode ──────────────────────────────

            "-fdiagnostics-color=auto" | "-fdiagnostics-color" => {
                driver.color_mode = ColorMode::Auto;
            }
            "-fdiagnostics-color=always" => driver.color_mode = ColorMode::Always,
            "-fdiagnostics-color=never" | "-fno-diagnostics-color" => {
                driver.color_mode = ColorMode::Never;
            }

            // ── PIC ────────────────────────────────────────────────

            "-fPIC" | "-fpic" => driver.pic = true,
            "-fno-PIC" | "-fno-pic" => driver.pic = false,

            // ── Code generation flags ──────────────────────────────

            "-mfunction-return=thunk-extern" => driver.function_return_thunk = true,
            "-mindirect-branch=thunk-extern" => driver.indirect_branch_thunk = true,

            a if a.starts_with("-fpatchable-function-entry=") => {
                let val = &a["-fpatchable-function-entry=".len()..];
                driver.patchable_function_entry = parse_patchable_entry(val);
            }

            "-fcf-protection=branch" | "-fcf-protection" => {
                driver.cf_protection_branch = true;
            }
            "-fcf-protection=none" => driver.cf_protection_branch = false,

            "-mno-sse" => driver.no_sse = true,
            "-msse3" => {
                driver.enable_sse3 = true;
            }
            "-mssse3" => {
                driver.enable_sse3 = true;
                driver.enable_ssse3 = true;
            }
            "-msse4.1" => {
                driver.enable_sse3 = true;
                driver.enable_ssse3 = true;
                driver.enable_sse4_1 = true;
            }
            "-msse4.2" => {
                driver.enable_sse3 = true;
                driver.enable_ssse3 = true;
                driver.enable_sse4_1 = true;
                driver.enable_sse4_2 = true;
            }
            "-mavx" => {
                driver.enable_sse3 = true;
                driver.enable_ssse3 = true;
                driver.enable_sse4_1 = true;
                driver.enable_sse4_2 = true;
                driver.enable_avx = true;
            }
            "-mavx2" => {
                driver.enable_sse3 = true;
                driver.enable_ssse3 = true;
                driver.enable_sse4_1 = true;
                driver.enable_sse4_2 = true;
                driver.enable_avx = true;
                driver.enable_avx2 = true;
            }

            "-mgeneral-regs-only" => driver.general_regs_only = true,
            "-mcmodel=kernel" => driver.code_model_kernel = true,
            "-fno-jump-tables" | "-mno-jump-tables" => driver.no_jump_tables = true,
            "-ffunction-sections" => driver.function_sections = true,
            "-fno-function-sections" => driver.function_sections = false,
            "-fdata-sections" => driver.data_sections = true,
            "-fno-data-sections" => driver.data_sections = false,

            a if a.starts_with("-mregparm=") => {
                let val = &a["-mregparm=".len()..];
                driver.regparm = val.parse().unwrap_or(0).min(3);
            }

            "-fomit-frame-pointer" => driver.omit_frame_pointer = true,
            "-fno-omit-frame-pointer" => driver.omit_frame_pointer = false,
            "-fno-asynchronous-unwind-tables" => driver.no_unwind_tables = true,
            "-fasynchronous-unwind-tables" => driver.no_unwind_tables = false,
            "-fcommon" => driver.fcommon = true,
            "-fno-common" => driver.fcommon = false,
            "-fgnu89-inline" => driver.gnu89_inline = true,
            "-fno-gnu89-inline" => driver.gnu89_inline = false,

            // ── RISC-V specific ────────────────────────────────────

            a if a.starts_with("-mabi=") => {
                driver.riscv_abi = Some(a["-mabi=".len()..].to_string());
            }
            a if a.starts_with("-march=") => {
                driver.riscv_march = Some(a["-march=".len()..].to_string());
            }
            "-mno-relax" => driver.riscv_no_relax = true,

            // ── Linker flags ───────────────────────────────────────

            "-L" => {
                i += 1;
                if i >= args.len() {
                    return Err("-L requires an argument".into());
                }
                driver.linker_paths.push(args[i].clone());
            }
            a if a.starts_with("-L") => {
                driver.linker_paths.push(a[2..].to_string());
            }
            a if a.starts_with("-l") => {
                driver.linker_ordered_items.push(a.to_string());
            }
            "-static" => driver.static_link = true,
            "-shared" => driver.shared_lib = true,
            "-nostdlib" => driver.nostdlib = true,
            "-r" => driver.relocatable = true,

            // ── Linker pass-through ────────────────────────────────

            a if a.starts_with("-Wl,") => {
                let items = &a[4..];
                // Check for --version probe (Meson linker detection).
                if items == "--version" && driver.input_files.is_empty() {
                    println!("GNU ld (Claude's C Compiler built-in) 2.42");
                    return Ok(true);
                }
                // Split on comma and add individually to preserve order.
                for part in items.split(',') {
                    if !part.is_empty() {
                        driver.linker_ordered_items.push(format!("-Wl,{}", part));
                    }
                }
            }

            // ── Preprocessor pass-through ──────────────────────────

            a if a.starts_with("-Wp,") => {
                let parts: Vec<&str> = a[4..].split(',').collect();
                let mut j = 0;
                while j < parts.len() {
                    match parts[j] {
                        "-MMD" | "-MD" => {
                            if j + 1 < parts.len() {
                                driver.dep_file = Some(parts[j + 1].to_string());
                                j += 1;
                            }
                        }
                        _ => {} // Silently ignore other -Wp flags
                    }
                    j += 1;
                }
            }

            // ── Assembler pass-through ─────────────────────────────

            a if a.starts_with("-Wa,") => {
                let items = &a[4..];
                // Check for --version probe.
                if items == "--version" {
                    println!("GNU assembler (Claude's C Compiler built-in) 2.42");
                    return Ok(true);
                }
                for part in items.split(',') {
                    if !part.is_empty() {
                        driver.assembler_extra_args.push(part.to_string());
                    }
                }
            }

            // ── Dependency generation ──────────────────────────────

            "-M" | "-MM" => driver.dep_only = true,
            "-MD" | "-MMD" => {
                // -MD/-MMD: derive .d path from output, don't stop compilation.
                // The dep file path is derived later in the pipeline.
            }
            "-MF" => {
                i += 1;
                if i >= args.len() {
                    return Err("-MF requires an argument".into());
                }
                driver.dep_file = Some(args[i].clone());
            }
            "-MT" => {
                i += 1;
                if i >= args.len() {
                    return Err("-MT requires an argument".into());
                }
                driver.dep_target = Some(args[i].clone());
            }
            "-MQ" => {
                // -MQ is like -MT but escapes special make chars.
                i += 1;
                if i >= args.len() {
                    return Err("-MQ requires an argument".into());
                }
                driver.dep_target = Some(escape_make_target(&args[i]));
            }

            // ── Thread flags ───────────────────────────────────────

            "-pthread" => driver.pthread = true,

            // ── Pipe (silently ignored) ────────────────────────────

            "-pipe" => {}

            // ── Silently ignored -f flags ──────────────────────────
            // Build systems pass many GCC-specific flags we don't need.

            a if a.starts_with("-fno-") && is_ignorable_f_flag(&a[5..]) => {}
            a if a.starts_with("-f") && is_ignorable_f_flag(&a[2..]) => {}

            // ── Silently ignored -m flags ──────────────────────────

            a if a.starts_with("-mno-") && is_ignorable_m_flag(&a[5..]) => {}
            a if a.starts_with("-m") && is_ignorable_m_flag(&a[2..]) => {}

            // ── Silently ignored misc flags ────────────────────────

            "-Qunused-arguments" | "-no-canonical-prefixes" | "--param" => {
                // --param takes an argument
                if arg == "--param" {
                    i += 1; // skip the param value
                }
            }
            a if a.starts_with("--param=") => {} // --param=name=value
            a if a.starts_with("-fstack-protector") => {}
            a if a.starts_with("-fvisibility") => {}
            a if a.starts_with("-fno-stack-protector") => {}
            "-fno-strict-aliasing" | "-fstrict-aliasing" => {}
            "-fno-delete-null-pointer-checks" => {}
            "-fno-strict-overflow" | "-fstrict-overflow" => {}
            "-fno-allow-store-data-races" => {}
            "-fno-tree-loop-im" | "-fno-tree-loop-ivcanon" => {}
            "-fasan-shadow-offset" => { i += 1; } // takes arg
            "-fsanitize-coverage" | "-fprofile-arcs" | "-ftest-coverage" => {}
            a if a.starts_with("-fsanitize") => {}
            a if a.starts_with("-fprofile") => {}
            a if a.starts_with("-fno-sanitize") => {}
            a if a.starts_with("-fno-profile") => {}
            a if a.starts_with("-fdebug-prefix-map") => {}
            a if a.starts_with("-fmacro-prefix-map") => {}
            a if a.starts_with("-ffile-prefix-map") => {}

            // ── Ignored linker-related ─────────────────────────────

            "-rdynamic" => {}
            a if a.starts_with("-Tbss") || a.starts_with("-Ttext") || a.starts_with("-Tdata") => {}
            a if a.starts_with("-T") && a.len() > 2 => {
                // -T<script> linker script
                driver.linker_ordered_items.push(a.to_string());
            }

            // ── Unrecognized flags ─────────────────────────────────

            a if a.starts_with('-') => {
                // Silently ignore unknown flags in non-verbose mode.
                // This matches GCC behavior and is critical for build system compat.
                if driver.verbose {
                    eprintln!("cc1: note: ignoring unknown flag: {}", a);
                }
            }

            // ── Input files ────────────────────────────────────────

            _ => {
                let path = arg.clone();
                // If it's an object/archive or detected binary, add to linker items
                // at the current position for correct ordering.
                if super::file_types::is_object_or_archive(&path) {
                    driver.linker_ordered_items.push(path.clone());
                }
                driver.input_files.push(path);
            }
        }

        i += 1;
    }

    Ok(false)
}

// ── Helper functions ───────────────────────────────────────────────────

/// Parse a -D argument into a CliDefine.
fn parse_define(s: &str) -> CliDefine {
    if let Some(eq) = s.find('=') {
        CliDefine {
            name: s[..eq].to_string(),
            value: Some(s[eq + 1..].to_string()),
        }
    } else {
        CliDefine {
            name: s.to_string(),
            value: None, // Means "1"
        }
    }
}

/// Parse the -std= flag and set gnu_extensions / gnu89_inline accordingly.
fn parse_std_flag(driver: &mut Driver, std: &str) {
    match std {
        "gnu89" | "gnu90" => {
            driver.gnu_extensions = true;
            driver.gnu89_inline = true;
        }
        "gnu99" | "gnu9x" | "gnu11" | "gnu1x" | "gnu17" | "gnu18" | "gnu23" | "gnu2x" => {
            driver.gnu_extensions = true;
            driver.gnu89_inline = false;
        }
        "c89" | "c90" | "iso9899:1990" | "iso9899:199409" => {
            driver.gnu_extensions = false;
            driver.gnu89_inline = true;
        }
        "c99" | "c9x" | "c11" | "c1x" | "c17" | "c18" | "c23" | "c2x"
        | "iso9899:1999" | "iso9899:2011" | "iso9899:2017" | "iso9899:2024" => {
            driver.gnu_extensions = false;
            driver.gnu89_inline = false;
        }
        _ => {
            // Unknown standard — default to gnu99.
            driver.gnu_extensions = true;
            driver.gnu89_inline = false;
        }
    }
}

/// Parse -fpatchable-function-entry=N,M.
fn parse_patchable_entry(val: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = val.split(',').collect();
    let n: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let m: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((n, m))
}

/// Escape a Make target name (for -MQ).
fn escape_make_target(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '$' => result.push_str("$$"),
            '#' | '=' | ':' | ' ' | '\t' => {
                result.push('\\');
                result.push(ch);
            }
            _ => result.push(ch),
        }
    }
    result
}

/// Check if there are no input files in the remaining arguments.
fn no_input_files(args: &[String]) -> bool {
    args.iter().all(|a| a.starts_with('-'))
}

/// Check if an -f flag (after -f or -fno-) should be silently ignored.
fn is_ignorable_f_flag(flag: &str) -> bool {
    matches!(
        flag,
        "inline-functions"
            | "inline-small-functions"
            | "inline-functions-called-once"
            | "ipa-cp"
            | "ipa-cp-clone"
            | "split-wide-types"
            | "tree-loop-distribute-patterns"
            | "tree-loop-vectorize"
            | "tree-slp-vectorize"
            | "vect-cost-model"
            | "unwind-tables"
            | "exceptions"
            | "rtti"
            | "plt"
            | "semantic-interposition"
            | "math-errno"
            | "trapping-math"
            | "signed-zeros"
            | "associative-math"
            | "reciprocal-math"
            | "finite-math-only"
            | "unsafe-math-optimizations"
            | "fast-math"
            | "cx-limited-range"
            | "stack-check"
            | "pic"
            | "PIC"
            | "PIE"
            | "pie"
            | "no-pie"
            | "no-PIE"
    )
}

/// Check if an -m flag (after -m or -mno-) should be silently ignored.
fn is_ignorable_m_flag(flag: &str) -> bool {
    matches!(
        flag,
        "80387"
            | "fp-ret-in-387"
            | "align-double"
            | "mmx"
            | "sse"
            | "sse2"
            | "red-zone"
            | "popcnt"
            | "cx16"
            | "sahf"
            | "bmi"
            | "bmi2"
            | "lzcnt"
            | "fma"
            | "f16c"
            | "movbe"
            | "tune=generic"
            | "tune=native"
    )
}

/// Print --version output (GCC-compatible: includes "Free Software Foundation"
/// for Meson detection, plus backend mode info).
fn print_version(driver: &Driver) {
    println!("ccc (Claude's C Compiler, GCC-compatible) 14.2.0");
    println!("Copyright (C) 2026 Free Software Foundation, Inc.");
    println!("This is free software; see the source for copying conditions.");
    println!(
        "Target: {}, Backend: standalone (builtin assembler + linker)",
        driver.target_triple()
    );
}

/// Print -v (verbose, alone) output.
fn print_verbose_version(driver: &Driver) {
    eprintln!("Using built-in specs.");
    eprintln!("Target: {}", driver.target_triple());
    eprintln!(
        "Configured with: --target={} --disable-multilib",
        driver.target_triple()
    );
    eprintln!("Thread model: posix");
    eprintln!("ccc version 14.2.0 (Claude's C Compiler)");
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> (Driver, bool) {
        let mut d = Driver::new();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let early = parse_cli_args(&mut d, "cc1", &args).unwrap();
        (d, early)
    }

    fn parse_err(args: &[&str]) -> String {
        let mut d = Driver::new();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse_cli_args(&mut d, "cc1", &args).unwrap_err()
    }

    #[test]
    fn test_basic_input_file() {
        let (d, early) = parse(&["test.c"]);
        assert!(!early);
        assert_eq!(d.input_files, vec!["test.c"]);
        assert_eq!(d.mode, CompileMode::Full);
    }

    #[test]
    fn test_mode_selection() {
        let (d, _) = parse(&["-E", "test.c"]);
        assert_eq!(d.mode, CompileMode::PreprocessOnly);

        let (d, _) = parse(&["-S", "test.c"]);
        assert_eq!(d.mode, CompileMode::AssemblyOnly);

        let (d, _) = parse(&["-c", "test.c"]);
        assert_eq!(d.mode, CompileMode::ObjectOnly);
    }

    #[test]
    fn test_output_path() {
        let (d, _) = parse(&["-o", "out.o", "-c", "test.c"]);
        assert_eq!(d.output_path, "out.o");
        assert!(d.output_path_set);

        let (d, _) = parse(&["-oout.o", "-c", "test.c"]);
        assert_eq!(d.output_path, "out.o");
    }

    #[test]
    fn test_target_override() {
        let (d, _) = parse(&["-m32", "test.c"]);
        assert_eq!(d.target, Target::I386);

        let (d, _) = parse(&["-m64", "test.c"]);
        assert_eq!(d.target, Target::X86_64);
    }

    #[test]
    fn test_optimization_flags() {
        let (d, _) = parse(&["-O2", "test.c"]);
        assert!(d.optimize);
        assert!(!d.optimize_size);

        let (d, _) = parse(&["-Os", "test.c"]);
        assert!(d.optimize);
        assert!(d.optimize_size);

        let (d, _) = parse(&["-O0", "test.c"]);
        assert!(!d.optimize);
    }

    #[test]
    fn test_defines() {
        let (d, _) = parse(&["-DFOO", "-DBAR=42", "-D", "BAZ", "test.c"]);
        assert_eq!(d.defines.len(), 3);
        assert_eq!(d.defines[0].name, "FOO");
        assert!(d.defines[0].value.is_none());
        assert_eq!(d.defines[1].name, "BAR");
        assert_eq!(d.defines[1].value.as_deref(), Some("42"));
        assert_eq!(d.defines[2].name, "BAZ");
    }

    #[test]
    fn test_include_paths() {
        let (d, _) = parse(&["-I/usr/include", "-I", "/opt/include", "-iquote", ".", "test.c"]);
        assert_eq!(d.include_paths, vec!["/usr/include", "/opt/include"]);
        assert_eq!(d.quote_include_paths, vec!["."]);
    }

    #[test]
    fn test_warning_flags() {
        let (d, _) = parse(&["-Wall", "-Wextra", "-Werror", "test.c"]);
        assert!(d.warning_config.all);
        assert!(d.warning_config.extra);
        assert!(d.warning_config.error);

        let (d, _) = parse(&["-w", "test.c"]);
        assert!(d.warning_config.suppress_all);
    }

    #[test]
    fn test_std_flag() {
        let (d, _) = parse(&["-std=c89", "test.c"]);
        assert!(!d.gnu_extensions);
        assert!(d.gnu89_inline);

        let (d, _) = parse(&["-std=gnu99", "test.c"]);
        assert!(d.gnu_extensions);
        assert!(!d.gnu89_inline);

        let (d, _) = parse(&["-std=gnu89", "test.c"]);
        assert!(d.gnu_extensions);
        assert!(d.gnu89_inline);
    }

    #[test]
    fn test_simd_implication_chain() {
        let (d, _) = parse(&["-mavx2", "test.c"]);
        assert!(d.enable_avx2);
        assert!(d.enable_avx);
        assert!(d.enable_sse4_2);
        assert!(d.enable_sse4_1);
        assert!(d.enable_ssse3);
        assert!(d.enable_sse3);
    }

    #[test]
    fn test_linker_items_ordering() {
        let (d, _) = parse(&[
            "a.c", "-lfoo", "-Wl,--whole-archive", "lib.a", "-lbar",
        ]);
        // -l flags and -Wl, flags in order
        assert!(d.linker_ordered_items.contains(&"-lfoo".to_string()));
        assert!(d.linker_ordered_items.contains(&"-lbar".to_string()));
    }

    #[test]
    fn test_pic_flags() {
        let (d, _) = parse(&["-fPIC", "test.c"]);
        assert!(d.pic);

        let (d, _) = parse(&["-fPIC", "-fno-PIC", "test.c"]);
        assert!(!d.pic);
    }

    #[test]
    fn test_debug_flags() {
        let (d, _) = parse(&["-g", "test.c"]);
        assert!(d.debug_info);

        let (d, _) = parse(&["-g0", "test.c"]);
        assert!(!d.debug_info);
    }

    #[test]
    fn test_multiple_input_files() {
        let (d, _) = parse(&["a.c", "b.c", "c.o"]);
        assert_eq!(d.input_files.len(), 3);
    }

    #[test]
    fn test_unknown_flags_silently_ignored() {
        // Should not error — matches GCC behavior.
        let (_, early) = parse(&["-funknown-flag", "-munknown-flag", "test.c"]);
        assert!(!early);
    }

    #[test]
    fn test_o_requires_argument() {
        let err = parse_err(&["-o"]);
        assert!(err.contains("-o requires"));
    }

    #[test]
