use super::*;

// ============================================================================
// Transient resource aliasing  (Phase B)
// ============================================================================

/// A single slot in the aliasing plan — one or more resources that share
/// the same physical memory because their lifetimes do not overlap.
#[derive(Clone, Debug)]
pub struct AliasSlot {
    /// Resources aliased into this slot.
    pub resources: Vec<String>,
    /// Index of this slot in the pool.
    pub slot_index: usize,
}

/// Plan produced by [`TransientResourcePool::build`] describing how to
/// alias transient resources onto a fixed number of memory slots.
#[derive(Clone, Debug, Default)]
pub struct AliasingPlan {
    /// Ordered list of alias slots.
    pub slots: Vec<AliasSlot>,
    /// Mapping from resource name to its assigned slot index.
    pub resource_to_slot: HashMap<String, usize>,
}

/// A pool that analyses resource lifetimes across the sorted pass order
/// and assigns non-overlapping resources to the same memory slot.
///
/// # Algorithm
///
/// 1. For each resource, compute the interval `[first_pass, last_pass]`
///    over the sorted pass order.
/// 2. Sort resources by their first-use pass.
/// 3. Greedy interval packing: assign each resource to the first slot
///    whose current occupant's interval does not overlap.
///
/// Resources whose names appear in `exempt` (e.g. `"swapchain"`) are
/// excluded from aliasing because their memory is owned by the swapchain.
#[derive(Clone, Debug)]
pub struct TransientResourcePool {
    /// Resource names that must not be aliased.
    exempt: Vec<String>,
}

impl TransientResourcePool {
    /// Create a new pool with the given exempt resources.
    pub fn new(exempt: Vec<String>) -> Self {
        Self { exempt }
    }

    /// Build an aliasing plan from the render graph's pass declarations.
    ///
    /// `pass_order` is the sorted execution order (e.g. from
    /// [`compile`](RenderGraph::compile)).
    pub fn build(&self, graph: &RenderGraph, pass_order: &[usize]) -> AliasingPlan {
        // ── Step 1: collect lifetime intervals ──────────────────────────
        let mut first_use: HashMap<String, usize> = HashMap::new();
        let mut last_use: HashMap<String, usize> = HashMap::new();

        for (sorted_idx, &pass_idx) in pass_order.iter().enumerate() {
            let pass = &graph.passes[pass_idx];
            for i in &pass.inputs {
                first_use.entry(i.name.clone()).or_insert(sorted_idx);
                last_use.insert(i.name.clone(), sorted_idx);
            }
            for o in &pass.outputs {
                first_use.entry(o.name.clone()).or_insert(sorted_idx);
                last_use.insert(o.name.clone(), sorted_idx);
            }
            if let Some(ref ds) = pass.depth_stencil {
                first_use.entry(ds.name.clone()).or_insert(sorted_idx);
                last_use.insert(ds.name.clone(), sorted_idx);
            }
        }

        // ── Step 2: build intervals, excluding exempt resources ─────────
        struct Interval {
            name: String,
            first: usize,
            last: usize,
        }

        let mut intervals: Vec<Interval> = Vec::new();
        for (name, &first) in &first_use {
            if self.exempt.iter().any(|e| e == name) {
                continue;
            }
            let last = *last_use.get(name).unwrap_or(&first);
            intervals.push(Interval {
                name: name.clone(),
                first,
                last,
            });
        }

        // Sort by first-use pass.
        intervals.sort_by_key(|iv| iv.first);

        // ── Step 3: greedy interval packing ─────────────────────────────
        let mut slot_ends: Vec<usize> = Vec::new();
        let mut slots: Vec<AliasSlot> = Vec::new();
        let mut resource_to_slot: HashMap<String, usize> = HashMap::new();

        for iv in &intervals {
            let mut placed = false;
            for (slot_idx, &end) in slot_ends.iter().enumerate() {
                if iv.first > end {
                    // Non-overlapping → alias into this slot.
                    slots[slot_idx].resources.push(iv.name.clone());
                    slot_ends[slot_idx] = slot_ends[slot_idx].max(iv.last);
                    resource_to_slot.insert(iv.name.clone(), slot_idx);
                    placed = true;
                    break;
                }
            }
            if !placed {
                // Need a new slot.
                let slot_idx = slots.len();
                slot_ends.push(iv.last);
                slots.push(AliasSlot {
                    resources: vec![iv.name.clone()],
                    slot_index: slot_idx,
                });
                resource_to_slot.insert(iv.name.clone(), slot_idx);
            }
        }

        AliasingPlan {
            slots,
            resource_to_slot,
        }
    }
}

impl Default for TransientResourcePool {
    fn default() -> Self {
        Self {
            exempt: vec!["swapchain".into()],
        }
    }
}

// ============================================================================
// Tests
// ============================================================================
