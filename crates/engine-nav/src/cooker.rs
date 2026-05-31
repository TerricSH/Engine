//! Navmesh baking — entry point for the new Recast-style pipeline.
//!
//! This module wraps the internal `cook::` pipeline stages and exposes a
//! simple `NavMeshCooker` struct that accepts triangle soup, a config, and
//! returns a [`NavMesh`] suitable for pathfinding.
//!
//! The pipeline:
//! ```text
//! triangles + config
//!   → Heightfield::rasterize_triangles  (voxelisation)
//!   → filter_*                           (walkability filters)
//!   → CompactHeightfield::build          (compact representation)
//!   → distance field + erosion           (agent-radius margin)
//!   → build_regions                      (watershed partitioning)
//!   → build_contours                     (region boundary extraction)
//!   → build_poly_mesh                    (ear-clip + convex merge)
//!   → polymesh_to_navmesh               (output conversion)
//! ```

use crate::cook::config::CookError;
pub use crate::cook::config::NavMeshCookConfig;
use crate::cook::{compact, contour, convert, heightfield, polymesh, region};
use crate::navmesh::NavMesh;
use glam::Vec3;

/// Cooks triangle geometry into a navigation mesh using the full Recast-style
/// voxelisation pipeline.
///
/// # Example
///
/// ```ignore
/// use engine_nav::{NavMeshCooker, NavMeshCookConfig};
///
/// let cooker = NavMeshCooker::new();
/// let mut cfg = NavMeshCookConfig::default();
/// cfg.bounds_min = Vec3::new(-10.0, -1.0, -10.0);
/// cfg.bounds_max = Vec3::new(10.0, 5.0, 10.0);
///
/// let navmesh = cooker.bake(&vertices, &indices, &cfg).unwrap();
/// ```
#[derive(Clone, Debug)]
pub struct NavMeshCooker;

impl NavMeshCooker {
    pub fn new() -> Self {
        Self
    }

    /// Bake a navigation mesh from triangle soup.
    ///
    /// `vertices` — flat array of vertex positions.
    /// `indices` — triangle index triplets into `vertices`.
    /// `config` — baking parameters (cell size, agent radius, etc.).
    pub fn bake(
        &self,
        vertices: &[Vec3],
        indices: &[u32],
        config: &NavMeshCookConfig,
    ) -> Result<NavMesh, CookError> {
        config.validate().map_err(CookError::InvalidConfig)?;

        // 1. Voxelise.
        let mut hf = heightfield::Heightfield::alloc(config);
        hf.rasterize_triangles(vertices, indices, 1);
        // 2. Walkability filters.
        let climb_vox = config.walkable_climb_voxels();
        let height_vox = config.walkable_height_voxels();
        hf.filter_low_hanging_walkable_obstacles(climb_vox);
        hf.filter_ledge_spans(height_vox, climb_vox);
        hf.filter_walkable_low_height_spans(height_vox);

        // 3. Compact + distance field + erosion.
        let mut chf =
            compact::CompactHeightfield::build_from_heightfield(&hf, height_vox, climb_vox);
        chf.build_distance_field();

        let radius_cells = config.walkable_radius_cells();
        if radius_cells > 0 {
            chf.erode_walkable_area(radius_cells);
        }

        // 4. Region partitioning.

        let _num_reg =
            region::build_regions(&mut chf, config.min_region_area, config.merge_region_area)
                .map_err(|_| CookError::RegionGenerationFailed)?;

        // 5. Contours.
        let cs =
            contour::build_contours(&chf, config.max_simplification_error, config.max_edge_len)
                .map_err(|e| CookError::ContourGenerationFailed(e.to_string()))?;

        // 6. Polygon mesh.
        let pm = polymesh::build_poly_mesh(&cs, config.max_verts_per_poly)
            .map_err(|e| CookError::PolyMeshGenerationFailed(e.to_string()))?;

        // 7. Convert to NavMesh.
        Ok(convert::polymesh_to_navmesh(&pm))
    }
}

impl Default for NavMeshCooker {
    fn default() -> Self {
        Self::new()
    }
}

// ── Re-export config for convenience ──────────────────────────────────────────

/// Alias for backward compatibility.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bake_empty_geometry() {
        let cooker = NavMeshCooker::new();
        let cfg = NavMeshCookConfig {
            bounds_min: Vec3::ZERO,
            bounds_max: Vec3::new(3.0, 2.0, 3.0),
            ..Default::default()
        };
        let result = cooker.bake(&[], &[], &cfg);
        assert!(result.is_err()); // no walkable surfaces
    }

    #[test]
    fn bake_flat_ground_produces_mesh() {
        let cooker = NavMeshCooker::new();
        let cfg = NavMeshCookConfig {
            bounds_min: Vec3::new(-2.0, -1.0, -2.0),
            bounds_max: Vec3::new(12.0, 5.0, 12.0),
            walkable_height: 1.0,
            walkable_climb: 0.3,
            walkable_radius: 0.0,
            min_region_area: 2,
            merge_region_area: 4,
            cell_size: 0.5,
            cell_height: 0.25,
            max_simplification_error: 2.0,
            ..Default::default()
        };

        let verts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 3.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 3.0),
            Vec3::new(0.0, 0.0, 3.0),
        ];
        let idxs: Vec<u32> = (0..6).collect();
        let result = cooker.bake(&verts, &idxs, &cfg);
        assert!(result.is_ok(), "bake failed: {:?}", result.err());
        let nm = result.unwrap();
        assert!(nm.polygon_count() > 0, "navmesh should have polygons");
    }
}
