// cc1 library root — re-exports for testing and integration.

pub mod target;
pub mod diagnostics;
pub mod source;
pub mod ctx;
pub mod opts;
pub mod frontend;
pub mod backend;
pub mod ir;
pub mod driver;

use std::thread;

/// Shared entry point called by each binary's main().
///
/// Spawns a worker thread with a 64 MB stack (the default ~8 MB is insufficient
/// for deeply recursive descent parsing of large generated C files like Bison
/// parsers). Catches panics and reports them as "internal error" messages.
pub fn compiler_main() -> i32 {
    let builder = thread::Builder::new()
        .name("cc1-worker".into())
        .stack_size(64 * 1024 * 1024); // 64 MB

    let handle = builder
        .spawn(|| compiler_main_inner())
        .expect("failed to spawn worker thread");

    match handle.join() {
        Ok(code) => code,
        Err(panic_info) => {
            let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".into()
            };
            eprintln!("cc1: internal error: {}", msg);
            eprintln!("Please report this bug.");
            1
        }
    }
}

fn compiler_main_inner() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let argv0 = args.first().cloned().unwrap_or_else(|| "cc1".into());
    let cli_args: Vec<String> = args.into_iter().skip(1).collect();

    let mut d = driver::Driver::new();

    match driver::cli::parse_cli_args(&mut d, &argv0, &cli_args) {
        Ok(true) => return 0,  // Query flag handled (e.g. --version)
        Ok(false) => {}        // Normal — proceed to compilation
        Err(e) => {
            eprintln!("cc1: error: {}", e);
            return 1;
        }
    }

    // Check for input files.
    if !d.has_input_files() {
        eprintln!("cc1: error: no input files");
        return 1;
    }

    // Run the compilation pipeline.
    match d.run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("cc1: error: {}", e);
            1
        }
    }
}
