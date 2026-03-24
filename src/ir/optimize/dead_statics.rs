// ir/optimize/dead_statics.rs — Dead static elimination.
//
// Removes internal-linkage functions and globals that are not reachable
// from any external root. Uses BFS reachability from external roots.

use crate::ir::instruction::Instruction;
use crate::ir::module::IrModule;
use crate::ir::types::Linkage;
use std::collections::HashSet;

/// Run dead static elimination on the module. Returns true if anything was removed.
pub fn dead_statics(module: &mut IrModule) -> bool {
    let mut changed = false;

    // Collect all external (non-eliminable) roots.
    let mut roots: HashSet<String> = HashSet::new();

    for func in &module.functions {
        if !is_internal_linkage(func.linkage) {
            roots.insert(func.name.clone());
        }
    }
    for glob in &module.globals {
        if !is_internal_linkage(glob.linkage) {
            roots.insert(glob.name.clone());
        }
    }
    // All externs are roots.
    for ext in &module.externs {
        roots.insert(ext.name.clone());
    }

    // BFS from roots to find all reachable symbols.
    let mut reachable: HashSet<String> = HashSet::new();
    let mut worklist: Vec<String> = roots.into_iter().collect();

    while let Some(name) = worklist.pop() {
        if !reachable.insert(name.clone()) {
            continue;
        }

        // Find references from this symbol.
        let refs = collect_references(module, &name);
        for r in refs {
            if !reachable.contains(&r) {
                worklist.push(r);
            }
        }
    }

    // Remove unreachable internal functions.
    let before_funcs = module.functions.len();
    module.functions.retain(|f| {
        if is_internal_linkage(f.linkage) && !reachable.contains(&f.name) {
            true // We'll mark for removal below
        } else {
            true
        }
    });

    // Actually remove: we need to filter
    let mut to_remove_funcs: Vec<String> = Vec::new();
    for f in &module.functions {
        if is_internal_linkage(f.linkage) && !reachable.contains(&f.name) {
            to_remove_funcs.push(f.name.clone());
        }
    }
    if !to_remove_funcs.is_empty() {
        let remove_set: HashSet<&str> = to_remove_funcs.iter().map(|s| s.as_str()).collect();
        module.functions.retain(|f| !remove_set.contains(f.name.as_str()));
        changed = true;
    }

    // Remove unreachable internal globals.
    let mut to_remove_globals: Vec<String> = Vec::new();
    for g in &module.globals {
        if is_internal_linkage(g.linkage) && !reachable.contains(&g.name) {
            to_remove_globals.push(g.name.clone());
        }
    }
    if !to_remove_globals.is_empty() {
        let remove_set: HashSet<&str> = to_remove_globals.iter().map(|s| s.as_str()).collect();
        module.globals.retain(|g| !remove_set.contains(g.name.as_str()));
        changed = true;
    }

    changed
}

fn is_internal_linkage(linkage: Linkage) -> bool {
    matches!(linkage, Linkage::Internal | Linkage::Private)
}

/// Collect all symbol names referenced by a given symbol.
fn collect_references(module: &IrModule, name: &str) -> Vec<String> {
    let mut refs = Vec::new();

    // Check functions.
    for func in &module.functions {
        if func.name != name {
            continue;
        }
        for block in &func.blocks {
            for inst in &block.insts {
                collect_inst_refs(inst, &mut refs);
            }
        }
    }

    // Check globals for initializer references.
    for glob in &module.globals {
        if glob.name != name {
            continue;
        }
        if let Some(ref init) = glob.init {
            collect_global_init_refs(init, &mut refs);
        }
    }

    refs
}

/// Collect symbol references from an instruction.
fn collect_inst_refs(inst: &Instruction, refs: &mut Vec<String>) {
    match inst {
        Instruction::Call { callee, .. } => {
            refs.push(callee.clone());
        }
        Instruction::GlobalAddr { name, .. } => {
            refs.push(name.clone());
        }
        _ => {}
    }
    // Also check operands for Global references.
    inst.for_each_operand(|op| {
        if let crate::ir::types::Operand::Global(g) = op {
            refs.push(g.clone());
        }
    });
}

/// Collect symbol references from a global initializer.
fn collect_global_init_refs(init: &crate::ir::module::GlobalInit, refs: &mut Vec<String>) {
    match init {
        crate::ir::module::GlobalInit::Address { symbol, .. } => {
            refs.push(symbol.clone());
        }
        crate::ir::module::GlobalInit::Compound(fields) => {
            for (_offset, f) in fields {
                collect_global_init_refs(f, refs);
            }
        }
        _ => {}
    }
}

