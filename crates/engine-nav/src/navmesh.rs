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
}

impl NavMesh {
    /// Create an empty navigation mesh.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            polygons: Vec::new(),
        }
    }

    /// Add a vertex and return its index.
    pub fn add_vertex(&mut self, position: Vec3) -> VertexIndex {
        let idx = self.vertices.len() as u32;
        self.vertices.push(position);
        VertexIndex(idx)
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
    /// Uses a convex-point test (signed cross product against every edge).
    /// If the point is on an edge or vertex it is considered inside.
    pub fn find_polygon_containing(&self, point: Vec3) -> Option<PolygonIndex> {
        let px = point.x;
        let pz = point.z;

        for (i, polygon) in self.polygons.iter().enumerate() {
            let n = polygon.vertices.len();
            if n < 3 {
                continue;
            }

            let mut positive = false;
            let mut negative = false;
            let mut valid = true;

            for j in 0..n {
                let a_idx = polygon.vertices[j];
                let b_idx = polygon.vertices[(j + 1) % n];

                let a = match self.vertices.get(a_idx.0 as usize) {
                    Some(v) => v,
                    None => {
                        valid = false;
                        break;
                    }
                };
                let b = match self.vertices.get(b_idx.0 as usize) {
                    Some(v) => v,
                    None => {
                        valid = false;
                        break;
                    }
                };

                // 2D cross product (edge × point-vertex) on XZ plane.
                let cross = (b.x - a.x) * (pz - a.z) - (b.z - a.z) * (px - a.x);

                if cross > 0.0 {
                    positive = true;
                } else if cross < 0.0 {
                    negative = true;
                }

                if positive && negative {
                    valid = false;
                    break;
                }
            }

            if valid {
                return Some(PolygonIndex(i as u32));
            }
        }

        None
    }

    /// Return the polygon whose center is nearest (by XZ distance) to `point`.
    ///
    /// Always returns a valid index; panics only if the mesh has no polygons.
    pub fn find_nearest_polygon(&self, point: Vec3) -> PolygonIndex {
        let mut best = PolygonIndex(0);
        let mut best_dist_sq = f32::MAX;

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
