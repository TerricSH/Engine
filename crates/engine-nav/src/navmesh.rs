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

    // ── Quantized BVH for spatial acceleration (not serialized) ──────────
    /// Quantized AABB tree for O(log n) polygon queries.
    /// Built by [`rebuild_bvh`](Self::rebuild_bvh); not serialized because
    /// it can be rebuilt cheaply from `polygons` on deserialization.
    #[serde(skip)]
    bvh_nodes: Vec<BvhNode>,
    /// World-space AABB of the entire mesh (used for quantization).
    #[serde(skip)]
    bvh_world_min: Vec3,
    #[serde(skip)]
    bvh_world_max: Vec3,
    #[serde(skip)]
    bvh_qfac: f32, // quantization scale factor
}

/// A single node in the quantized BVH.
///
/// Layout mirrors Detour's `dtBVNode`:
/// - `bmin`/`bmax`: quantised AABB (each axis 0..65535).
/// - `index`: if ≥ 0 → leaf, this is the polygon index.
///           if < 0  → internal node, `-index` is the *escape offset*
///             to skip this node's subtree during linear traversal.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct BvhNode {
    bmin: [u16; 3],
    bmax: [u16; 3],
    index: i32,
}

/// Internal entry used during BVH construction.
struct BvhEntry {
    idx: u32,
    bx: u16, by: u16, bz: u16,
    ex: u16, ey: u16, ez: u16,
}

impl NavMesh {
    /// Create an empty navigation mesh.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            polygons: Vec::new(),
            bvh_nodes: Vec::new(),
            bvh_world_min: Vec3::ZERO,
            bvh_world_max: Vec3::ZERO,
            bvh_qfac: 1.0,
        }
    }

    /// Add a vertex and return its index.
    pub fn add_vertex(&mut self, position: Vec3) -> VertexIndex {
        let idx = self.vertices.len() as u32;
        self.vertices.push(position);
        VertexIndex(idx)
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

    // ── BVH construction ─────────────────────────────────────────────────

    /// Rebuild the quantised BVH from all polygons.
    ///
    /// Must be called after loading a serialised `NavMesh` or after adding
    /// polygons incrementally (the BVH is not updated per-`add_polygon`).
    pub fn rebuild_bvh(&mut self) {
        self.bvh_nodes.clear();
        let n = self.polygons.len();
        if n == 0 { return; }

        // 1. Compute world AABB of entire mesh (for quantisation).
        let mut world_min = Vec3::splat(f32::MAX);
        let mut world_max = Vec3::splat(f32::MIN);
        let mut poly_boxes: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(n);

        for poly in &self.polygons {
            if let Some((min_x, max_x, min_z, max_z)) = self.poly_aabb(&poly.vertices) {
                if min_x < world_min.x { world_min.x = min_x; }
                if min_z < world_min.z { world_min.z = min_z; }
                if max_x > world_max.x { world_max.x = max_x; }
                if max_z > world_max.z { world_max.z = max_z; }
                poly_boxes.push((min_x, max_x, min_z, max_z));
            } else {
                poly_boxes.push((0.0, 0.0, 0.0, 0.0));
            }
        }

        // 2. Compute quantisation factor.
        // qfac maps world-unit distance to u16 range (0…65535).
        let span_x = (world_max.x - world_min.x).max(0.01);
        let span_z = (world_max.z - world_min.z).max(0.01);
        let span = span_x.max(span_z);
        self.bvh_qfac = 65535.0 / span;
        self.bvh_world_min = world_min;
        self.bvh_world_max = world_max;

        // 3. Build a list of quantised AABBs.
        let q = self.bvh_qfac;
        let ox = world_min.x;
        let oz = world_min.z;
        let oy = world_min.y;

        let mut entries: Vec<BvhEntry> = poly_boxes.iter().enumerate().map(|(i, &(min_x, max_x, min_z, max_z))| {
            let qx = |v: f32| ((v - ox) * q).clamp(0.0, 65535.0) as u16;
            let qz = |v: f32| ((v - oz) * q).clamp(0.0, 65535.0) as u16;
            let qy = |v: f32| ((v - oy) * q).clamp(0.0, 65535.0) as u16;
            BvhEntry {
                idx: i as u32,
                bx: qx(min_x), by: qy(0.0), bz: qz(min_z),
                ex: qx(max_x), ey: qy(0.0), ez: qz(max_z),
            }
        }).collect();

        // 4. Sort by X midpoint (simple SAH-like heuristic in 2D).
        entries.sort_by_key(|e| e.bx);

        // 5. Build nodes bottom-up, then flatten with escape offsets.
        let nodes = Self::build_bvh_recursive(&entries, 0, entries.len());
        self.bvh_nodes = nodes;
    }

    /// Recursively build BVH nodes. Returns a flat list with Detour-style
    /// escape offsets: internal node's `index` is `-escape_count`.
    fn build_bvh_recursive(entries: &[BvhEntry], lo: usize, hi: usize) -> Vec<BvhNode> {
        let mut nodes = Vec::new();
        let count = hi - lo;

        if count == 0 { return nodes; }

        // Compute AABB of the range.
        let mut bmin = [u16::MAX; 3];
        let mut bmax = [0u16; 3];
        for e in &entries[lo..hi] {
            bmin[0] = bmin[0].min(e.bx);
            bmin[1] = bmin[1].min(e.by);
            bmin[2] = bmin[2].min(e.bz);
            bmax[0] = bmax[0].max(e.ex);
            bmax[1] = bmax[1].max(e.ey);
            bmax[2] = bmax[2].max(e.ez);
        }

        if count == 1 {
            // Leaf node.
            nodes.push(BvhNode {
                bmin, bmax,
                index: entries[lo].idx as i32,
            });
            return nodes;
        }

        // Internal node: split at midpoint of the longest axis.
        let axis = if bmax[0] - bmin[0] >= bmax[2] - bmin[2] { 0usize } else { 2usize };
        let mid_val = (bmin[axis] as u32 + bmax[axis] as u32) / 2;

        // Find split point.
        let split = match entries[lo..hi].binary_search_by(|e| {
            let center = match axis {
                0 => (e.bx as u32 + e.ex as u32) / 2,
                _ => (e.bz as u32 + e.ez as u32) / 2,
            };
            center.cmp(&mid_val)
        }) {
            Ok(p) | Err(p) => lo + p,
        };

        let split = if split == lo || split == hi {
            // All on one side — split in the middle.
            lo + count / 2
        } else {
            split
        };

        // Build children.
        let left = Self::build_bvh_recursive(entries, lo, split);
        let right = Self::build_bvh_recursive(entries, split, hi);

        let left_count = left.len();
        let right_count = right.len();

        // Internal node: escape offset = -(total children + 1 (self)).
        // This is the Detour convention: when `index < 0`, skip
        // `-index` nodes (including self) to reach the next sibling.
        // For an internal node, the escape jumps over both children.
        let escape = -(1i32 + left_count as i32 + right_count as i32);

        let mut result = Vec::with_capacity(1 + left_count + right_count);
        result.push(BvhNode {
            bmin, bmax,
            index: escape,
        });
        result.extend(left);
        result.extend(right);
        result
    }

    // ── BVH queries ──────────────────────────────────────────────────────

    /// BVH linear traversal: collect all polygon indices whose AABB
    /// overlaps the query point (XZ plane).
    fn bvh_query(&self, px: f32, pz: f32) -> Vec<PolygonIndex> {
        if self.bvh_nodes.is_empty() {
            // Fallback: linear scan.
            return (0..self.polygons.len() as u32).map(PolygonIndex).collect();
        }

        let q = self.bvh_qfac;
        let ox = self.bvh_world_min.x;
        let oz = self.bvh_world_min.z;

        // Quantize query point.
        let qpx = ((px - ox) * q).clamp(0.0, 65535.0) as u16;
        let qpz = ((pz - oz) * q).clamp(0.0, 65535.0) as u16;
        let qpy = 0u16; // Y ignored for XZ queries

        let mut results = Vec::new();
        let mut i = 0usize;
        while i < self.bvh_nodes.len() {
            let node = &self.bvh_nodes[i];
            // Overlap test (point in AABB).
            if qpx >= node.bmin[0] && qpx <= node.bmax[0]
                && qpz >= node.bmin[2] && qpz <= node.bmax[2]
                && qpy >= node.bmin[1] && qpy <= node.bmax[1]
            {
                if node.index >= 0 {
                    // Leaf.
                    results.push(PolygonIndex(node.index as u32));
                    i += 1;
                } else {
                    // Internal — descend into children.
                    i += 1;
                }
            } else {
                // No overlap — escape subtree.
                if node.index >= 0 {
                    i += 1; // leaf, just skip
                } else {
                    // Jump over the entire subtree.
                    let skip = (-node.index) as usize;
                    i = i.checked_add(skip).unwrap_or(self.bvh_nodes.len());
                }
            }
        }
        results
    }

    /// Add a convex polygon defined by an ordered list of vertex indices.
    ///
    /// The polygon's *neighbors* are detected automatically: any existing
    /// polygon that shares at least 2 vertices with this one is considered
    /// adjacent.  The new polygon records those neighbors, and the existing
    /// polygons are updated symmetrically.
    ///
    /// `cost` — movement-cost multiplier (≥ 0).  1.0 is normal terrain;
    /// values > 1 make the polygon more expensive for A*.
    ///
    /// **Note**: after adding all polygons, call [`rebuild_bvh`](Self::rebuild_bvh)
    /// to build the spatial acceleration structure.
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

        PolygonIndex(idx)
    }

    /// Rebuild the spatial acceleration structure (BVH).
    ///
    /// Call this after adding all polygons or after deserialising a `NavMesh`.
    /// This replaces the old `rebuild_spatial_grid`.
    pub fn rebuild_spatial_grid(&mut self) {
        self.rebuild_bvh();
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
    /// Uses the BVH to narrow candidates, then performs a convex-point test
    /// (signed cross product against every edge).
    /// If the point is on an edge or vertex it is considered inside.
    pub fn find_polygon_containing(&self, point: Vec3) -> Option<PolygonIndex> {
        let px = point.x;
        let pz = point.z;

        let candidates = self.bvh_query(px, pz);
        for &pi in &candidates {
            if let Some(polygon) = self.polygons.get(pi.0 as usize) {
                if polygon.vertices.len() < 3 { continue; }
                if point_in_convex_polygon_xz(px, pz, &polygon.vertices, &self.vertices) {
                    return Some(pi);
                }
            }
        }
        None
    }

    /// Return the polygon whose center is nearest (by XZ distance) to `point`.
    ///
    /// Always returns a valid index; panics only if the mesh has no polygons.
    pub fn find_nearest_polygon(&self, point: Vec3) -> PolygonIndex {
        let candidates = self.bvh_query(point.x, point.z);
        let mut best_dist_sq = f32::MAX;
        let mut best = PolygonIndex(0);

        for &pi in &candidates {
            if let Some(center) = self.polygon_center(pi) {
                let dx = center.x - point.x;
                let dz = center.z - point.z;
                let dist_sq = dx * dx + dz * dz;
                if dist_sq < best_dist_sq {
                    best_dist_sq = dist_sq;
                    best = pi;
                }
            }
        }

        // Fallback: if BVH returned nothing (empty mesh), linear scan.
        if best_dist_sq == f32::MAX {
            for i in 0..self.polygons.len() {
                if let Some(center) = self.polygon_center(PolygonIndex(i as u32)) {
                    let dx = center.x - point.x;
                    let dz = center.z - point.z;
                    let dist_sq = dx * dx + dz * dz;
                    if dist_sq < best_dist_sq {
                        best_dist_sq = dist_sq;
                        best = PolygonIndex(i as u32);
                    }
                }
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
