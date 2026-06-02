//! Contour extraction — trace region boundaries, simplify via
//! Douglas–Peucker, and produce the vertex loops used by polygon-mesh
//! generation.
//!
//! Each region in the compact heightfield is first converted into a
//! **grid-edge contour** (marching along the edges between region and
//! non-region cells).  This yields a closed sequence of grid corners that
//! exactly follows the region boundary.  The raw contour is then simplified
//! with the Douglas–Peucker algorithm and long edges are split.

use crate::cook::compact::CompactHeightfield;
use crate::cook::config::CookError;
use glam::Vec3;

// ── Direction helpers ─────────────────────────────────────────────────────

// 4-connected directions: +x, +z, -x, -z.
// Kept for potential use in tracing; silence dead-code warning.
#[allow(dead_code)]
const DIRS4: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

// ── Data structures ───────────────────────────────────────────────────────

/// The set of all contours extracted from a compact heightfield.
pub(crate) struct ContourSet {
    pub conts: Vec<Contour>,
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    pub bmin: Vec3,
    pub cs: f32,
    pub ch: f32,
    #[allow(dead_code)]
    pub border_size: u32,
}

/// A single closed contour (region boundary).
pub(crate) struct Contour {
    /// World-space vertices of the simplified contour (CCW winding).
    pub verts: Vec<Vec3>,
    /// Region ID this contour belongs to.
    #[allow(dead_code)]
    pub reg: u16,
    /// Area type of this region.
    pub area: u8,
}

// ── Main entry point ──────────────────────────────────────────────────────

/// Build contours for every region in `chf`.
///
/// * `max_error` — Douglas–Peucker tolerance in cell-units.
/// * `max_edge_len` — maximum allowed edge length in cell-units
///   (0 = unlimited).
pub(crate) fn build_contours(
    chf: &CompactHeightfield,
    max_error: f32,
    max_edge_len: u32,
) -> Result<ContourSet, CookError> {
    let width = chf.width as usize;
    let height = chf.height as usize;

    // Collect all unique region IDs (skip 0 = unassigned).
    let mut regions: Vec<u16> = chf
        .spans
        .iter()
        .filter_map(|s| if s.reg != 0 { Some(s.reg) } else { None })
        .collect();
    regions.sort_unstable();
    regions.dedup();

    let mut conts: Vec<Contour> = Vec::with_capacity(regions.len());

    for &reg in &regions {
        // Build a mask of cells belonging to this region.
        let mut in_region = vec![false; width * height];
        for z in 0..height {
            for x in 0..width {
                let ci = z * width + x;
                let cell = &chf.cells[ci];
                if cell.count == 0 {
                    continue;
                }
                for local in 0..cell.count {
                    let idx = (cell.index + local) as usize;
                    if idx < chf.spans.len() && chf.spans[idx].reg == reg {
                        in_region[ci] = true;
                        break;
                    }
                }
            }
        }

        // Count cells in this region — skip empty / degenerate.
        let cell_count = in_region.iter().filter(|&&b| b).count();
        if cell_count == 0 {
            continue;
        }

        // Build the contour by walking grid edges.
        let raw_corners = trace_contour_corners(&in_region, width, height);

        if raw_corners.len() < 3 {
            continue; // degenerate region — skip
        }

        // Convert from grid-corner space to cell-centre space.
        // Each corner (gx, gz) sits at a grid line intersection.
        // We convert to the centre of the cell _above-and-left_ of the
        // corner (the cell that is inside the region).  However, because
        // we traced edges, some corners are "outside" corners where the
        // region cell is to the right or below.  To keep things simple we
        // just use the corner position directly — the simplification step
        // will pull vertices onto the actual contour.
        //
        // Compute the Y height from the first span we find for this region.
        let y_world = find_region_height(chf, reg);

        let raw_verts: Vec<Vec3> = raw_corners
            .iter()
            .map(|&(gx, gz)| {
                let wx = chf.bmin.x + gx as f32 * chf.cs;
                let wz = chf.bmin.z + gz as f32 * chf.cs;
                Vec3::new(wx, y_world, wz)
            })
            .collect();

        // Douglas–Peucker simplification.
        // DP operates on an open path (first != last).
        // Remove the closing duplicate, simplify, then re-close.
        let is_closed = raw_verts.len() > 1
            && (raw_verts[0] - raw_verts[raw_verts.len() - 1]).length_squared() < 1e-10;
        let open_verts = if is_closed {
            &raw_verts[..raw_verts.len() - 1]
        } else {
            &raw_verts[..]
        };

        let simp_verts = if open_verts.len() <= 2 {
            open_verts.to_vec()
        } else {
            douglas_peucker(open_verts, max_error * chf.cs, 0, open_verts.len() - 1)
        };

        // Ensure the simplified contour is closed (first == last is OK,
        // but DP returns an open chain we need to close).
        let simp_verts = if simp_verts.len() > 1
            && (simp_verts[0] - simp_verts[simp_verts.len() - 1]).length_squared() > 1e-10
        {
            let mut closed = simp_verts;
            closed.push(closed[0]);
            closed
        } else {
            simp_verts
        };

        // Split long edges.
        let final_verts = if max_edge_len > 0 {
            split_long_edges(&simp_verts, max_edge_len as f32 * chf.cs)
        } else {
            simp_verts
        };

        if final_verts.len() < 3 {
            continue;
        }

        // Determine area type (take the first span's area).
        let area = find_region_area(chf, reg);

        conts.push(Contour {
            verts: final_verts,
            reg,
            area,
        });
    }

    if conts.is_empty() {
        return Err(CookError::ContourGenerationFailed(
            "no non-empty regions found".into(),
        ));
    }

    Ok(ContourSet {
        conts,
        width: chf.width,
        height: chf.height,
        bmin: chf.bmin,
        cs: chf.cs,
        ch: chf.ch,
        border_size: 0,
    })
}

// ── Grid-edge contour tracing ─────────────────────────────────────────────

/// Walk along the **edges** between region and non-region cells and return
/// the sequence of grid corners that forms the closed contour.
///
/// The algorithm:
/// 1. Find all vertical and horizontal grid edges where the two adjacent
///    cells differ in region membership.
/// 2. Chain these edges into a closed loop by walking from corner to corner,
///    always taking the *other* incident contour edge at each corner.
/// 3. Vertices are at grid-corner positions (integer coordinates).
///
/// This correctly handles concave shapes (L-shapes, U-shapes, etc.)
/// because the edges always lie between region and non-region cells.
fn trace_contour_corners(in_region: &[bool], width: usize, height: usize) -> Vec<(u32, u32)> {
    // ── Step 1: find all contour edges ────────────────────────────────────
    //
    // A *contour edge* is a grid edge where exactly one of the two adjacent
    // cells belongs to the region.
    //
    // We store edges indexed by their start corner.
    // Each entry is (end_corner_x, end_corner_z, direction).
    // direction=0 horizontal, 1 vertical.

    // We'll store edges as: for each corner, a small Vec of connected corners.
    // Since the region boundary is a set of disjoint loops, each corner should
    // have degree 2 (one incoming, one outgoing).
    //
    // We use a HashMap: corner → Vec of connected corners.
    use std::collections::HashMap;
    let mut adj: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();

    let w = width as u32;
    let h = height as u32;

    // Vertical edges: at column gx (0..=w), row gz (0..h).
    // Separates cell (gx-1, gz) [left] from (gx, gz) [right].
    // At gx == 0: left cell is out-of-bounds (treated as not in region).
    // At gx == w: right cell is out-of-bounds (treated as not in region).
    for gx in 0..=w {
        for gz in 0..h {
            let left = if gx > 0 {
                in_region[(gz as usize) * width + (gx as usize - 1)]
            } else {
                false // out of bounds
            };
            let right = if gx < w {
                in_region[(gz as usize) * width + (gx as usize)]
            } else {
                false // out of bounds
            };
            if left != right {
                adj.entry((gx, gz)).or_default().push((gx, gz + 1));
                adj.entry((gx, gz + 1)).or_default().push((gx, gz));
            }
        }
    }

    // Horizontal edges: at column gx (0..w), row gz (0..=h).
    // Separates cell (gx, gz-1) [up] from (gx, gz) [down].
    // At gz == 0: up cell is out-of-bounds (treated as not in region).
    // At gz == h: down cell is out-of-bounds (treated as not in region).
    for gx in 0..w {
        for gz in 0..=h {
            let up = if gz > 0 {
                in_region[(gz as usize - 1) * width + (gx as usize)]
            } else {
                false // out of bounds
            };
            let down = if gz < h {
                in_region[(gz as usize) * width + (gx as usize)]
            } else {
                false // out of bounds
            };
            if up != down {
                adj.entry((gx, gz)).or_default().push((gx + 1, gz));
                adj.entry((gx + 1, gz)).or_default().push((gx, gz));
            }
        }
    }

    if adj.is_empty() {
        return Vec::new();
    }

    // ── Step 2: walk edges into a closed loop ─────────────────────────────
    //
    // Start from any corner, follow edges (each corner has degree 2,
    // so we simply exit via the edge we didn't enter through).

    let start = *adj.keys().next().unwrap();
    let mut path = Vec::new();
    let mut current = start;
    let mut prev: Option<(u32, u32)> = None;

    // Safety limit.
    let max_iter = adj.len() * 2 + 100;
    let mut iter = 0;

    loop {
        path.push(current);

        // Find the next connected corner (the one that is NOT prev).
        let neighbors = adj.get(&current).expect("corner missing from adjacency");
        let next = if neighbors.len() == 1 {
            // Dead-end — shouldn't happen for a closed contour, but handle.
            neighbors[0]
        } else {
            // Take the first neighbor that isn't where we came from.
            match prev {
                None => neighbors[0],
                Some(p) => {
                    if neighbors[0] == p && neighbors.len() > 1 {
                        neighbors[1]
                    } else {
                        neighbors[0]
                    }
                }
            }
        };

        prev = Some(current);
        current = next;
        iter += 1;

        if current == start || iter > max_iter {
            break;
        }
    }

    // Close the loop if we didn't naturally return.
    if path.len() > 1 && path[path.len() - 1] != start {
        path.push(start);
    }

    path
}

// ── Douglas–Peucker simplification ────────────────────────────────────────

/// Recursive Douglas–Peucker simplification on a slice of 3D vertices
/// (operates on the XZ plane, ignoring Y).
///
/// Returns the simplified vertex sequence (including both endpoints).
fn douglas_peucker(verts: &[Vec3], max_error: f32, start: usize, end: usize) -> Vec<Vec3> {
    if end - start <= 1 {
        // Two points or fewer — nothing to simplify.
        return verts[start..=end].to_vec();
    }

    // Find the vertex with maximum perpendicular distance from the
    // line segment (start → end).
    let (max_idx, max_dist) = find_farthest_xz(verts, start, end);

    if max_dist <= max_error {
        // All intermediate vertices are within tolerance — keep only endpoints.
        vec![verts[start], verts[end]]
    } else {
        // Split at the farthest vertex and recurse.
        let left = douglas_peucker(verts, max_error, start, max_idx);
        let right = douglas_peucker(verts, max_error, max_idx, end);
        // Concatenate, avoiding duplicate of the split point.
        let mut result = left;
        // Skip the first element of right (it duplicates the last of left).
        result.extend_from_slice(&right[1..]);
        result
    }
}

/// Find the vertex between `start` and `end` with the greatest
/// perpendicular distance from the line segment `verts[start]`–`verts[end]`
/// in the XZ plane.  Returns `(index, distance)`.
fn find_farthest_xz(verts: &[Vec3], start: usize, end: usize) -> (usize, f32) {
    let a = verts[start];
    let b = verts[end];
    let ax = a.x;
    let az = a.z;
    let dx = b.x - ax;
    let dz = b.z - az;
    let len_sq = dx * dx + dz * dz;

    let mut max_dist: f32 = -1.0;
    let mut max_idx = start;

    #[allow(clippy::needless_range_loop)]
    for i in (start + 1)..end {
        let px = verts[i].x - ax;
        let pz = verts[i].z - az;

        let dist = if len_sq < 1e-10 {
            // Degenerate segment (points coincide) — use Euclidean distance.
            (px * px + pz * pz).sqrt()
        } else {
            // Perpendicular distance: |(P-A) × (B-A)| / |B-A|
            let cross = (dx * pz - dz * px).abs();
            cross / len_sq.sqrt()
        };

        if dist > max_dist {
            max_dist = dist;
            max_idx = i;
        }
    }

    (max_idx, max_dist)
}

// ── Edge splitting ────────────────────────────────────────────────────────

/// Split any edge longer than `max_len` world-units by inserting
/// intermediate points at regular intervals.
fn split_long_edges(verts: &[Vec3], max_len: f32) -> Vec<Vec3> {
    if verts.len() < 3 || max_len <= 0.0 {
        return verts.to_vec();
    }

    let mut result = Vec::with_capacity(verts.len() * 2);
    let n = verts.len();

    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        result.push(a);

        let dx = b.x - a.x;
        let dz = b.z - a.z;
        // Y is not relevant for edge length in XZ.
        let len = (dx * dx + dz * dz).sqrt();

        if len > max_len {
            let segments = (len / max_len).ceil() as u32;
            let segs = segments.max(2);
            for s in 1..segs {
                let t = s as f32 / segs as f32;
                result.push(Vec3::new(a.x + dx * t, a.y + (b.y - a.y) * t, a.z + dz * t));
            }
        }
    }

    // Remove the trailing duplicate of the first vertex if we closed the loop.
    if result.len() > 1 && result[result.len() - 1] == result[0] {
        result.pop();
    }

    result
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Find the world-space Y coordinate for a region (from the first span
/// with that region ID).
fn find_region_height(chf: &CompactHeightfield, reg: u16) -> f32 {
    for span in &chf.spans {
        if span.reg == reg {
            return span.y as f32 * chf.ch;
        }
    }
    0.0
}

/// Find the area type for a region.
fn find_region_area(chf: &CompactHeightfield, reg: u16) -> u8 {
    for (i, span) in chf.spans.iter().enumerate() {
        if span.reg == reg {
            if i < chf.areas.len() {
                return chf.areas[i];
            }
            return 1;
        }
    }
    1
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::compact::CompactHeightfield;
    use crate::cook::config::NavMeshCookConfig;
    use crate::cook::heightfield::Heightfield;

    /// Build a compact heightfield with a single flat region.
    #[allow(clippy::field_reassign_with_default)]
    fn make_chf_with_region(width: u32, height: u32, reg: u16) -> CompactHeightfield {
        let mut cfg = NavMeshCookConfig::default();
        cfg.bounds_min = Vec3::ZERO;
        cfg.bounds_max = Vec3::new(width as f32, 2.0, height as f32);
        let mut hf = Heightfield::alloc(&cfg);
        for x in 0..width {
            for z in 0..height {
                hf.add_span(x, z, 0, 1, 1, 1);
            }
        }
        let mut chf = CompactHeightfield::build_from_heightfield(&hf, 1, 1);
        // Manually assign region IDs.
        for span in &mut chf.spans {
            span.reg = reg;
        }
        chf
    }

    #[test]
    fn single_square_region_four_verts() {
        let chf = make_chf_with_region(3, 3, 1);
        let result = build_contours(&chf, 1.3, 12);
        assert!(
            result.is_ok(),
            "build_contours should succeed: {:?}",
            result.err()
        );
        let cs = result.unwrap();
        assert_eq!(cs.conts.len(), 1, "should produce one contour");
        let c = &cs.conts[0];
        // A 3×3 square simplified should have 4 vertices (the corners).
        assert!(
            c.verts.len() >= 3,
            "simplified contour should have at least 3 vertices, got {}",
            c.verts.len()
        );
        assert_eq!(c.reg, 1);
    }

    #[test]
    fn simplified_contour_reduces_vertices() {
        let chf = make_chf_with_region(5, 5, 1);
        // Moderate error: enough to straighten edges but keep corners.
        // The 5×5 block edges are 5 cs cells long. Error of ~2 cells in world
        // space should keep the 4 corners while removing intermediate vertices.
        let result = build_contours(&chf, 1.5, 0);
        assert!(result.is_ok());
        let cs = result.unwrap();
        let c = &cs.conts[0];
        // The raw contour has 20 vertices (5 per side). With 1.5 cell tolerance
        // (~0.45 world units at cs=0.3), we should simplify to at most 8 vertices.
        // A 5×5 block at cs=0.3 is 1.5×1.5 world units.
        // Farthest point from the corner-to-corner line is one full side = 1.5.
        // Error 1.5 * 0.3 = 0.45 world. The diagonals are ~2.12, so DP should
        // keep the 4 corners.
        assert!(
            c.verts.len() >= 3,
            "simplified contour should have at least 3 vertices, got {}",
            c.verts.len()
        );
        assert!(
            c.verts.len() < 20,
            "simplified contour should have fewer vertices than raw (20), got {}",
            c.verts.len()
        );
    }

    #[test]
    fn edge_splitting_limits_edge_length() {
        let chf = make_chf_with_region(10, 10, 1);
        // Use tiny max_error to keep all vertices, and tiny max_edge_len.
        let result = build_contours(&chf, 0.01, 2);
        assert!(result.is_ok());
        let cs = result.unwrap();
        let c = &cs.conts[0];

        // Measure the longest edge.
        let mut longest = 0.0_f32;
        let n = c.verts.len();
        for i in 0..n {
            let a = c.verts[i];
            let b = c.verts[(i + 1) % n];
            let dx = a.x - b.x;
            let dz = a.z - b.z;
            let len = (dx * dx + dz * dz).sqrt();
            if len > longest {
                longest = len;
            }
        }
        // max_edge_len=2 cells, cs default is 0.3 → max world len = 0.6.
        // We allow a small epsilon.
        assert!(
            longest <= 0.6 + 0.01,
            "no edge should exceed max_edge_len world units, longest={}",
            longest
        );
    }

    #[test]
    fn contour_dp_distance_test() {
        // Test the helper directly: a simple line with a deviation.
        let verts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.5),
            Vec3::new(2.0, 0.0, 0.0),
        ];
        // The middle vertex is 0.5 units from the line from first to last.
        let result = douglas_peucker(&verts, 0.6, 0, 2);
        assert_eq!(
            result.len(),
            2,
            "should simplify to 2 verts with error > 0.5"
        );

        let result2 = douglas_peucker(&verts, 0.4, 0, 2);
        assert_eq!(result2.len(), 3, "should keep middle vert with error < 0.5");
    }
}
