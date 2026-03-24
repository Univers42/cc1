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
#[derive(Default)]
struct PassChanges {
    cfg_simplify1: usize,
    copy_prop1: usize,
    div_by_const: usize,
    narrow: usize,
    simplify: usize,
    constant_fold: usize,
    gvn: usize,
    licm: usize,
    ivsr: usize,
    if_convert: usize,
    copy_prop2: usize,
    dce: usize,
    cfg_simplify2: usize,
    ipcp: usize,
}

impl PassChanges {
    fn new_first_iter() -> Self {
        // First iteration: all counts set to max so every pass runs.
        Self {
            cfg_simplify1: usize::MAX,
            copy_prop1: usize::MAX,
            div_by_const: usize::MAX,
            narrow: usize::MAX,
            simplify: usize::MAX,
            constant_fold: usize::MAX,
            gvn: usize::MAX,
            licm: usize::MAX,
            ivsr: usize::MAX,
            if_convert: usize::MAX,
            copy_prop2: usize::MAX,
            dce: usize::MAX,
            cfg_simplify2: usize::MAX,
            ipcp: usize::MAX,
        }
    }

    /// Total changes excluding DCE (for diminishing returns check).
    fn total_excluding_dce(&self) -> usize {
        let vals = [
            self.cfg_simplify1,
            self.copy_prop1,
            self.div_by_const,
            self.narrow,
            self.simplify,
            self.constant_fold,
            self.gvn,
            self.licm,
            self.ivsr,
            self.if_convert,
            self.copy_prop2,
            self.cfg_simplify2,
            self.ipcp,
        ];
        vals.iter()
            .filter(|&&v| v != usize::MAX)
            .sum()
    }
}

/// Helper: run a pass on all dirty functions, tracking changes.
fn run_on_dirty(
    module: &mut IrModule,
    dirty: &[bool],
    changed: &mut Vec<bool>,
    pass_name: &str,
    disabled: &HashSet<String>,
    timing: bool,
    mut pass_fn: impl FnMut(&mut crate::ir::module::IrFunction) -> bool,
) -> usize {
    if disabled.contains(pass_name) || disabled.contains("all") {
        return 0;
    }
    let start = std::time::Instant::now();
    let mut total_changes = 0usize;
    for i in 0..module.functions.len() {
        if module.functions[i].blocks.is_empty() {
            continue; // declaration only
        }
        if !dirty[i] {
            continue;
        }
        if pass_fn(&mut module.functions[i]) {
            changed[i] = true;
            total_changes += 1;
        }
    }
    if timing && total_changes > 0 {
        eprintln!(
            "[OPT] {}: {:.3}ms ({} functions changed)",
            pass_name,
            start.elapsed().as_secs_f64() * 1000.0,
            total_changes
        );
    }
    total_changes
}

/// Macro-like check: should this pass run based on previous iteration's changes?
macro_rules! should_run {
    ($prev:expr, $self_field:ident $(, $dep:ident)*) => {
        $prev.$self_field > 0 $(|| $prev.$dep > 0)*
    };
}

/// Run the full optimization pipeline on an IR module.
