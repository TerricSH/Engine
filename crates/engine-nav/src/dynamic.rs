//! Incrementally replaceable navigation tiles for mutable runtime geometry.

use std::collections::BTreeMap;

use glam::Vec3;
use thiserror::Error;

use crate::{NavMesh, NavMeshCookConfig, NavMeshCooker};

/// Stable key for one independently rebuilt navigation tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DynamicNavTileKey {
    pub domain: u64,
    pub coordinates: [i64; 3],
}

impl DynamicNavTileKey {
    pub const fn new(domain: u64, coordinates: [i64; 3]) -> Self {
        Self {
            domain,
            coordinates,
        }
    }
}

/// Per-tile baking settings. Bounds are derived from replacement geometry.
#[derive(Clone, Debug)]
pub struct DynamicNavBuildConfig {
    pub cook: NavMeshCookConfig,
    pub bounds_padding: f32,
    /// Logical-world up direction for this tile. Planetary callers provide
    /// the radial direction; planar worlds keep +Y.
    pub up: Vec3,
}

impl Default for DynamicNavBuildConfig {
    fn default() -> Self {
        Self {
            cook: NavMeshCookConfig::default(),
            bounds_padding: 0.5,
            up: Vec3::Y,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DynamicNavTile {
    pub logical_origin: [f64; 3],
    pub revision: u64,
    pub local_bounds_min: Vec3,
    pub local_bounds_max: Vec3,
    /// World-space basis axes for nav-local X/Y/Z coordinates.
    pub world_from_nav: [Vec3; 3],
    pub navmesh: NavMesh,
}

impl DynamicNavTile {
    pub fn contains_logical_position(&self, position: [f64; 3]) -> bool {
        let local = self.logical_to_nav(position);
        (0..3).all(|axis| {
            local[axis] >= self.local_bounds_min[axis] && local[axis] <= self.local_bounds_max[axis]
        })
    }

    pub fn logical_to_nav(&self, position: [f64; 3]) -> Vec3 {
        let delta = Vec3::new(
            (position[0] - self.logical_origin[0]) as f32,
            (position[1] - self.logical_origin[1]) as f32,
            (position[2] - self.logical_origin[2]) as f32,
        );
        Vec3::new(
            delta.dot(self.world_from_nav[0]),
            delta.dot(self.world_from_nav[1]),
            delta.dot(self.world_from_nav[2]),
        )
    }

    pub fn nav_to_logical(&self, position: Vec3) -> [f64; 3] {
        let world = self.world_from_nav[0] * position.x
            + self.world_from_nav[1] * position.y
            + self.world_from_nav[2] * position.z;
        std::array::from_fn(|axis| self.logical_origin[axis] + f64::from(world[axis]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicNavTileUpdate {
    Rebuilt,
    Removed,
    Unchanged,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DynamicNavError {
    #[error("dynamic navigation vertices must be finite")]
    NonFiniteVertex,
    #[error("dynamic navigation triangle {triangle} references a missing vertex")]
    InvalidTriangleIndex { triangle: usize },
    #[error("dynamic navigation bounds padding must be finite and non-negative")]
    InvalidBoundsPadding,
    #[error("dynamic navigation up direction must be finite and non-zero")]
    InvalidUpDirection,
    #[error("dynamic navigation tile bake failed: {0}")]
    Bake(String),
    #[error("dynamic navigation bake produced an empty tile")]
    EmptyTile,
}

/// A set of independently replaceable NavMesh tiles. A failed rebuild leaves
/// the previous revision installed, so transient edit/cook failures do not
/// invalidate navigation outside the affected frame.
#[derive(Clone, Debug, Default)]
pub struct DynamicNavTileSet {
    tiles: BTreeMap<DynamicNavTileKey, DynamicNavTile>,
}

impl DynamicNavTileSet {
    pub fn rebuild(
        &mut self,
        key: DynamicNavTileKey,
        logical_origin: [f64; 3],
        revision: u64,
        vertices: &[[f32; 3]],
        triangles: &[[u32; 3]],
        config: &DynamicNavBuildConfig,
    ) -> Result<DynamicNavTileUpdate, DynamicNavError> {
        if self
            .tiles
            .get(&key)
            .is_some_and(|tile| tile.revision == revision)
        {
            return Ok(DynamicNavTileUpdate::Unchanged);
        }
        if vertices.is_empty() || triangles.is_empty() {
            self.tiles.remove(&key);
            return Ok(DynamicNavTileUpdate::Removed);
        }
        if !config.bounds_padding.is_finite() || config.bounds_padding < 0.0 {
            return Err(DynamicNavError::InvalidBoundsPadding);
        }
        if !config.up.is_finite() || config.up.length_squared() <= f32::EPSILON {
            return Err(DynamicNavError::InvalidUpDirection);
        }

        let world_vertices = vertices
            .iter()
            .copied()
            .map(Vec3::from_array)
            .collect::<Vec<_>>();
        if world_vertices.iter().any(|vertex| !vertex.is_finite()) {
            return Err(DynamicNavError::NonFiniteVertex);
        }
        let world_from_nav = navigation_basis(config.up);
        let vertices = world_vertices
            .iter()
            .map(|vertex| {
                Vec3::new(
                    vertex.dot(world_from_nav[0]),
                    vertex.dot(world_from_nav[1]),
                    vertex.dot(world_from_nav[2]),
                )
            })
            .collect::<Vec<_>>();
        let indices = triangles.iter().enumerate().try_fold(
            Vec::with_capacity(triangles.len() * 3),
            |mut out, (index, triangle)| {
                if triangle.iter().any(|vertex| {
                    usize::try_from(*vertex).map_or(true, |vertex| vertex >= vertices.len())
                }) {
                    return Err(DynamicNavError::InvalidTriangleIndex { triangle: index });
                }
                out.extend_from_slice(triangle);
                Ok(out)
            },
        )?;
        let (bounds_min, bounds_max) = geometry_bounds(&vertices, config.bounds_padding);
        let mut cook = config.cook.clone();
        cook.bounds_min = bounds_min;
        cook.bounds_max = bounds_max;
        let navmesh = NavMeshCooker::new()
            .bake(&vertices, &indices, &cook)
            .map_err(|error| DynamicNavError::Bake(error.to_string()))?;
        if navmesh.polygon_count() == 0 {
            return Err(DynamicNavError::EmptyTile);
        }
        self.tiles.insert(
            key,
            DynamicNavTile {
                logical_origin,
                revision,
                local_bounds_min: bounds_min,
                local_bounds_max: bounds_max,
                world_from_nav,
                navmesh,
            },
        );
        Ok(DynamicNavTileUpdate::Rebuilt)
    }

    pub fn remove(&mut self, key: DynamicNavTileKey) -> Option<DynamicNavTile> {
        self.tiles.remove(&key)
    }

    pub fn get(&self, key: DynamicNavTileKey) -> Option<&DynamicNavTile> {
        self.tiles.get(&key)
    }

    pub fn tile_at_logical_position(&self, position: [f64; 3]) -> Option<&DynamicNavTile> {
        self.tiles
            .values()
            .filter(|tile| tile.contains_logical_position(position))
            .max_by_key(|tile| tile.revision)
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

fn geometry_bounds(vertices: &[Vec3], padding: f32) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for vertex in vertices {
        min = min.min(*vertex);
        max = max.max(*vertex);
    }
    let padding = Vec3::splat(padding);
    (min - padding, max + padding)
}

fn navigation_basis(up: Vec3) -> [Vec3; 3] {
    let up = up.normalize();
    let reference = if up.dot(Vec3::Y).abs() > 0.9 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = up.cross(reference).normalize();
    let forward = right.cross(up).normalize();
    [right, up, forward]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_quad() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        (
            vec![
                [0.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 0.0, 4.0],
                [0.0, 0.0, 4.0],
            ],
            vec![[0, 2, 1], [0, 3, 2]],
        )
    }

    #[test]
    fn replaces_one_tile_without_touching_neighbors() {
        let (vertices, triangles) = flat_quad();
        let mut tiles = DynamicNavTileSet::default();
        let config = DynamicNavBuildConfig {
            cook: NavMeshCookConfig {
                cell_size: 0.5,
                cell_height: 0.25,
                walkable_height: 1.0,
                walkable_radius: 0.0,
                min_region_area: 1,
                merge_region_area: 1,
                ..NavMeshCookConfig::default()
            },
            bounds_padding: 0.5,
            up: Vec3::Y,
        };
        let first = DynamicNavTileKey::new(7, [0, 0, 0]);
        let second = DynamicNavTileKey::new(7, [1, 0, 0]);
        assert_eq!(
            tiles
                .rebuild(first, [1.0e12, 0.0, 0.0], 1, &vertices, &triangles, &config)
                .unwrap(),
            DynamicNavTileUpdate::Rebuilt
        );
        assert_eq!(
            tiles
                .rebuild(
                    second,
                    [1.0e12 + 4.0, 0.0, 0.0],
                    1,
                    &vertices,
                    &triangles,
                    &config
                )
                .unwrap(),
            DynamicNavTileUpdate::Rebuilt
        );
        assert_eq!(tiles.len(), 2);
        assert_eq!(
            tiles
                .rebuild(first, [1.0e12, 0.0, 0.0], 1, &vertices, &triangles, &config)
                .unwrap(),
            DynamicNavTileUpdate::Unchanged
        );
        assert_eq!(
            tiles
                .rebuild(first, [1.0e12, 0.0, 0.0], 2, &[], &[], &config)
                .unwrap(),
            DynamicNavTileUpdate::Removed
        );
        assert!(tiles.get(first).is_none());
        assert!(tiles.get(second).is_some());
    }

    #[test]
    fn failed_rebuild_retains_previous_revision() {
        let (vertices, triangles) = flat_quad();
        let mut tiles = DynamicNavTileSet::default();
        let key = DynamicNavTileKey::new(9, [-1, 2, 3]);
        let config = DynamicNavBuildConfig {
            cook: NavMeshCookConfig {
                cell_size: 0.5,
                cell_height: 0.25,
                walkable_height: 1.0,
                walkable_radius: 0.0,
                min_region_area: 1,
                merge_region_area: 1,
                ..NavMeshCookConfig::default()
            },
            bounds_padding: 0.5,
            up: Vec3::Y,
        };
        tiles
            .rebuild(key, [0.0; 3], 5, &vertices, &triangles, &config)
            .unwrap();
        let error = tiles
            .rebuild(key, [0.0; 3], 6, &vertices, &[[0, 1, 99]], &config)
            .unwrap_err();
        assert_eq!(error, DynamicNavError::InvalidTriangleIndex { triangle: 0 });
        assert_eq!(tiles.get(key).unwrap().revision, 5);
    }

    #[test]
    fn radial_up_rotates_planetary_tiles_into_nav_space() {
        let basis = navigation_basis(Vec3::X);
        assert!((basis[1] - Vec3::X).length() < 1.0e-6);
        let tile = DynamicNavTile {
            logical_origin: [1.0e12, 2.0e12, 3.0e12],
            revision: 1,
            local_bounds_min: Vec3::splat(-2.0),
            local_bounds_max: Vec3::splat(2.0),
            world_from_nav: basis,
            navmesh: NavMesh::new(),
        };
        let logical = tile.nav_to_logical(Vec3::new(1.0, 0.5, -0.25));
        let roundtrip = tile.logical_to_nav(logical);
        assert!((roundtrip - Vec3::new(1.0, 0.5, -0.25)).length() < 1.0e-3);
    }
}
