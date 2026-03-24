// ir/optimize/mod.rs — Optimization pipeline orchestration with dirty tracking.
//
// All optimization levels run the same full set of passes. The pipeline
// uses per-function dirty tracking and per-pass skip logic to avoid
// redundant work, and terminates early on diminishing returns.

pub mod cfg_simplify;
pub mod constant_fold;
pub mod copy_prop;
pub mod dce;
pub mod dead_statics;
pub mod div_by_const;
pub mod gvn;
pub mod if_convert;
pub mod inline;
pub mod ipcp;
pub mod iv_strength_reduce;
pub mod licm;
pub mod loop_analysis;
pub mod narrow;
pub mod resolve_asm;
pub mod simplify;

use crate::ir::module::IrModule;
use crate::target::Target;
use std::collections::HashSet;

/// Maximum number of main-loop iterations.
const MAX_ITERATIONS: usize = 3;

/// Diminishing returns threshold: stop if changes < 5% of first iteration.
const DIMINISHING_RETURNS_RATIO: f64 = 0.05;

/// Set of passes that can be individually disabled via CCC_DISABLE_PASSES.
fn disabled_passes() -> HashSet<String> {
    match std::env::var("CCC_DISABLE_PASSES") {
        Ok(val) => val.split(',').map(|s| s.trim().to_lowercase()).collect(),
        Err(_) => HashSet::new(),
    }
}

fn time_passes() -> bool {
    std::env::var("CCC_TIME_PASSES").is_ok()
}

/// Per-pass change counts from the previous iteration, used for skip logic.
