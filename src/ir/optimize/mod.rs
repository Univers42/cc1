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
pub fn optimize(module: &mut IrModule, target: Target) {
    let disabled = disabled_passes();
    if disabled.contains("all") {
        return;
    }
    let timing = time_passes();
    let total_start = std::time::Instant::now();

    let nfuncs = module.functions.len();

    // ── Phase 0: Inlining ──────────────────────────────────────────
    if !disabled.contains("inline") {
        inline::run_inline(module, timing);
    }

    // Post-inline cleanup (single pass on all functions)
    {
        let mut any_dirty = vec![true; nfuncs];
        let mut any_changed = vec![false; nfuncs];

        // mem2reg — currently a no-op stub
        // TODO: full mem2reg implementation
        run_on_dirty(module, &any_dirty, &mut any_changed, "mem2reg", &disabled, timing, |_f| false);

        // constant_fold
        run_on_dirty(module, &any_dirty, &mut any_changed, "constfold", &disabled, timing, |f| {
            constant_fold::constant_fold(f)
        });

        // copy_prop
        run_on_dirty(module, &any_dirty, &mut any_changed, "copyprop", &disabled, timing, |f| {
            copy_prop::copy_prop(f)
        });

        // simplify
        run_on_dirty(module, &any_dirty, &mut any_changed, "simplify", &disabled, timing, |f| {
            simplify::simplify(f)
        });

        // constant_fold (again)
        run_on_dirty(module, &any_dirty, &mut any_changed, "constfold", &disabled, timing, |f| {
            constant_fold::constant_fold(f)
        });

        // copy_prop (again)
        run_on_dirty(module, &any_dirty, &mut any_changed, "copyprop", &disabled, timing, |f| {
            copy_prop::copy_prop(f)
        });

        // resolve_asm
        run_on_dirty(module, &any_dirty, &mut any_changed, "resolveasm", &disabled, timing, |f| {
            resolve_asm::resolve_asm(f)
        });
    }

    // ── Phase 0.5: IsConstant Resolution ───────────────────────────
    // Any remaining IsConstant instructions → 0 (false).
    // (Currently not applicable since we have no IsConstant IR instruction.)

    // ── Main Loop ──────────────────────────────────────────────────
    let mut dirty = vec![true; nfuncs];
    let mut changed = vec![false; nfuncs];
    let mut prev = PassChanges::new_first_iter();
    let mut first_iter_baseline = 0usize;

    let is_64bit = matches!(target, Target::X86_64);

    for iteration in 0..MAX_ITERATIONS {
        let mut cur = PassChanges::default();

        // 1. cfg_simplify (first)
        if should_run!(prev, cfg_simplify1, constant_fold, dce) {
            cur.cfg_simplify1 = run_on_dirty(
                module, &dirty, &mut changed, "cfg", &disabled, timing,
                |f| cfg_simplify::cfg_simplify(f),
            );
        }

        // 2. copy_prop (first)
        if should_run!(prev, copy_prop1, cfg_simplify1, gvn, licm, if_convert) {
            cur.copy_prop1 = run_on_dirty(
                module, &dirty, &mut changed, "copyprop", &disabled, timing,
                |f| copy_prop::copy_prop(f),
            );
        }

        // 2a. div_by_const (iteration 0 only, 64-bit targets only)
        if iteration == 0 && is_64bit {
            if should_run!(prev, div_by_const) {
                cur.div_by_const = run_on_dirty(
                    module, &dirty, &mut changed, "divconst", &disabled, timing,
                    |f| div_by_const::div_by_const(f, is_64bit),
                );
            }
        }

        // 2b. narrow
        if should_run!(prev, narrow, copy_prop1) {
            cur.narrow = run_on_dirty(
                module, &dirty, &mut changed, "narrow", &disabled, timing,
                |f| narrow::narrow(f),
            );
        }

        // 3. simplify
        if should_run!(prev, simplify, copy_prop1, narrow) {
            cur.simplify = run_on_dirty(
                module, &dirty, &mut changed, "simplify", &disabled, timing,
                |f| simplify::simplify(f),
            );
        }

        // 4. constant_fold
        if should_run!(prev, constant_fold, copy_prop1, narrow, simplify, if_convert, copy_prop2) {
            cur.constant_fold = run_on_dirty(
                module, &dirty, &mut changed, "constfold", &disabled, timing,
                |f| constant_fold::constant_fold(f),
            );
        }

        // 5-6a. gvn + licm + ivsr (shared CFG analysis)
        {
            let run_gvn = should_run!(prev, gvn, cfg_simplify1, copy_prop1, simplify);
            let run_licm = should_run!(prev, licm, cfg_simplify1, copy_prop1, gvn);
            let run_ivsr = iteration == 0;

            if run_gvn || run_licm || run_ivsr {
                let start = std::time::Instant::now();
                for i in 0..module.functions.len() {
                    if module.functions[i].blocks.is_empty() || !dirty[i] {
                        continue;
                    }
                    let (g, l, v) = gvn_licm_ivsr_shared(
                        &mut module.functions[i],
                        run_gvn && !disabled.contains("gvn"),
                        run_licm && !disabled.contains("licm"),
                        run_ivsr && !disabled.contains("ivsr"),
                    );
                    if g { cur.gvn += 1; changed[i] = true; }
                    if l { cur.licm += 1; changed[i] = true; }
                    if v { cur.ivsr += 1; changed[i] = true; }
                }
                if timing {
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    if cur.gvn + cur.licm + cur.ivsr > 0 {
                        eprintln!(
                            "[OPT] gvn+licm+ivsr: {:.3}ms (gvn={}, licm={}, ivsr={})",
                            elapsed, cur.gvn, cur.licm, cur.ivsr
                        );
                    }
                }
            }
        }

        // 7. if_convert
        if should_run!(prev, if_convert, cfg_simplify1, constant_fold) {
            cur.if_convert = run_on_dirty(
                module, &dirty, &mut changed, "ifconv", &disabled, timing,
                |f| if_convert::if_convert(f),
            );
        }

        // 8. copy_prop (second round)
        if should_run!(prev, copy_prop2, gvn, licm, if_convert)
            || cur.simplify > 0
            || cur.constant_fold > 0
        {
            cur.copy_prop2 = run_on_dirty(
                module, &dirty, &mut changed, "copyprop", &disabled, timing,
                |f| copy_prop::copy_prop(f),
            );
        }

        // 9. dce
        if should_run!(prev, dce, gvn, licm, if_convert, copy_prop2) {
            cur.dce = run_on_dirty(
                module, &dirty, &mut changed, "dce", &disabled, timing,
                |f| dce::dce(f),
            );
        }

        // 10. cfg_simplify (second round)
        if should_run!(prev, cfg_simplify2, constant_fold, if_convert, dce) {
            cur.cfg_simplify2 = run_on_dirty(
                module, &dirty, &mut changed, "cfg", &disabled, timing,
                |f| cfg_simplify::cfg_simplify(f),
            );
        }

        // 10.5. ipcp (interprocedural)
        if !disabled.contains("ipcp") {
            if ipcp::ipcp(module) {
                cur.ipcp = 1;
                // Mark all functions dirty for next iteration
                for c in changed.iter_mut() {
                    *c = true;
                }
            }
        }

        // ── End of iteration bookkeeping ───────────────────────────
        let iter_total = cur.total_excluding_dce();

        if iteration == 0 {
            first_iter_baseline = iter_total;
        }

        // Swap dirty/changed
        std::mem::swap(&mut dirty, &mut changed);
        changed.iter_mut().for_each(|c| *c = false);

        // Check for fixed point
        if iter_total == 0 && cur.dce == 0 && cur.ipcp == 0 {
            if timing {
                eprintln!("[OPT] Fixed point reached after {} iteration(s)", iteration + 1);
            }
            break;
        }

        // Diminishing returns check (after at least 2 iterations)
        if iteration >= 1 && first_iter_baseline > 0 {
            let ratio = iter_total as f64 / first_iter_baseline as f64;
            if ratio < DIMINISHING_RETURNS_RATIO && cur.ipcp == 0 {
                if timing {
                    eprintln!(
                        "[OPT] Diminishing returns ({:.1}%) after {} iteration(s)",
                        ratio * 100.0,
                        iteration + 1
                    );
                }
                break;
            }
        }

        prev = cur;
    }

    // ── Phase 11: Dead Static Elimination ──────────────────────────
    if !disabled.contains("deadstatics") {
        dead_statics::dead_statics(module);
    }

    if timing {
        eprintln!(
            "[OPT] Total optimization: {:.3}ms",
            total_start.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Run GVN, LICM, and IVSR with shared CFG analysis on a single function.
fn gvn_licm_ivsr_shared(
    func: &mut crate::ir::module::IrFunction,
    run_gvn: bool,
    run_licm: bool,
    run_ivsr: bool,
) -> (bool, bool, bool) {
    let mut gvn_changed = false;
    let mut licm_changed = false;
    let mut ivsr_changed = false;

    // For single-block functions, only GVN runs (fast path).
    if func.blocks.len() <= 1 {
        if run_gvn {
            gvn_changed = gvn::gvn(func);
        }
        return (gvn_changed, false, false);
    }

    // Build shared CFG analysis
    func.compute_predecessors();
    let cfg = loop_analysis::CfgAnalysis::build(func);

    if run_gvn {
        gvn_changed = gvn::gvn(func);
    }

    if run_licm {
        licm_changed = licm::licm(func, &cfg);
    }

    if run_ivsr && !cfg.loops.is_empty() {
        ivsr_changed = iv_strength_reduce::iv_strength_reduce(func, &cfg);
    }

    (gvn_changed, licm_changed, ivsr_changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::module::IrModule;

    #[test]
    fn test_optimize_empty_module() {
        let mut module = IrModule::new("test");
        optimize(&mut module, Target::X86_64);
        assert!(module.functions.is_empty());
    }

    #[test]
    fn test_disabled_passes() {
        // Clearing the env var for safety
        std::env::remove_var("CCC_DISABLE_PASSES");
        let d = disabled_passes();
        assert!(d.is_empty());
    }

    #[test]
    fn test_pass_changes_first_iter() {
        let p = PassChanges::new_first_iter();
        // All fields should be usize::MAX
        assert_eq!(p.cfg_simplify1, usize::MAX);
        assert_eq!(p.gvn, usize::MAX);
    }
}
