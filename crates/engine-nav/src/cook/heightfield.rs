//! Voxelisation — rasterises input triangles into a solid heightfield.
//!
//! This module mirrors Recast's `rcHeightfield` / `rcSpan` and the three
//! standard walkability filters:
//!
//! 1. **Low-hanging obstacles** — small steps below `walkable_climb` are
//!    made walkable (lets agents step over kerbs, small rocks, etc.).
//! 2. **Ledge / slope filter** — spans whose neighbour floor drops more
//!    than `walkable_climb` or lacks `walkable_height` headroom are marked
//!    non-walkable.
//! 3. **Low-height filter** — spans with less than `walkable_height`
//!    headroom above are marked non-walkable (prevents agents from walking
//!    under low ceilings).

use crate::cook::config::NavMeshCookConfig;
use glam::Vec3;

// ── Constants ────────────────────────────────────────────────────────────────

/// Sentinel value meaning "no span" in index-based linked lists.
pub(crate) const NULL_SPAN: u32 = u32::MAX;

/// Maximum number of vertices per polygon during clipping.
#[allow(dead_code)]
const MAX_CLIP_VERTS: usize = 12;

// ── Span ─────────────────────────────────────────────────────────────────────

/// A single vertical run of solid voxels within one grid column.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    /// Bottom of the solid span in cell-height units (inclusive).
    pub smin: u16,
    /// Top of the solid span in cell-height units (exclusive).
    pub smax: u16,
    /// Index of the next span higher in this column, or `NULL_SPAN`.
    pub next: u32,
    /// Area type (0 = unwalkable, 1 = walkable, >1 = user-defined).
    pub area: u8,
}

// ── Heightfield ──────────────────────────────────────────────────────────────

/// A 2D grid of vertical span columns — the raw voxel representation of
/// the input geometry.
pub(crate) struct Heightfield {
    pub width: u32,
    pub height: u32,
    pub bmin: Vec3,
    #[allow(dead_code)]
    pub bmax: Vec3,
    pub cs: f32,
    pub ch: f32,
    /// Flat pool of all spans; new spans are always pushed.
    pub(crate) spans: Vec<Span>,
    /// For each cell `(x,z)`: index of the *lowest* span in that column
    /// (or `NULL_SPAN` if the column is empty).
    pub(crate) cells: Vec<u32>,
}

impl Heightfield {
    /// Allocate a new heightfield sized from `cfg`.
    pub fn alloc(cfg: &NavMeshCookConfig) -> Self {
        let (w, h) = cfg.grid_size();
        let cell_count = (w * h) as usize;
        Self {
            width: w,
            height: h,
            bmin: cfg.bounds_min,
            bmax: cfg.bounds_max,
            cs: cfg.cell_size,
            ch: cfg.cell_height,
            spans: Vec::with_capacity(cell_count * 2),
            cells: vec![NULL_SPAN; cell_count],
        }
    }

    /// Convert a world-space position to grid cell coordinates.
    fn world_to_cell(&self, p: Vec3) -> (i32, i32) {
        let x = ((p.x - self.bmin.x) / self.cs).floor() as i32;
        let z = ((p.z - self.bmin.z) / self.cs).floor() as i32;
        (x, z)
    }

    fn cell_index(&self, x: u32, z: u32) -> usize {
        (z * self.width + x) as usize
    }

    /// Add a span to column `(x, z)`, merging with existing spans when they
    /// overlap or are adjacent within `merge_threshold` voxels.
    ///
    /// This is the hot path of rasterisation — called once per grid cell
    /// covered by a triangle.
    pub(crate) fn add_span(
        &mut self,
        x: u32,
        z: u32,
        smin: u16,
        smax: u16,
        area: u8,
        merge_threshold: u16,
    ) {
        if x >= self.width || z >= self.height {
            return;
        }
        if smin >= smax {
            return;
        }
        let idx = self.cell_index(x, z);

        // If column is empty, just insert the new span.
        if self.cells[idx] == NULL_SPAN {
            let new_idx = self.spans.len() as u32;
            self.spans.push(Span {
                smin,
                smax,
                next: NULL_SPAN,
                area,
            });
            self.cells[idx] = new_idx;
            return;
        }

        // Walk the linked list to find insertion/merge point.
        let mut prev = NULL_SPAN;
        let mut curr = self.cells[idx];
        let mut new_smin = smin;
        let mut new_smax = smax;
        let mut new_area = area;

        while curr != NULL_SPAN {
            let s_smin = self.spans[curr as usize].smin;
            let s_smax = self.spans[curr as usize].smax;
            let s_area = self.spans[curr as usize].area;
            let s_next = self.spans[curr as usize].next;

            // Check overlap or adjacency within threshold.
            if new_smax + merge_threshold >= s_smin && new_smin <= s_smax + merge_threshold {
                // Merge: extend bounds.
                if s_smin < new_smin {
                    new_smin = s_smin;
                }
                if s_smax > new_smax {
                    new_smax = s_smax;
                }
                if s_area > new_area {
                    new_area = s_area;
                }

                // Remove current span by unlinking it.
                if prev == NULL_SPAN {
                    self.cells[idx] = s_next;
                } else {
                    self.spans[prev as usize].next = s_next;
                }
                curr = s_next;
                continue; // re-check with next span (merged might overlap more)
            } else if new_smax < s_smin {
                // Insert before current span.
                let new_idx = self.spans.len() as u32;
                self.spans.push(Span {
                    smin: new_smin,
                    smax: new_smax,
                    next: curr,
                    area: new_area,
                });
                if prev == NULL_SPAN {
                    self.cells[idx] = new_idx;
                } else {
                    self.spans[prev as usize].next = new_idx;
                }
                return;
            }

            prev = curr;
            curr = s_next;
        }

        // Append at end of list.
        let new_idx = self.spans.len() as u32;
        self.spans.push(Span {
            smin: new_smin,
            smax: new_smax,
            next: NULL_SPAN,
            area: new_area,
        });
        if prev == NULL_SPAN {
            self.cells[idx] = new_idx;
        } else {
            self.spans[prev as usize].next = new_idx;
        }
    }

    /// Rasterise a set of triangles into the heightfield.
    ///
    /// `vertices` and `indices` define the triangle soup (index triplets).
    /// Every triangle is projected onto the XZ plane, its covered grid cells
    /// are determined by scanline walk, and the vertical extent
    /// (`min_y` … `max_y`) is recorded as a span.
    pub fn rasterize_triangles(&mut self, vertices: &[Vec3], indices: &[u32], area: u8) {
        let merge_threshold = 1; // merge adjacent spans within 1 voxel
        for chunk in indices.chunks(3) {
            if chunk.len() < 3 {
                break;
            }
            let i0 = chunk[0] as usize;
            let i1 = chunk[1] as usize;
            let i2 = chunk[2] as usize;
            if i0 >= vertices.len() || i1 >= vertices.len() || i2 >= vertices.len() {
                continue;
            }
            let a = vertices[i0];
            let b = vertices[i1];
            let c = vertices[i2];

            // Reject degenerate / steep triangles.
            let e1 = b - a;
            let e2 = c - a;
            let n = e1.cross(e2);
            let twice_area = n.length();
            if twice_area < 1e-10 {
                continue;
            }
            // Check slope against walkable_angle.
            // Since we don't have walkable_slope here, accept all for now;
            // filtering happens later in CompactHeightfield.

            self.rasterize_triangle(a, b, c, area, merge_threshold);
        }
    }

    /// Rasterise a single triangle (internal).
    fn rasterize_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, area: u8, merge: u16) {
        // Project triangle onto XZ, find grid bounding box.
        let min_x = a.x.min(b.x).min(c.x);
        let max_x = a.x.max(b.x).max(c.x);
        let min_z = a.z.min(b.z).min(c.z);
        let max_z = a.z.max(b.z).max(c.z);

        let (ix0, iz0) = self.world_to_cell(Vec3::new(min_x, 0.0, min_z));
        let (ix1, iz1) = self.world_to_cell(Vec3::new(max_x, 0.0, max_z));

        // Clamp to grid.
        let _ix0 = ix0.max(0).min(self.width as i32 - 1) as u32;
        let _ix1 = ix1.max(0).min(self.width as i32 - 1) as u32;
        let iz0 = iz0.max(0).min(self.height as i32 - 1) as u32;
        let iz1 = iz1.max(0).min(self.height as i32 - 1) as u32;

        // For each grid row, compute the triangle's intersection with
        // that row (a scanline segment), then for each cell compute Y range.
        for iz in iz0..=iz1 {
            let cell_z = self.bmin.z + (iz as f32 + 0.5) * self.cs;

            // Edge function to test if a point (x,z) is inside the triangle.
            let edge = |p: Vec3, q: Vec3, r: Vec3| -> f32 {
                (q.x - p.x) * (r.z - p.z) - (q.z - p.z) * (r.x - p.x)
            };

            // Compute min/max x of the triangle on this scanline (z = cell_z).
            let x_on_edge = |p: Vec3, q: Vec3, z: f32| -> f32 {
                if (q.z - p.z).abs() < 1e-10 {
                    return f32::NAN;
                } // horizontal edge
                p.x + (z - p.z) * (q.x - p.x) / (q.z - p.z)
            };

            // Collect x intersections on this row.
            let mut xs = Vec::with_capacity(4);
            for edge_verts in &[(a, b), (b, c), (c, a)] {
                let (p, q) = edge_verts;
                // Check if the edge spans this z row.
                let z_min = p.z.min(q.z);
                let z_max = p.z.max(q.z);
                if cell_z >= z_min && cell_z <= z_max {
                    let x = x_on_edge(*p, *q, cell_z);
                    if x.is_finite() {
                        xs.push(x);
                    }
                }
            }

            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let scan_min_x = xs[0];
            let scan_max_x = xs[xs.len() - 1];

            let sx0 = ((scan_min_x - self.bmin.x) / self.cs).floor().max(0.0) as u32;
            let sx1 = ((scan_max_x - self.bmin.x) / self.cs)
                .floor()
                .min((self.width - 1) as f32) as u32;

            for ix in sx0..=sx1 {
                let cell_x = self.bmin.x + (ix as f32 + 0.5) * self.cs;
                let p = Vec3::new(cell_x, 0.0, cell_z);

                // Point-in-triangle test via edge functions.
                let e0 = edge(b, a, p);
                let e1 = edge(c, b, p);
                let e2 = edge(a, c, p);
                let inside =
                    (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0);
                if !inside {
                    continue;
                }

                // Compute Y range of the triangle at this cell centre.
                // Barycentric coordinates would be more accurate, but for
                // voxelisation just sample the plane equation.
                let normal = (b - a).cross(c - a);
                let n_len = normal.length();
                if n_len < 1e-10 {
                    continue;
                }
                let n = normal / n_len;
                let d = n.dot(a);

                // Plane: n · point = d  →  y = (d - n.x*x - n.z*z) / n.y
                if n.y.abs() < 1e-10 {
                    continue;
                } // vertical triangle shouldn't be walkable, skip
                let surface_y = (d - n.x * cell_x - n.z * cell_z) / n.y;

                let smin_i = ((surface_y - self.bmin.y) / self.ch).floor() as i32;
                let smax_i = smin_i + 1;

                let smin = smin_i.max(0) as u16;
                let smax = (smax_i.max(smin_i + 1) as u16).max(smin + 1);

                self.add_span(ix, iz, smin, smax, area, merge);
            }
        }
    }

    /// Filter 1: Low-hanging walkable obstacles.
    ///
    /// If a non-walkable span sits just above a walkable span within
    /// `walkable_climb` voxels, mark it walkable too (lets agents step up).
    pub fn filter_low_hanging_walkable_obstacles(&mut self, walkable_climb: u16) {
        for z in 0..self.height {
            for x in 0..self.width {
                let idx = self.cell_index(x, z);
                let mut curr = self.cells[idx];
                let mut prev_smax: Option<u16> = None;

                while curr != NULL_SPAN {
                    let s = self.spans[curr as usize];
                    if s.area == 0 {
                        // Non-walkable. Check if there's a walkable span below.
                        if let Some(prev_top) = prev_smax {
                            let gap = s.smin as i32 - prev_top as i32;
                            if gap >= 0 && (gap as u16) <= walkable_climb {
                                // The gap between the walkable surface and this
                                // obstacle bottom is small enough to step over.
                                self.spans[curr as usize].area = 1;
                            }
                        }
                    }
                    prev_smax = Some(s.smax);
                    curr = s.next;
                }
            }
        }
    }

    /// Filter 2: Ledge / steep-slope detection.
    ///
    /// For each walkable span, check all four neighbours. If any neighbour
    /// is missing or the floor-height difference exceeds `walkable_climb`,
    /// mark the span non-walkable (it's a ledge / too steep).
    pub fn filter_ledge_spans(&mut self, walkable_height: u16, walkable_climb: u16) {
        let dirs: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
        let mut to_remove = Vec::new();

        for z in 0..self.height {
            for x in 0..self.width {
                let idx = self.cell_index(x, z);
                let mut curr = self.cells[idx];
                while curr != NULL_SPAN {
                    // Copy data BEFORE any mutable borrow.
                    let s_area = self.spans[curr as usize].area;
                    let _s_smin = self.spans[curr as usize].smin;
                    let s_smax = self.spans[curr as usize].smax;
                    let s_next = self.spans[curr as usize].next;

                    if s_area == 0 {
                        curr = s_next;
                        continue;
                    }

                    let mut is_ledge = false;

                    for &(dx, dz) in &dirs {
                        let nx = x as i32 + dx;
                        let nz = z as i32 + dz;
                        if nx < 0 || nx >= self.width as i32 || nz < 0 || nz >= self.height as i32 {
                            continue; // world edge — not a ledge
                        }
                        let nidx = self.cell_index(nx as u32, nz as u32);
                        if self.cells[nidx] == NULL_SPAN {
                            continue; // empty neighbour — not a ledge
                        }
                        let mut ncurr = self.cells[nidx];
                        let mut found_connection = false;

                        while ncurr != NULL_SPAN {
                            let ns_smax = self.spans[ncurr as usize].smax;
                            let ns_next = self.spans[ncurr as usize].next;

                            // Floor-height difference (agent walks on top of solid).
                            let floor_diff = (ns_smax as i32 - s_smax as i32).abs();
                            if floor_diff as u16 > walkable_climb {
                                ncurr = ns_next;
                                continue;
                            }

                            // Headroom above this span: distance to next solid (or infinity).
                            let headroom_n = if ns_next != NULL_SPAN {
                                self.spans[ns_next as usize].smin.saturating_sub(ns_smax)
                            } else {
                                u16::MAX
                            };

                            // Check that BOTH this span and neighbour have enough headroom.
                            let headroom_s = if s_next != NULL_SPAN {
                                self.spans[s_next as usize].smin.saturating_sub(s_smax)
                            } else {
                                u16::MAX
                            };

                            if headroom_n >= walkable_height && headroom_s >= walkable_height {
                                found_connection = true;
                                break;
                            }
                            ncurr = ns_next;
                        }

                        if !found_connection {
                            is_ledge = true;
                            break;
                        }
                    }

                    if is_ledge {
                        to_remove.push(curr);
                    }

                    curr = s_next;
                }
            }
        }

        // Apply removals.
        for &span_idx in &to_remove {
            let si = span_idx as usize;
            if si < self.spans.len() {
                self.spans[si].area = 0;
            }
        }
    }

    /// Filter 3: Low headroom.
    ///
    /// If the open space above a walkable span is less than
    /// `walkable_height`, mark the span non-walkable.
    pub fn filter_walkable_low_height_spans(&mut self, walkable_height: u16) {
        let mut to_remove = Vec::new();
        for z in 0..self.height {
            for x in 0..self.width {
                let idx = self.cell_index(x, z);
                let mut curr = self.cells[idx];

                while curr != NULL_SPAN {
                    let s_area = self.spans[curr as usize].area;
                    let s_smax = self.spans[curr as usize].smax;
                    let s_next = self.spans[curr as usize].next;

                    if s_area == 0 {
                        curr = s_next;
                        continue;
                    }

                    let headroom = if s_next != NULL_SPAN {
                        let ns_smin = self.spans[s_next as usize].smin;
                        ns_smin as i32 - s_smax as i32
                    } else {
                        i32::MAX
                    };

                    if headroom < walkable_height as i32 {
                        to_remove.push(curr);
                    }

                    curr = s_next;
                }
            }
        }
        for &span_idx in &to_remove {
            let si = span_idx as usize;
            if si < self.spans.len() {
                self.spans[si].area = 0;
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_cfg() -> NavMeshCookConfig {
        NavMeshCookConfig {
            bounds_min: Vec3::ZERO,
            bounds_max: Vec3::new(3.0, 2.0, 3.0),
            ..Default::default()
        }
    }

    #[test]
    fn heightfield_alloc() {
        let cfg = default_cfg();
        let hf = Heightfield::alloc(&cfg);
        assert_eq!(hf.width, 10);
        assert_eq!(hf.height, 10);
        assert_eq!(hf.cells.len(), 100);
        assert!(hf.spans.is_empty());
    }

    #[test]
    fn add_single_span() {
        let cfg = default_cfg();
        let mut hf = Heightfield::alloc(&cfg);
        hf.add_span(2, 3, 5, 10, 1, 1);
        let idx = hf.cells[3 * 10 + 2];
        assert_ne!(idx, NULL_SPAN);
        let s = &hf.spans[idx as usize];
        assert_eq!(s.smin, 5);
        assert_eq!(s.smax, 10);
        assert_eq!(s.area, 1);
    }

    #[test]
    fn add_merge_adjacent_spans() {
        let cfg = default_cfg();
        let mut hf = Heightfield::alloc(&cfg);
        hf.add_span(0, 0, 0, 5, 1, 2); // span 0-5
        hf.add_span(0, 0, 3, 8, 1, 2); // overlaps → should merge to 0-8
        let idx = hf.cells[0];
        let s = &hf.spans[idx as usize];
        assert_eq!(s.smin, 0);
        assert_eq!(s.smax, 8);
        assert_eq!(s.next, NULL_SPAN);
    }

    #[test]
    fn add_span_below_existing() {
        let cfg = default_cfg();
        let mut hf = Heightfield::alloc(&cfg);
        hf.add_span(0, 0, 5, 10, 1, 1); // upper
        hf.add_span(0, 0, 0, 3, 1, 1); // lower — inserts before
        let idx = hf.cells[0]; // should point to lower
        let s0 = &hf.spans[idx as usize];
        assert_eq!(s0.smin, 0);
        assert_eq!(s0.smax, 3);
        let s1 = &hf.spans[s0.next as usize];
        assert_eq!(s1.smin, 5);
        assert_eq!(s1.smax, 10);
        assert_eq!(s1.next, NULL_SPAN);
    }

    #[test]
    fn filter_low_hanging() {
        let cfg = default_cfg();
        let mut hf = Heightfield::alloc(&cfg);
        hf.add_span(0, 0, 0, 5, 1, 0); // walkable
        hf.add_span(0, 0, 7, 9, 0, 0); // non-walkable, 2 voxels above (≤ climb=2)
        hf.filter_low_hanging_walkable_obstacles(2);
        let idx = hf.cells[0];
        let s0 = &hf.spans[idx as usize];
        assert_eq!(s0.area, 1); // unchanged
        let s1 = &hf.spans[s0.next as usize];
        assert_eq!(s1.area, 1); // now walkable (low obstacle)
    }

    #[test]
    fn filter_low_headroom() {
        let cfg = default_cfg();
        let mut hf = Heightfield::alloc(&cfg);
        hf.add_span(0, 0, 0, 5, 1, 0); // walkable
        hf.add_span(0, 0, 7, 11, 0, 0); // ceiling, 2 voxels gap (< walkable_height)
        hf.filter_walkable_low_height_spans(5); // needs 5 voxels headroom
        let idx = hf.cells[0];
        let s0 = &hf.spans[idx as usize];
        assert_eq!(s0.area, 0); // became unwalkable (only 2 voxels headroom, < 5)
    }

    #[test]
    fn rasterize_flat_plane() {
        let cfg = NavMeshCookConfig {
            bounds_min: Vec3::new(-1.0, -1.0, -1.0),
            bounds_max: Vec3::new(4.0, 3.0, 4.0),
            ..Default::default()
        };
        let mut hf = Heightfield::alloc(&cfg);

        // A single flat triangle on y=0.
        let verts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 3.0),
        ];
        let idxs: Vec<u32> = (0..3).collect();
        hf.rasterize_triangles(&verts, &idxs, 1);

        // Count non-empty cells — should be > 0.
        let filled: u32 = hf
            .cells
            .iter()
            .map(|&c| if c != NULL_SPAN { 1 } else { 0 })
            .sum();
        assert!(
            filled > 0,
            "flat triangle should rasterize to at least one cell"
        );
    }
}
