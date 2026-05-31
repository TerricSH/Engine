//! Navigation mesh baking configuration.
//!
//! Mirrors the same parameters as Recast's `rcConfig`, controlling how input
//! triangle geometry is voxelised, filtered, partitioned, and finally
//! converted into convex navigation polygons.

use glam::Vec3;

/// Baking configuration that controls every stage of the navmesh pipeline.
#[derive(Clone, Debug)]
pub struct NavMeshCookConfig {
    /// XZ cell size in world units (default: 0.3).
    pub cell_size: f32,
    /// Y cell height in world units (default: 0.2).
    pub cell_height: f32,
    /// Maximum walkable slope in degrees (default: 45).
    pub walkable_slope: f32,
    /// Minimum headroom in world units (default: 2.0).
    pub walkable_height: f32,
    /// Maximum step-up height in world units (default: 0.5).
    pub walkable_climb: f32,
    /// Agent radius for erosion in world units (default: 0.3).
    pub walkable_radius: f32,
    /// Minimum region area in cells² (default: 8).
    pub min_region_area: u32,
    /// Merge small regions below this size in cells² (default: 20).
    pub merge_region_area: u32,
    /// Maximum contour edge length in cells (default: 12, 0 = unlimited).
    pub max_edge_len: u32,
    /// Douglas–Peucker simplification tolerance in cells (default: 1.3).
    pub max_simplification_error: f32,
    /// Max vertices per polygon (default: 6).
    pub max_verts_per_poly: u32,
    /// World-space AABB minimum.
    pub bounds_min: Vec3,
    /// World-space AABB maximum.
    pub bounds_max: Vec3,
}

impl Default for NavMeshCookConfig {
    fn default() -> Self {
        Self {
            cell_size: 0.3,
            cell_height: 0.2,
            walkable_slope: 45.0,
            walkable_height: 2.0,
            walkable_climb: 0.5,
            walkable_radius: 0.3,
            min_region_area: 8,
            merge_region_area: 20,
            max_edge_len: 12,
            max_simplification_error: 1.3,
            max_verts_per_poly: 6,
            bounds_min: Vec3::ZERO,
            bounds_max: Vec3::ONE,
        }
    }
}

impl NavMeshCookConfig {
    pub fn grid_size(&self) -> (u32, u32) {
        let w = ((self.bounds_max.x - self.bounds_min.x) / self.cell_size).ceil() as u32;
        let h = ((self.bounds_max.z - self.bounds_min.z) / self.cell_size).ceil() as u32;
        (w.max(1), h.max(1))
    }
    pub fn walkable_height_voxels(&self) -> u16 {
        (self.walkable_height / self.cell_height).ceil() as u16
    }
    pub fn walkable_climb_voxels(&self) -> u16 {
        (self.walkable_climb / self.cell_height).floor() as u16
    }
    pub fn walkable_radius_cells(&self) -> u32 {
        (self.walkable_radius / self.cell_size).ceil() as u32
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.cell_size <= 0.0 {
            return Err("cell_size must be positive".into());
        }
        if self.cell_height <= 0.0 {
            return Err("cell_height must be positive".into());
        }
        if self.walkable_slope < 0.0 || self.walkable_slope > 90.0 {
            return Err("walkable_slope must be in [0, 90]".into());
        }
        if self.walkable_radius < 0.0 {
            return Err("walkable_radius must be non-negative".into());
        }
        if self.walkable_height <= 0.0 {
            return Err("walkable_height must be positive".into());
        }
        if self.max_verts_per_poly < 3 {
            return Err("max_verts_per_poly must be ≥ 3".into());
        }
        let (w, h) = self.grid_size();
        if w > 1024 || h > 1024 {
            return Err(format!("grid too large: {}×{} (max 1024×1024)", w, h));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum CookError {
    InvalidConfig(String),
    NoWalkableSurfaces,
    RegionGenerationFailed,
    PolyMeshGenerationFailed(String),
    ContourGenerationFailed(String),
}

impl std::fmt::Display for CookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CookError::InvalidConfig(msg) => write!(f, "invalid config: {msg}"),
            CookError::NoWalkableSurfaces => write!(f, "no walkable surfaces found"),
            CookError::RegionGenerationFailed => write!(f, "region generation failed"),
            CookError::PolyMeshGenerationFailed(msg) => write!(f, "polymesh: {msg}"),
            CookError::ContourGenerationFailed(msg) => write!(f, "contour: {msg}"),
        }
    }
}
impl std::error::Error for CookError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_defaults() {
        let cfg = NavMeshCookConfig::default();
        assert!((cfg.cell_size - 0.3).abs() < 1e-6);
    }
    #[test]
    fn config_validate_ok() {
        let mut cfg = NavMeshCookConfig::default();
        cfg.bounds_max = Vec3::new(10.0, 5.0, 10.0);
        assert!(cfg.validate().is_ok());
    }
    #[test]
    fn config_grid_size() {
        let mut cfg = NavMeshCookConfig::default();
        cfg.bounds_min = Vec3::ZERO;
        cfg.bounds_max = Vec3::new(3.0, 1.0, 3.0);
        cfg.cell_size = 0.3;
        let (w, h) = cfg.grid_size();
        assert_eq!(w, 10);
        assert_eq!(h, 10);
    }
    #[test]
    fn config_voxel_conversions() {
        let cfg = NavMeshCookConfig {
            cell_height: 0.2,
            walkable_height: 2.0,
            walkable_climb: 0.5,
            walkable_radius: 0.3,
            cell_size: 0.3,
            ..Default::default()
        };
        assert_eq!(cfg.walkable_height_voxels(), 10);
        assert_eq!(cfg.walkable_climb_voxels(), 2);
        assert_eq!(cfg.walkable_radius_cells(), 1);
    }
}
