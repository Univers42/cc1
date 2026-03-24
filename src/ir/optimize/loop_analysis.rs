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
fn compute_dominators(n: usize, preds: &[Vec<BlockId>], _succs: &[Vec<BlockId>]) -> Vec<BlockId> {
    // RPO numbering
    let rpo = compute_rpo(n, _succs);
    let mut rpo_num = vec![usize::MAX; n];
    for (i, &b) in rpo.iter().enumerate() {
        rpo_num[b] = i;
    }

    let mut idom = vec![BlockId(u32::MAX); n];
    idom[0] = BlockId(0); // Entry dominates itself.

    let mut changed = true;
    while changed {
        changed = false;
        for &b in &rpo[1..] {
            // Find first processed predecessor
            let mut new_idom: Option<usize> = None;
            for p in &preds[b] {
                let pi = p.0 as usize;
                if idom[pi].0 != u32::MAX {
                    new_idom = Some(match new_idom {
                        None => pi,
                        Some(current) => intersect(current, pi, &idom, &rpo_num),
                    });
                }
            }
            if let Some(ni) = new_idom {
                let new_bid = BlockId(ni as u32);
                if idom[b] != new_bid {
                    idom[b] = new_bid;
                    changed = true;
                }
            }
        }
    }

    idom
}

fn intersect(
    mut b1: usize,
    mut b2: usize,
    idom: &[BlockId],
    rpo_num: &[usize],
) -> usize {
    while b1 != b2 {
        while rpo_num[b1] > rpo_num[b2] {
            b1 = idom[b1].0 as usize;
        }
        while rpo_num[b2] > rpo_num[b1] {
            b2 = idom[b2].0 as usize;
        }
    }
    b1
}

/// Compute reverse postorder.
fn compute_rpo(n: usize, succs: &[Vec<BlockId>]) -> Vec<usize> {
    let mut visited = vec![false; n];
    let mut postorder = Vec::with_capacity(n);

    fn dfs(
        b: usize,
        succs: &[Vec<BlockId>],
        visited: &mut [bool],
        postorder: &mut Vec<usize>,
    ) {
        visited[b] = true;
        for s in &succs[b] {
            let si = s.0 as usize;
            if si < visited.len() && !visited[si] {
                dfs(si, succs, visited, postorder);
            }
        }
        postorder.push(b);
    }

    dfs(0, succs, &mut visited, &mut postorder);
    postorder.reverse();
    postorder
}

/// Detect natural loops.
fn detect_loops(
    n: usize,
    preds: &[Vec<BlockId>],
    succs: &[Vec<BlockId>],
    idom: &[BlockId],
) -> Vec<NaturalLoop> {
    let mut loops_map: HashMap<BlockId, NaturalLoop> = HashMap::new();

    // Find back edges: tail -> header where header dominates tail.
    for tail in 0..n {
        for succ in &succs[tail] {
            let header = succ.0 as usize;
            if header < n && dominates(header, tail, idom) {
                let header_bid = BlockId(header as u32);
                let tail_bid = BlockId(tail as u32);

                let entry = loops_map.entry(header_bid).or_insert_with(|| NaturalLoop {
                    header: header_bid,
                    body: HashSet::new(),
                    preheader: None,
                    latches: Vec::new(),
                });
                entry.latches.push(tail_bid);
            }
        }
    }

    // Compute loop bodies and preheaders.
    for (_, nl) in loops_map.iter_mut() {
        // Compute body via reverse BFS from latches.
        nl.body.insert(nl.header);
        let mut worklist: VecDeque<BlockId> = VecDeque::new();
        for latch in &nl.latches {
            if !nl.body.contains(latch) {
                nl.body.insert(*latch);
                worklist.push_back(*latch);
            }
        }
        while let Some(b) = worklist.pop_front() {
            let bi = b.0 as usize;
            if bi < preds.len() {
                for p in &preds[bi] {
                    if !nl.body.contains(p) {
                        nl.body.insert(*p);
                        worklist.push_back(*p);
                    }
                }
            }
        }

        // Find preheader: single predecessor of header outside the loop.
        let header_idx = nl.header.0 as usize;
        if header_idx < preds.len() {
            let outside_preds: Vec<BlockId> = preds[header_idx]
                .iter()
                .filter(|p| !nl.body.contains(p))
                .copied()
                .collect();
            if outside_preds.len() == 1 {
                nl.preheader = Some(outside_preds[0]);
            }
        }
    }

    // Collect and sort innermost-first (smaller body first).
    let mut loops: Vec<NaturalLoop> = loops_map.into_values().collect();
    loops.sort_by_key(|l| l.body.len());
    loops
}

/// Check if `a` dominates `b` by walking up the dominator tree.
fn dominates(a: usize, b: usize, idom: &[BlockId]) -> bool {
    let mut cur = b;
    loop {
        if cur == a {
            return true;
        }
        let parent = idom[cur].0 as usize;
        if parent == cur {
            break; // Reached entry
        }
        cur = parent;
    }
    cur == a // Final check at entry
}

