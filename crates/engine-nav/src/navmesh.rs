use std::collections::HashMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Index newtypes
// ---------------------------------------------------------------------------

/// Index into [`NavMesh`] vertex storage.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct VertexIndex(pub u32);

/// Index into [`NavMesh`] polygon storage.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct PolygonIndex(pub u32);

// ---------------------------------------------------------------------------
// NavError
// ---------------------------------------------------------------------------

/// Errors produced by pathfinding on a [`NavMesh`].
#[derive(Error, Debug)]
pub enum NavError {
    /// A* search completed but no sequence of connected polygons was found.
    #[error("No path found between the specified points")]
    NoPathFound,

    /// The navigation mesh is empty, missing polygons, or structurally invalid.
    #[error("Invalid navigation mesh: {0}")]
    InvalidNavMesh(String),

    /// The agent's current position is not on any polygon and no fallback
    /// polygon was available.
    #[error("Agent is not on the navigation mesh")]
    AgentNotOnMesh,
}

// ---------------------------------------------------------------------------
// Internal polygon representation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Polygon {
    vertices: Vec<VertexIndex>,
    neighbors: Vec<PolygonIndex>,
    /// Movement-cost multiplier (1.0 = normal).
    pub(crate) cost: f32,
}

// ---------------------------------------------------------------------------
// NavMesh
// ---------------------------------------------------------------------------

/// A navigation mesh — a graph of connected convex polygons.
///
/// The mesh is stored as two flat arrays (vertices + polygons) connected by
/// index.  Polygons reference their corner vertices and track which other
/// polygons they share an edge with.
///
/// All spatial queries project onto the **XZ plane** (2D), ignoring Y for
/// connectivity purposes, per the engine's right-handed +Y-up convention
/// (FD-031).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NavMesh {
    vertices: Vec<Vec3>,
    pub(crate) polygons: Vec<Polygon>,

    // ── Spatial acceleration grid (not serialized) ───────────────────────
    /// Grid cell size in world units (default: 5.0).
    /// Smaller = finer spatial queries but more memory.
    #[serde(skip)]
    grid_cell_size: f32,
    /// Map from grid cell `(cell_x, cell_z)` → polygon indices whose AABB
    /// overlaps that cell.  Built lazily; rebuilt by
    /// [`rebuild_spatial_grid`](Self::rebuild_spatial_grid).
    #[serde(skip)]
    spatial_grid: HashMap<(i32, i32), Vec<PolygonIndex>>,
}

impl NavMesh {
    /// Create an empty navigation mesh with default grid cell size (5.0).
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            polygons: Vec::new(),
            grid_cell_size: 5.0,
            spatial_grid: HashMap::new(),
        }
    }

    /// Set the spatial grid cell size.  Call before adding polygons or
    /// call [`rebuild_spatial_grid`](Self::rebuild_spatial_grid) afterwards.
    pub fn set_grid_cell_size(&mut self, size: f32) {
        self.grid_cell_size = size.max(0.1);
    }

    /// Add a vertex and return its index.
    pub fn add_vertex(&mut self, position: Vec3) -> VertexIndex {
        let idx = self.vertices.len() as u32;
        self.vertices.push(position);
        VertexIndex(idx)
    }

    /// Convert a world position to grid cell coordinates.
    fn pos_to_cell(&self, p: Vec3) -> (i32, i32) {
        (
            (p.x / self.grid_cell_size).floor() as i32,
            (p.z / self.grid_cell_size).floor() as i32,
        )
    }

    /// Compute the axis-aligned bounding box of a polygon in the XZ plane.
    fn poly_aabb(&self, verts: &[VertexIndex]) -> Option<(f32, f32, f32, f32)> {
        let mut iter = verts.iter().filter_map(|vi| self.vertex(*vi).copied());
        let first = iter.next()?;
        let (mut min_x, mut max_x, mut min_z, mut max_z) =
            (first.x, first.x, first.z, first.z);
        for v in iter {
            if v.x < min_x { min_x = v.x; }
            if v.x > max_x { max_x = v.x; }
            if v.z < min_z { min_z = v.z; }
            if v.z > max_z { max_z = v.z; }
        }
        Some((min_x, max_x, min_z, max_z))
    }

    /// Rebuild the spatial acceleration grid from scratch.
    /// Call this after loading a serialized NavMesh or after changing
    /// `grid_cell_size`.
    pub fn rebuild_spatial_grid(&mut self) {
        self.spatial_grid.clear();
        if self.grid_cell_size <= 0.0 { return; }
        for (i, poly) in self.polygons.iter().enumerate() {
            let Some((min_x, max_x, min_z, max_z)) = self.poly_aabb(&poly.vertices) else { continue; };
            let cell_min = self.pos_to_cell(Vec3::new(min_x, 0.0, min_z));
            let cell_max = self.pos_to_cell(Vec3::new(max_x, 0.0, max_z));
            for cz in cell_min.1..=cell_max.1 {
                for cx in cell_min.0..=cell_max.0 {
                    self.spatial_grid
                        .entry((cx, cz))
                        .or_default()
                        .push(PolygonIndex(i as u32));
                }
            }
        }
    }

    /// Return the grid cell candidates for a point query.
    /// Includes the cell containing the point and its 8 neighbours (3×3
    /// region) to catch edge cases.
    fn query_cells(&self, point: Vec3) -> Vec<PolygonIndex> {
        let (cx, cz) = self.pos_to_cell(point);
        let mut seen_set = std::collections::HashSet::new();
        let mut seen = Vec::new();
        for dz in -1..=1 {
            for dx in -1..=1 {
                let key = (cx + dx, cz + dz);
                if let Some(polys) = self.spatial_grid.get(&key) {
                    for &pi in polys {
                        if seen_set.insert(pi) {
                            seen.push(pi);
                        }
                    }
                }
            }
        }
        seen
    }

    /// Add a convex polygon defined by an ordered list of vertex indices.
    ///
    /// The polygon's *neighbors* are detected automatically: any existing
    /// polygon that shares at least 2 vertices with this one is considered
    /// adjacent.  The new polygon records those neighbors, and the existing
    /// polygons are updated symmetrically.  The spatial grid is also updated.
    ///
    /// `cost` — movement-cost multiplier (≥ 0).  1.0 is normal terrain;
    /// values > 1 make the polygon more expensive for A*.
    pub fn add_polygon(&mut self, vertices: &[VertexIndex], cost: f32) -> PolygonIndex {
        let idx = self.polygons.len() as u32;

        // Detect neighbors by shared-edge (≥ 2 shared vertices).
        let mut neighbors = Vec::new();
        for (i, existing) in self.polygons.iter().enumerate() {
            let shared = vertices
                .iter()
                .filter(|v| existing.vertices.contains(v))
                .count();
            if shared >= 2 {
                neighbors.push(PolygonIndex(i as u32));
            }
        }

        // Record the new polygon.
        self.polygons.push(Polygon {
            vertices: vertices.to_vec(),
            neighbors: neighbors.clone(),
            cost,
        });

        // Symmetrically update existing neighbours.
        for &n in &neighbors {
            if let Some(p) = self.polygons.get_mut(n.0 as usize) {
                if !p.neighbors.contains(&PolygonIndex(idx)) {
                    p.neighbors.push(PolygonIndex(idx));
                }
            }
        }

        // Update spatial grid for the new polygon.
        let pi = PolygonIndex(idx);
        if let Some((min_x, max_x, min_z, max_z)) = self.poly_aabb(vertices) {
            let cell_min = self.pos_to_cell(Vec3::new(min_x, 0.0, min_z));
            let cell_max = self.pos_to_cell(Vec3::new(max_x, 0.0, max_z));
            for cz in cell_min.1..=cell_max.1 {
                for cx in cell_min.0..=cell_max.0 {
                    self.spatial_grid
                        .entry((cx, cz))
                        .or_default()
                        .push(pi);
                }
            }
        }

        pi
    }

    /// Number of vertices in the mesh.
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Number of polygons in the mesh.
    pub fn polygon_count(&self) -> usize {
        self.polygons.len()
    }

    /// Compute the centroid of a polygon (average of its vertex positions).
    /// Returns `None` if the polygon index is out of range or has no vertices.
    pub fn polygon_center(&self, poly: PolygonIndex) -> Option<Vec3> {
        let polygon = self.polygons.get(poly.0 as usize)?;
        if polygon.vertices.is_empty() {
            return None;
        }
        let mut sum = Vec3::ZERO;
        for vi in &polygon.vertices {
            if let Some(v) = self.vertices.get(vi.0 as usize) {
                sum += *v;
            }
        }
        Some(sum / polygon.vertices.len() as f32)
    }

    /// Find the first polygon whose **XZ projection** contains `point`.
    ///
    /// Uses the spatial acceleration grid to narrow candidates, then performs
    /// a convex-point test (signed cross product against every edge).
    /// If the point is on an edge or vertex it is considered inside.
    pub fn find_polygon_containing(&self, point: Vec3) -> Option<PolygonIndex> {
        let px = point.x;
        let pz = point.z;

        // Try spatially indexed lookup first (3×3 neighbourhood).
        if !self.spatial_grid.is_empty() {
            for &pi in &self.query_cells(point) {
                if let Some(polygon) = self.polygons.get(pi.0 as usize) {
                    if polygon.vertices.len() < 3 { continue; }
                    if point_in_convex_polygon_xz(px, pz, &polygon.vertices, &self.vertices) {
                        return Some(pi);
                    }
                }
            }
            return None; // not found in any nearby cell
        }

        // Fallback: linear scan (used when grid is empty / not built).
        for (i, polygon) in self.polygons.iter().enumerate() {
            if polygon.vertices.len() < 3 { continue; }
            if point_in_convex_polygon_xz(px, pz, &polygon.vertices, &self.vertices) {
                return Some(PolygonIndex(i as u32));
            }
        }

        None
    }

    /// Return the polygon whose center is nearest (by XZ distance) to `point`.
    ///
    /// Always returns a valid index; panics only if the mesh has no polygons.
    /// Uses the spatial grid to accelerate the search when available.
    pub fn find_nearest_polygon(&self, point: Vec3) -> PolygonIndex {
        let mut best_dist_sq = f32::MAX;
        let mut best = PolygonIndex(0);

        // Helper to check candidate polygons.
        let mut check = |pi: PolygonIndex| {
            if let Some(center) = self.polygon_center(pi) {
                let dx = center.x - point.x;
                let dz = center.z - point.z;
                let dist_sq = dx * dx + dz * dz;
                if dist_sq < best_dist_sq {
                    best_dist_sq = dist_sq;
                    best = pi;
                }
            }
        };

        if !self.spatial_grid.is_empty() {
            // Search outward in expanding rings around the query point.
            let (cx, cz) = self.pos_to_cell(point);
            let max_r = 8i32;
            let mut found_any = false;
            for radius in 0i32..=max_r {
                for dz in -radius..=radius {
                    for dx in -radius..=radius {
                        if dx.abs() != radius && dz.abs() != radius { continue; }
                        if let Some(polys) = self.spatial_grid.get(&(cx + dx, cz + dz)) {
                            for &pi in polys {
                                check(pi);
                                found_any = true;
                            }
                        }
                    }
                }
                if found_any && radius > 0 {
                    break;
                }
            }
            // Fallback: if ring search found nothing, scan all polygons.
            if !found_any {
                for i in 0..self.polygons.len() {
                    check(PolygonIndex(i as u32));
                }
            }
        } else {
            // Fallback: linear scan (grid is empty / not built).
            for i in 0..self.polygons.len() {
                check(PolygonIndex(i as u32));
            }
        }

        best
    }

    /// Return the list of polygons adjacent to `poly`.
    pub fn polygon_neighbors(&self, poly: PolygonIndex) -> Vec<PolygonIndex> {
        self.polygons
            .get(poly.0 as usize)
            .map(|p| p.neighbors.clone())
            .unwrap_or_default()
    }

    // ── Debug / FFI accessors ──────────────────────────────────────────────

    /// All vertices in the mesh.
    pub fn vertices(&self) -> &[Vec3] {
        &self.vertices
    }

    /// Get a vertex position by index.
    pub fn vertex(&self, idx: VertexIndex) -> Option<&Vec3> {
        self.vertices.get(idx.0 as usize)
    }

    /// The vertex indices of a polygon, in winding order.
    pub fn polygon_vertex_indices(&self, idx: PolygonIndex) -> Option<&[VertexIndex]> {
        self.polygons.get(idx.0 as usize).map(|p| p.vertices.as_slice())
    }

    /// The cost multiplier of a polygon (1.0 = normal).
    pub fn polygon_cost(&self, idx: PolygonIndex) -> f32 {
        self.polygons
            .get(idx.0 as usize)
            .map(|p| p.cost)
            .unwrap_or(1.0)
    }
}

impl Default for NavMesh {
    fn default() -> Self {
        Self::new()
    }
}

// ── Free helpers ─────────────────────────────────────────────────────────────

/// Test whether `(px, pz)` is inside a convex polygon (XZ projection).
/// Returns `true` if point is on an edge or vertex.
fn point_in_convex_polygon_xz(
    px: f32, pz: f32,
    verts: &[VertexIndex],
    all_verts: &[Vec3],
) -> bool {
    let n = verts.len();
    if n < 3 { return false; }
    let mut positive = false;
    let mut negative = false;
    for j in 0..n {
        let a = match all_verts.get(verts[j].0 as usize) { Some(v) => v, None => return false };
        let b = match all_verts.get(verts[(j + 1) % n].0 as usize) { Some(v) => v, None => return false };
        let cross = (b.x - a.x) * (pz - a.z) - (b.z - a.z) * (px - a.x);
        if cross > 0.0 { positive = true; }
        else if cross < 0.0 { negative = true; }
        if positive && negative { return false; }
    }
    true
}
