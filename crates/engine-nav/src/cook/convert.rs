//! Conversion from [`PolyMesh`] to [`NavMesh`].
//!
//! All vertices from the polygon mesh are registered in the navigation mesh,
//! and each convex polygon is added via [`NavMesh::add_polygon`], which
//! auto-detects neighbour connectivity through shared vertex pairs.

use crate::cook::polymesh::PolyMesh;
use crate::navmesh::{NavMesh, VertexIndex};

/// Convert a baked [`PolyMesh`] into a run-time [`NavMesh`].
///
/// This is the final step of the navmesh baking pipeline.  Every polygon is
/// registered with a default cost of 1.0 (normal terrain); neighbours are
/// detected automatically by [`NavMesh::add_polygon`] using the ≥2-shared-
/// vertices rule.
pub(crate) fn polymesh_to_navmesh(pm: &PolyMesh) -> NavMesh {
    let mut nav = NavMesh::new();

    // 1. Register all vertices.
    let mut vi_map: Vec<VertexIndex> = Vec::with_capacity(pm.verts.len());
    for &v in &pm.verts {
        vi_map.push(nav.add_vertex(v));
    }

    // 2. Register each polygon.
    for poly in &pm.polys {
        if poly.verts.len() < 3 {
            continue; // skip degenerate
        }
        let mapped: Vec<VertexIndex> = poly
            .verts
            .iter()
            .map(|&local_idx| {
                let idx = local_idx as usize;
                if idx < vi_map.len() {
                    vi_map[idx]
                } else {
                    // Should not happen with valid data, but guard.
                    VertexIndex(0)
                }
            })
            .collect();

        nav.add_polygon(&mapped, 1.0);
    }

    nav
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::contour::{Contour, ContourSet};
    use crate::cook::polymesh::build_poly_mesh;
    use glam::Vec3;

    /// Build a simple PolyMesh from a single square contour.
    fn square_polymesh() -> PolyMesh {
        let contour = Contour {
            verts: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 2.0),
                Vec3::new(0.0, 0.0, 2.0),
            ],
            reg: 1,
            area: 1,
        };
        let cs = ContourSet {
            conts: vec![contour],
            width: 10,
            height: 10,
            bmin: Vec3::ZERO,
            cs: 1.0,
            ch: 1.0,
            border_size: 0,
        };
        build_poly_mesh(&cs, 6).expect("square polymesh")
    }

    /// Build two disconnected square regions.
    fn two_squares_polymesh() -> PolyMesh {
        let mut cs = ContourSet {
            conts: vec![],
            width: 20,
            height: 20,
            bmin: Vec3::ZERO,
            cs: 1.0,
            ch: 1.0,
            border_size: 0,
        };
        cs.conts.push(Contour {
            verts: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 2.0),
                Vec3::new(0.0, 0.0, 2.0),
            ],
            reg: 1,
            area: 1,
        });
        cs.conts.push(Contour {
            verts: vec![
                Vec3::new(5.0, 0.0, 5.0),
                Vec3::new(7.0, 0.0, 5.0),
                Vec3::new(7.0, 0.0, 7.0),
                Vec3::new(5.0, 0.0, 7.0),
            ],
            reg: 2,
            area: 1,
        });
        build_poly_mesh(&cs, 6).expect("two squares polymesh")
    }

    #[test]
    fn single_square_roundtrip() {
        let pm = square_polymesh();
        let nav = polymesh_to_navmesh(&pm);

        // Should have 4 unique vertices and 1 polygon (merged quad).
        assert_eq!(
            nav.vertex_count(),
            4,
            "square should have 4 vertices"
        );
        assert_eq!(
            nav.polygon_count(),
            1,
            "square should produce 1 polygon"
        );

        // Polygon should have 4 vertices.
        let verts = nav.polygon_vertex_indices(crate::PolygonIndex(0)).unwrap();
        assert_eq!(verts.len(), 4, "polygon should be a quad");
    }

    #[test]
    fn two_squares_no_neighbours() {
        let pm = two_squares_polymesh();
        let nav = polymesh_to_navmesh(&pm);

        assert_eq!(nav.polygon_count(), 2, "should have 2 polygons");
        // The two squares should NOT be neighbours (they are far apart).
        assert!(
            nav.polygon_neighbors(crate::PolygonIndex(0)).is_empty(),
            "disconnected squares should have no neighbours"
        );
        assert!(
            nav.polygon_neighbors(crate::PolygonIndex(1)).is_empty(),
            "disconnected squares should have no neighbours"
        );
    }

    #[test]
    fn adjacent_squares_have_neighbour_connectivity() {
        // Two squares sharing an edge at x=[0,1]z=[0,1] and x=[1,2]z=[0,1].
        let c1 = Contour {
            verts: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            reg: 1,
            area: 1,
        };
        let c2 = Contour {
            verts: vec![
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 1.0),
            ],
            reg: 2,
            area: 1,
        };
        let cs = ContourSet {
            conts: vec![c1, c2],
            width: 10,
            height: 10,
            bmin: Vec3::ZERO,
            cs: 1.0,
            ch: 1.0,
            border_size: 0,
        };
        let pm = build_poly_mesh(&cs, 6).expect("adjacent squares");
        let nav = polymesh_to_navmesh(&pm);

        // Both should exist.
        assert_eq!(nav.polygon_count(), 2);

        // They should be neighbours (share 2 vertices at the seam).
        let n0 = nav.polygon_neighbors(crate::PolygonIndex(0));
        let n1 = nav.polygon_neighbors(crate::PolygonIndex(1));
        assert!(
            n0.contains(&crate::PolygonIndex(1)),
            "polygon 0 should neighbour polygon 1"
        );
        assert!(
            n1.contains(&crate::PolygonIndex(0)),
            "polygon 1 should neighbour polygon 0"
        );
    }

    #[test]
    fn polymesh_to_navmesh_empty_returns_empty_mesh() {
        let pm = PolyMesh {
            verts: Vec::new(),
            polys: Vec::new(),
            bmin: Vec3::ZERO,
            cs: 1.0,
            ch: 1.0,
        };
        let nav = polymesh_to_navmesh(&pm);
        assert_eq!(nav.vertex_count(), 0);
        assert_eq!(nav.polygon_count(), 0);
    }
}
