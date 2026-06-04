//! NavMesh debug visualization.
//!
//! Provides [`NavMeshDebugDraw`] which implements the
//! [`DebugDrawProvider`] trait from `engine-renderer` so that
//! the navmesh polygons, computed paths, and agent positions
//! are rendered as wireframe overlays.

use crate::navmesh::{NavMesh, PolygonIndex};
use crate::pathfinding::Path;
use engine_renderer::debug_draw::{DebugDrawBuffer, DebugDrawProvider};
use glam::{Mat4, Vec3};
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Colour constants
// ---------------------------------------------------------------------------

/// Colour for walkable (cost ≈ 1.0) polygon edges — green.
const COLOR_WALKABLE: [f32; 4] = [0.0, 0.8, 0.2, 1.0];
/// Colour for polygon edges that belong to the current path — yellow.
const COLOR_PATH_POLY: [f32; 4] = [1.0, 1.0, 0.0, 1.0];
/// Colour for path waypoint arrows — cyan.
const COLOR_PATH_ARROW: [f32; 4] = [0.0, 0.8, 1.0, 1.0];
/// Colour for the agent position sphere — red.
const COLOR_AGENT: [f32; 4] = [1.0, 0.2, 0.2, 1.0];

// ---------------------------------------------------------------------------
// Internal storage
// ---------------------------------------------------------------------------

/// Per-polygon data extracted from a [`NavMesh`] for drawing.
struct PolyDraw {
    /// Indices into the debug draw's vertex list.
    vertex_indices: Vec<usize>,

}

// ---------------------------------------------------------------------------
// NavMeshDebugDraw
// ---------------------------------------------------------------------------

/// Debug visualisation for navigation meshes.
///
/// Register an instance of this type with a
/// [`DebugDrawRegistry`] to have the navmesh rendered
/// as a wireframe overlay.
///
/// # Usage
///
/// ```ignore
/// use engine_nav::debug::NavMeshDebugDraw;
/// use engine_renderer::DebugDrawRegistry;
///
/// let mut reg = DebugDrawRegistry::new();
/// reg.register(Box::new(NavMeshDebugDraw::new()));
/// ```
pub struct NavMeshDebugDraw {
    vertices: Vec<Vec3>,
    polygons: Vec<PolyDraw>,
    /// Polygon indices (in [`NavMesh`] order) that lie on the current path.
    path_polygons: HashSet<PolygonIndex>,
    path_waypoints: Vec<Vec3>,
    agent_position: Option<Vec3>,
}

impl NavMeshDebugDraw {
    /// Create an empty debug drawer (nothing drawn until data is set).
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            polygons: Vec::new(),
            path_polygons: HashSet::new(),
            path_waypoints: Vec::new(),
            agent_position: None,
        }
    }

    /// Copy navmesh geometry for debug drawing.
    ///
    /// Call this whenever the navmesh changes or once after creation.
    pub fn set_navmesh(&mut self, navmesh: &NavMesh) {
        self.vertices = navmesh.vertices().to_vec();
        self.polygons.clear();

        for i in 0..navmesh.polygon_count() {
            let idx = PolygonIndex(i as u32);
            if let Some(indices) = navmesh.polygon_vertex_indices(idx) {
                self.polygons.push(PolyDraw {
                    vertex_indices: indices.iter().map(|vi| vi.0 as usize).collect(),
                });
            }
        }

        // Discard stale path polygon markers (indices may have changed).
        self.path_polygons.clear();
        self.path_waypoints.clear();
    }

    /// Store a computed path to render as arrows.
    ///
    /// Call this whenever a new path is computed for an agent.
    /// The polygon indices from the path are used to highlight
    /// the corresponding polygons.
    pub fn set_path(&mut self, path: &Path) {
        self.path_waypoints = path.waypoints().iter().map(|wp| wp.position).collect();
        self.path_polygons = path.waypoints().iter().map(|wp| wp.polygon).collect();
    }

    /// Set the agent's world position to draw a marker sphere.
    pub fn set_agent_position(&mut self, pos: Vec3) {
        self.agent_position = Some(pos);
    }
}

impl Default for NavMeshDebugDraw {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugDrawProvider for NavMeshDebugDraw {
    fn name(&self) -> &str {
        "NavMesh"
    }

    fn populate(&self, buffer: &mut DebugDrawBuffer, _view: &Mat4, _proj: &Mat4) {
        // ── Polygon edges ─────────────────────────────────────────────
        for (i, poly) in self.polygons.iter().enumerate() {
            let poly_idx = PolygonIndex(i as u32);
            let is_path = self.path_polygons.contains(&poly_idx);
            let color = if is_path {
                COLOR_PATH_POLY
            } else {
                COLOR_WALKABLE
            };

            let n = poly.vertex_indices.len();
            if n < 2 {
                continue;
            }

            // Draw each edge of the closed polygon.
            for j in 0..n {
                let a_idx = poly.vertex_indices[j];
                let b_idx = poly.vertex_indices[(j + 1) % n];

                if a_idx < self.vertices.len() && b_idx < self.vertices.len() {
                    buffer.line(self.vertices[a_idx], self.vertices[b_idx], color);
                }
            }
        }

        // ── Path waypoint arrows ──────────────────────────────────────
        for window in self.path_waypoints.windows(2) {
            buffer.arrow(window[0], window[1], COLOR_PATH_ARROW);
        }

        // ── Agent position sphere ─────────────────────────────────────
        if let Some(pos) = self.agent_position {
            buffer.sphere_wireframe(pos, 0.3, COLOR_AGENT);
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NavMesh, PathPoint};
    use glam::Vec3;

    #[test]
    fn new_debug_draw_is_empty() {
        let dd = NavMeshDebugDraw::new();
        assert!(dd.vertices.is_empty());
        assert!(dd.polygons.is_empty());
        assert!(dd.path_waypoints.is_empty());
        assert!(dd.agent_position.is_none());
    }

    #[test]
    fn set_navmesh_populates_internal_data() {
        let mut mesh = NavMesh::new();
        let a = mesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = mesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
        let c = mesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
        mesh.add_polygon(&[a, b, c], 1.0);

        let mut dd = NavMeshDebugDraw::new();
        dd.set_navmesh(&mesh);

        assert_eq!(dd.vertices.len(), 3);
        assert_eq!(dd.polygons.len(), 1);
        assert_eq!(dd.polygons[0].vertex_indices.len(), 3);
    }

    #[test]
    fn set_path_records_waypoints_and_polygons() {
        let path = Path::new(vec![
            PathPoint {
                position: Vec3::ZERO,
                polygon: PolygonIndex(0),
            },
            PathPoint {
                position: Vec3::new(10.0, 0.0, 0.0),
                polygon: PolygonIndex(1),
            },
        ]);

        let mut dd = NavMeshDebugDraw::new();
        dd.set_path(&path);

        assert_eq!(dd.path_waypoints.len(), 2);
        assert!(dd.path_polygons.contains(&PolygonIndex(0)));
        assert!(dd.path_polygons.contains(&PolygonIndex(1)));
    }

    #[test]
    fn set_navmesh_clears_stale_path_data() {
        let mut mesh = NavMesh::new();
        let a = mesh.add_vertex(Vec3::ZERO);
        let b = mesh.add_vertex(Vec3::X);
        let c = mesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
        mesh.add_polygon(&[a, b, c], 1.0);

        let mut dd = NavMeshDebugDraw::new();
        dd.path_waypoints = vec![Vec3::new(5.0, 0.0, 0.0)];
        dd.path_polygons.insert(PolygonIndex(99));

        dd.set_navmesh(&mesh);

        assert!(dd.path_waypoints.is_empty());
        assert!(dd.path_polygons.is_empty());
    }

    #[test]
    fn populate_draws_polygon_edges() {
        let mut mesh = NavMesh::new();
        let a = mesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = mesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
        let c = mesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
        mesh.add_polygon(&[a, b, c], 1.0);

        let mut dd = NavMeshDebugDraw::new();
        dd.set_navmesh(&mesh);

        let mut buf = DebugDrawBuffer::new();
        dd.populate(&mut buf, &Mat4::IDENTITY, &Mat4::IDENTITY);

        // A triangle has 3 edges → 3 lines
        assert_eq!(buf.lines.len(), 3);
    }

    #[test]
    fn populate_draws_path_arrows() {
        let path = Path::new(vec![
            PathPoint {
                position: Vec3::ZERO,
                polygon: PolygonIndex(0),
            },
            PathPoint {
                position: Vec3::new(5.0, 0.0, 0.0),
                polygon: PolygonIndex(1),
            },
            PathPoint {
                position: Vec3::new(10.0, 0.0, 0.0),
                polygon: PolygonIndex(2),
            },
        ]);

        let mut dd = NavMeshDebugDraw::new();
        dd.set_path(&path);

        let mut buf = DebugDrawBuffer::new();
        dd.populate(&mut buf, &Mat4::IDENTITY, &Mat4::IDENTITY);

        // 3 waypoints → 2 arrows
        assert_eq!(buf.shapes.len(), 2);
    }

    #[test]
    fn populate_draws_agent_sphere() {
        let mut dd = NavMeshDebugDraw::new();
        dd.set_agent_position(Vec3::new(1.0, 2.0, 3.0));

        let mut buf = DebugDrawBuffer::new();
        dd.populate(&mut buf, &Mat4::IDENTITY, &Mat4::IDENTITY);

        assert_eq!(buf.shapes.len(), 1);
    }

    #[test]
    fn name_returns_navmesh() {
        let dd = NavMeshDebugDraw::new();
        assert_eq!(dd.name(), "NavMesh");
    }
}
