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
