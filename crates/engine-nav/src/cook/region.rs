//! Watershed region generation for navmesh baking.
//!
//! This module implements Recast's watershed algorithm to partition the
//! compact heightfield into connected walkable regions.  Each region
//! will later be converted into one or more convex navigation-mesh
//! polygons via contour tracing and polygonisation.
//!
//! ## Algorithm overview
//!
//! 1. **Bucket spans by distance** — interior walkable spans are sorted
//!    into layers by their Chamfer-distance value.
//! 2. **Watershed flooding** — distance levels are processed from highest
//!    (deepest interior) down to 1:
//!    a. **Expand** — existing regions grow outward into the current level
//!       via iterative neighbour-propagation.
//!    b. **Seed** — any unassigned span at this level starts a new region
//!       via flood-fill through same-level connected spans.
//! 3. **Grow** — regions expand into any remaining unassigned spans
//!    (distance 0 borders).
//! 4. **Filter** — regions smaller than `min_region_area` are discarded.
//! 5. **Merge** — regions smaller than `merge_region_area` are absorbed
//!    into their largest neighbour.

use super::compact::{CompactHeightfield, CompactSpan};
use super::config::CookError;

// ── Constants ────────────────────────────────────────────────────────────────

/// Sentinel value meaning "no connection" in the packed neighbour field.
const RC_NOT_CONNECTED: u32 = 0x3f;

/// Maximum number of expansion iterations per watershed level.
const MAX_EXPAND_ITER: u32 = 8;

/// Maximum number of iterations for the final grow phase.
const MAX_GROW_ITER: u32 = 16;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract the packed neighbour index for direction `dir` (0 = +x, 1 = +z,
/// 2 = –x, 3 = –z).  Returns `RC_NOT_CONNECTED` (0x3f) when no neighbour
/// exists in that direction.
#[inline]
fn get_con(s: &CompactSpan, dir: usize) -> u32 {
    (s.con >> (dir * 6)) & 0x3f
}

/// Returns the global span index of the neighbour in `dir`, or `None` if
/// the neighbour does not exist (cell boundary or unconnected).
fn neighbor_span_idx(
    chf: &CompactHeightfield,
    x: u32,
    z: u32,
    local: u32,
    dir: usize,
) -> Option<u32> {
    // Read the current span to get the packed con value.
    let cell = &chf.cells[(z * chf.width + x) as usize];
    let s = &chf.spans[(cell.index + local) as usize];
    let con = get_con(s, dir);
    if con == RC_NOT_CONNECTED {
        return None;
    }

    // Neighbour cell coordinates.
    static DIRS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
    let nx = x as i32 + DIRS[dir].0;
    let nz = z as i32 + DIRS[dir].1;
    if nx < 0 || nz < 0 || nx >= chf.width as i32 || nz >= chf.height as i32 {
        return None;
    }
    let nci = (nz as u32 * chf.width + nx as u32) as usize;
    let ncell = &chf.cells[nci];
    if ncell.count == 0 || con >= ncell.count {
        return None;
    }
    Some(ncell.index + con)
}

// ── Span metadata ────────────────────────────────────────────────────────────

/// Precomputed cell position for every span in the flat `spans[]` array.
///
/// Watershed operations frequently need to traverse from a global span
/// index to its 4-direction neighbours, which requires the (x, z) cell
/// coordinate and the span's local index within that cell.
#[derive(Clone, Copy, Debug)]
struct SpanCell {
    x: u32,
    z: u32,
    local: u32,
}

/// Build the span→cell lookup table.
fn build_span_cells(chf: &CompactHeightfield) -> Vec<SpanCell> {
    let mut meta = vec![
        SpanCell {
            x: 0,
            z: 0,
            local: 0
        };
        chf.spans.len()
    ];
    for z in 0..chf.height {
        for x in 0..chf.width {
            let ci = (z * chf.width + x) as usize;
            let cell = &chf.cells[ci];
            for local in 0..cell.count {
                let idx = (cell.index + local) as usize;
                meta[idx] = SpanCell { x, z, local };
            }
        }
    }
    meta
}

/// Group span indices by distance level.
///
/// Returns a vector where `layers[d]` holds the global span indices for all
/// walkable interior spans at distance `d` (d ≥ 1).  `layers[0]` is always
/// empty (border spans are not seeded).
fn build_layers(chf: &CompactHeightfield) -> Vec<Vec<u32>> {
    let max_dist = chf.max_dist as usize;
    let mut layers = vec![Vec::new(); max_dist + 1];
    for (idx, &d) in chf.dist.iter().enumerate() {
        if d > 0 && chf.areas[idx] > 0 {
            layers[d as usize].push(idx as u32);
        }
    }
    layers
}

// ── Flood helpers ────────────────────────────────────────────────────────────

/// Expand existing regions into unassigned spans at `level`.
///
/// Each unassigned span (dist == level, reg == 0, area > 0) is examined;
/// if any of its 4 neighbours has a non-zero region ID, the span inherits
/// that region.  This step is repeated for up to `max_iter` iterations
/// (equivalent to a constrained morphological dilation).
fn expand_regions(chf: &mut CompactHeightfield, level: u16, max_iter: u32) {
    for _ in 0..max_iter {
        let mut expanded = false;

        for z in 0..chf.height {
            for x in 0..chf.width {
                let ci = (z * chf.width + x) as usize;
                let cell = &chf.cells[ci];
                if cell.count == 0 {
                    continue;
                }
                for local in 0..cell.count {
                    let idx = (cell.index + local) as usize;

                    // Only spans at the current level that are unassigned.
                    if chf.dist[idx] != level {
                        continue;
                    }
                    if chf.spans[idx].reg != 0 || chf.areas[idx] == 0 {
                        continue;
                    }

                    // Check all 4 neighbours.
                    for dir in 0..4 {
                        if let Some(nidx) = neighbor_span_idx(chf, x, z, local, dir) {
                            let nreg = chf.spans[nidx as usize].reg;
                            if nreg != 0 {
                                chf.spans[idx].reg = nreg;
                                expanded = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        if !expanded {
            break;
        }
    }
}

/// Watershed flood-fill from a seed span.
///
/// Assigns `new_reg_id` to the seed and then performs a DFS (stack-based)
/// through all connected, unassigned walkable spans whose distance equals
/// the seed's distance level.  Only 4-directional connectivity (via the
/// packed `con` field) is considered.
fn flood_region(
    chf: &mut CompactHeightfield,
    start_idx: u32,
    new_reg_id: u16,
    level: u16,
    meta: &[SpanCell],
) {
    let mut stack = Vec::new();
    stack.push(start_idx as usize);
    chf.spans[start_idx as usize].reg = new_reg_id;

    while let Some(idx) = stack.pop() {
        let mc = &meta[idx];
        for dir in 0..4 {
            if let Some(nidx) = neighbor_span_idx(chf, mc.x, mc.z, mc.local, dir) {
                let nidx = nidx as usize;
                if chf.spans[nidx].reg == 0 && chf.areas[nidx] > 0 && chf.dist[nidx] == level {
                    chf.spans[nidx].reg = new_reg_id;
                    stack.push(nidx);
                }
            }
        }
    }
}

// ── Filter & merge ───────────────────────────────────────────────────────────

/// Remove every region whose span count is below `min_area`.
///
/// Affected spans are reset to 0 (unassigned) so they can be absorbed
/// during the merge or grow phase.
fn filter_small_regions(chf: &mut CompactHeightfield, min_area: u32) {
    if min_area == 0 {
        return;
    }

    // Find the highest region ID in use.
    let max_reg = chf.spans.iter().map(|s| s.reg).max().unwrap_or(0) as usize;

    if max_reg == 0 {
        return;
    }

    // Count spans per region.
    let mut counts = vec![0u32; max_reg + 1];
    for (idx, s) in chf.spans.iter().enumerate() {
        if chf.areas[idx] > 0 && s.reg != 0 {
            counts[s.reg as usize] += 1;
        }
    }

    // Zero out regions below threshold.
    for idx in 0..chf.spans.len() {
        if chf.areas[idx] > 0 {
            let reg = chf.spans[idx].reg;
            if reg != 0 && counts[reg as usize] < min_area {
                chf.spans[idx].reg = 0;
            }
        }
    }
}

/// Merge small regions (< `merge_area`) into their largest neighbour.
///
/// The algorithm iterates until convergence: for each region below the
/// threshold, we scan all spans on its boundary to collect adjacent
/// region IDs with contact counts, then merge the small region into
/// whichever neighbour has the longest shared border.  This prevents
/// tiny fragmented regions from surviving into the final mesh.
fn merge_regions(chf: &mut CompactHeightfield, merge_area: u32) {
    if merge_area == 0 {
        return;
    }

    // First pass: re-count regions (some may have been zeroed by filtering).
    let max_reg = chf.spans.iter().map(|s| s.reg).max().unwrap_or(0) as usize;

    if max_reg < 2 {
        return;
    }

    let mut counts = vec![0u32; max_reg + 1];
    for (idx, s) in chf.spans.iter().enumerate() {
        if chf.areas[idx] > 0 && s.reg != 0 {
            counts[s.reg as usize] += 1;
        }
    }

    loop {
        let mut any_merged = false;

        // Collect a list of regions below the merge threshold (sorted by
        // region ID for determinism).
        let small_regions: Vec<u16> = (2..=max_reg as u16)
            .filter(|&r| counts[r as usize] > 0 && counts[r as usize] < merge_area)
            .collect();

        if small_regions.is_empty() {
            break;
        }

        for &reg_id in &small_regions {
            if counts[reg_id as usize] == 0 {
                continue; // already merged in a previous iteration
            }

            // Scan all spans belonging to this region and collect neighbour
            // region contact counts.
            let mut adj: Vec<(u16, u32)> = Vec::new();
            let mut adj_map: Vec<u32> = vec![0u32; max_reg + 1];

            for z in 0..chf.height {
                for x in 0..chf.width {
                    let ci = (z * chf.width + x) as usize;
                    let cell = &chf.cells[ci];
                    if cell.count == 0 {
                        continue;
                    }
                    for local in 0..cell.count {
                        let idx = (cell.index + local) as usize;
                        if chf.spans[idx].reg != reg_id || chf.areas[idx] == 0 {
                            continue;
                        }
                        for dir in 0..4 {
                            if let Some(nidx) = neighbor_span_idx(chf, x, z, local, dir) {
                                let nreg = chf.spans[nidx as usize].reg;
                                if nreg != 0 && nreg != reg_id {
                                    adj_map[nreg as usize] += 1;
                                }
                            }
                        }
                    }
                }
            }

            for (nid, &cnt) in adj_map.iter().enumerate().skip(2) {
                if cnt > 0 {
                    adj.push((nid as u16, cnt));
                }
            }

            if adj.is_empty() {
                // Isolated small region — it gets to stay as-is.
                continue;
            }

            // Merge into the neighbour with the most shared edges.
            adj.sort_by(|a, b| b.1.cmp(&a.1));
            let best = adj[0].0;

            // Also skip if the best neighbour is itself small — we might
            // merge into another small region.  That's fine; it will be
            // handled in the next iteration.
            for idx in 0..chf.spans.len() {
                if chf.spans[idx].reg == reg_id && chf.areas[idx] > 0 {
                    chf.spans[idx].reg = best;
                    counts[best as usize] += 1;
                }
            }
            counts[reg_id as usize] = 0;
            any_merged = true;
        }

        if !any_merged {
            break;
        }
    }
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Build watershed regions on a compact heightfield with a pre-computed
/// distance field.
///
/// `min_region_area` — regions with fewer walkable spans are discarded.
/// `merge_region_area` — regions below this size are merged into their
/// largest neighbour (after filtering).
///
/// Returns the number of regions created, or an error if no walkable
/// surface exists.
pub(crate) fn build_regions(
    chf: &mut CompactHeightfield,
    min_region_area: u32,
    merge_region_area: u32,
) -> Result<u16, CookError> {
    // Bail early if there is nothing to process.
    let walkable = chf.areas.iter().filter(|&&a| a > 0).count();
    if walkable == 0 {
        return Err(CookError::RegionGenerationFailed);
    }

    // If the distance field hasn't been computed (all zero), we cannot
    // run watershed.  This is unlikely in practice, but guard against it.
    if chf.max_dist == 0 {
        // Fall back: assign everything to a single region.
        for (idx, s) in chf.spans.iter_mut().enumerate() {
            if chf.areas[idx] > 0 {
                s.reg = 1;
            }
        }
        return Ok(1);
    }

    // ── 1. Prepare data structures ───────────────────────────────────────
    let meta = build_span_cells(chf);
    let layers = build_layers(chf);

    let mut next_reg_id: u16 = 1;

    // ── 2. Watershed flooding (high→low distance) ───────────────────────
    for level in (1..=chf.max_dist).rev() {
        // 2a. Expand existing regions into this level.
        expand_regions(chf, level, MAX_EXPAND_ITER);

        // 2b. Seed new regions from any remaining unassigned spans.
        for &span_idx in &layers[level as usize] {
            let idx = span_idx as usize;
            if chf.spans[idx].reg != 0 || chf.areas[idx] == 0 {
                continue;
            }
            let new_id = next_reg_id;
            next_reg_id += 1;
            flood_region(chf, span_idx, new_id, level, &meta);
        }
    }

    // ── 3. Grow into remaining unassigned spans (distance 0, or missed) ─
    for _ in 0..MAX_GROW_ITER {
        let mut expanded = false;

        for z in 0..chf.height {
            for x in 0..chf.width {
                let ci = (z * chf.width + x) as usize;
                let cell = &chf.cells[ci];
                if cell.count == 0 {
                    continue;
                }
                for local in 0..cell.count {
                    let idx = (cell.index + local) as usize;
                    if chf.spans[idx].reg != 0 || chf.areas[idx] == 0 {
                        continue;
                    }
                    for dir in 0..4 {
                        if let Some(nidx) = neighbor_span_idx(chf, x, z, local, dir) {
                            let nreg = chf.spans[nidx as usize].reg;
                            if nreg != 0 {
                                chf.spans[idx].reg = nreg;
                                expanded = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        if !expanded {
            break;
        }
    }

    // ── 4. Filter small regions ─────────────────────────────────────────
    filter_small_regions(chf, min_region_area);

    // ── 5. Merge small regions ──────────────────────────────────────────
    merge_regions(chf, merge_region_area);

    // ── 6. Count remaining regions ──────────────────────────────────────
    let max_reg = chf.spans.iter().map(|s| s.reg).max().unwrap_or(0);

    // Renumber regions to be contiguous starting from 1.
    let mut remap = vec![0u16; (max_reg as usize + 1).max(2)];
    let mut count = 0u16;
    for (idx, s) in chf.spans.iter_mut().enumerate() {
        if chf.areas[idx] > 0 && s.reg != 0 {
            let r = s.reg as usize;
            if remap[r] == 0 {
                count += 1;
                remap[r] = count;
            }
            s.reg = remap[r];
        }
    }

    Ok(count)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::compact::CompactHeightfield;
    use crate::cook::config::NavMeshCookConfig;
    use crate::cook::heightfield::Heightfield;
    use glam::Vec3;

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Build a compact heightfield from a flat 10×10 walkable platform
    /// and compute its distance field.
    fn flat_10x10() -> CompactHeightfield {
        let mut cfg = NavMeshCookConfig::default();
        cfg.bounds_min = Vec3::ZERO;
        cfg.bounds_max = Vec3::new(3.0, 2.0, 3.0);
        let mut hf = Heightfield::alloc(&cfg);
        for x in 0..10 {
            for z in 0..10 {
                hf.add_span(x, z, 0, 1, 1, 1);
            }
        }
        let mut chf = CompactHeightfield::build_from_heightfield(&hf, 5, 2);
        chf.build_distance_field();
        chf
    }

    /// Build two disconnected 4×4 platforms separated by a 2-cell gap.
    fn two_platforms() -> CompactHeightfield {
        let mut cfg = NavMeshCookConfig::default();
        cfg.bounds_min = Vec3::ZERO;
        cfg.bounds_max = Vec3::new(12.0, 2.0, 6.0);
        let mut hf = Heightfield::alloc(&cfg);
        // Left platform (x 0..4, z 0..4)
        for x in 0..4 {
            for z in 0..4 {
                hf.add_span(x, z, 0, 1, 1, 1);
            }
        }
        // Right platform (x 6..10, z 0..4)
        for x in 6..10 {
            for z in 0..4 {
                hf.add_span(x, z, 0, 1, 1, 1);
            }
        }
        let mut chf = CompactHeightfield::build_from_heightfield(&hf, 5, 2);
        chf.build_distance_field();
        chf
    }

    /// Build a platform that has one large area and a small 2×2 bump
    /// connected by a narrow 1-cell corridor.
    fn platform_with_appendage() -> CompactHeightfield {
        let mut cfg = NavMeshCookConfig::default();
        cfg.bounds_min = Vec3::ZERO;
        cfg.bounds_max = Vec3::new(10.0, 2.0, 10.0);
        let mut hf = Heightfield::alloc(&cfg);
        // Main body: 8×8
        for x in 0..8 {
            for z in 0..8 {
                hf.add_span(x, z, 0, 1, 1, 1);
            }
        }
        // Small appendage: 2×4 connected at x=8, z=2..6
        for x in 8..10 {
            for z in 2..6 {
                hf.add_span(x, z, 0, 1, 1, 1);
            }
        }
        let mut chf = CompactHeightfield::build_from_heightfield(&hf, 5, 2);
        chf.build_distance_field();
        chf
    }

    // ── Tests ───────────────────────────────────────────────────────────

    #[test]
    fn single_region_flat_plane() {
        let mut chf = flat_10x10();
        let n = build_regions(&mut chf, 4, 20).unwrap();
        // Entire 10×10 platform should be one connected region.
        assert_eq!(n, 1, "flat plane should be a single region");

        // All walkable spans should be assigned.
        for (idx, s) in chf.spans.iter().enumerate() {
            if chf.areas[idx] > 0 {
                assert!(s.reg != 0, "span {} is unassigned", idx);
            }
        }
        // All assigned spans should have region 1.
        for s in &chf.spans {
            assert!(s.reg == 0 || s.reg == 1);
        }
    }

    #[test]
    fn two_disconnected_regions() {
        let mut chf = two_platforms();
        let n = build_regions(&mut chf, 4, 20).unwrap();
        // Two disconnected 4×4 platforms → 2 regions (each has 16 spans
        // which is well above min_region_area=4).
        assert_eq!(n, 2, "two disconnected platforms should produce 2 regions");

        // Verify each region is internally connected: count spans per region.
        let mut counts: Vec<u32> = vec![0u32; (n as usize + 1).max(3)];
        for (idx, s) in chf.spans.iter().enumerate() {
            if chf.areas[idx] > 0 && s.reg != 0 {
                let r = s.reg as usize;
                if r >= counts.len() {
                    counts.resize(r + 1, 0);
                }
                counts[r] += 1;
            }
        }
        for r in 1..=n as usize {
            assert_eq!(counts[r], 16, "region {} should have exactly 16 spans", r);
        }
    }

    #[test]
    fn small_region_filtered() {
        let mut chf = platform_with_appendage();
        // Use a large min_region_area (40) so the 2×4 = 8-span appendage
        // gets filtered out, leaving only the main 8×8 = 64-span region.
        let n = build_regions(&mut chf, 40, 20).unwrap();
        assert_eq!(n, 1, "small appendage should be filtered out");
    }

    #[test]
    fn small_region_merged() {
        let mut chf = platform_with_appendage();
        // Use min_region_area=4 (keeps everything) but merge_region_area=40
        // so the 8-span appendage gets merged into the 64-span main region.
        let n = build_regions(&mut chf, 4, 40).unwrap();
        assert_eq!(n, 1, "small appendage should be merged into main region");
    }

    #[test]
    fn empty_returns_error() {
        let mut cfg = NavMeshCookConfig::default();
        cfg.bounds_min = Vec3::ZERO;
        cfg.bounds_max = Vec3::new(3.0, 2.0, 3.0);
        let hf = Heightfield::alloc(&cfg);
        // No spans added → no walkable surface.
        let mut chf = CompactHeightfield::build_from_heightfield(&hf, 5, 2);
        chf.build_distance_field();
        let result = build_regions(&mut chf, 4, 20);
        assert!(result.is_err(), "empty heightfield should return error");
    }

    #[test]
    fn no_unassigned_spans_after_regions() {
        let mut chf = flat_10x10();
        build_regions(&mut chf, 4, 20).unwrap();
        // All walkable spans must be assigned.
        for (idx, s) in chf.spans.iter().enumerate() {
            if chf.areas[idx] > 0 {
                assert!(
                    s.reg != 0,
                    "walkable span {} should be assigned a region",
                    idx
                );
            }
        }
    }

    #[test]
    fn region_ids_contiguous() {
        let mut chf = two_platforms();
        let n = build_regions(&mut chf, 4, 20).unwrap();
        // Region IDs must be 1..=n with no gaps.
        let mut seen = vec![false; n as usize + 1];
        for (idx, s) in chf.spans.iter().enumerate() {
            if chf.areas[idx] > 0 && s.reg != 0 {
                let r = s.reg as usize;
                if r <= n as usize {
                    seen[r] = true;
                }
            }
        }
        for r in 1..=n as usize {
            assert!(seen[r], "region ID {} is unused after renumbering", r);
        }
    }

    #[test]
    fn min_region_area_zero_preserves_all() {
        let mut chf = two_platforms();
        let n = build_regions(&mut chf, 0, 0).unwrap();
        // With min_area=0 and merge_area=0, both platforms should survive.
        assert_eq!(n, 2, "both platforms should survive with zero thresholds");
    }
}
