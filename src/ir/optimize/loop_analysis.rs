// ir/optimize/loop_analysis.rs — Shared loop detection and body computation.
//
// Provides natural loop detection, loop body computation, and preheader
// detection used by both LICM and IVSR.

use crate::ir::module::IrFunction;
use crate::ir::types::BlockId;
use std::collections::{HashMap, HashSet, VecDeque};

/// A natural loop in the CFG.
#[derive(Debug, Clone)]
pub struct NaturalLoop {
    /// The loop header block.
    pub header: BlockId,
    /// All blocks in the loop body (including the header).
    pub body: HashSet<BlockId>,
    /// The single preheader block (predecessor of header outside the loop), if any.
    pub preheader: Option<BlockId>,
    /// Back-edge tail blocks.
    pub latches: Vec<BlockId>,
}

/// Shared CFG analysis computed once and passed to GVN, LICM, and IVSR.
#[derive(Debug)]
pub struct CfgAnalysis {
    /// Detected natural loops, sorted innermost-first (smallest body first).
    pub loops: Vec<NaturalLoop>,
    /// Dominator tree: immediate dominator for each block. Entry has itself.
    pub idom: Vec<BlockId>,
    /// Predecessor map: block_id -> [predecessor block_ids].
    pub preds: Vec<Vec<BlockId>>,
    /// Successor map: block_id -> [successor block_ids].
    pub succs: Vec<Vec<BlockId>>,
}

impl CfgAnalysis {
    /// Build the full CFG analysis for a function.
    pub fn build(func: &IrFunction) -> Self {
        let n = func.blocks.len();
        if n == 0 {
            return CfgAnalysis {
                loops: Vec::new(),
                idom: Vec::new(),
                preds: Vec::new(),
                succs: Vec::new(),
            };
        }

        // Build adjacency
        let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
        let mut succs: Vec<Vec<BlockId>> = vec![Vec::new(); n];

        for (i, block) in func.blocks.iter().enumerate() {
            let s = block.terminator.successors();
            for succ in &s {
                let si = succ.0 as usize;
                if si < n {
                    succs[i].push(*succ);
                    preds[si].push(BlockId(i as u32));
                }
            }
        }

        // Compute dominators using Cooper-Harvey-Kennedy algorithm
        let idom = compute_dominators(n, &preds, &succs);

        // Detect natural loops
        let loops = detect_loops(n, &preds, &succs, &idom);

        CfgAnalysis {
            loops,
            idom,
            preds,
            succs,
        }
    }
}

/// Compute immediate dominators using simple iterative algorithm.
