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
