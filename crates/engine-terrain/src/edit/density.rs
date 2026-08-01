use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use engine_procgen::{Fbm2D, FbmParams, Seed, WarpParams, WarpedFbm2D};

use crate::{PlanetTerrainQuery, TerrainTopology, TerrainVolume};

/// Maximum number of lattice samples a single untrusted brush may inspect.
pub const MAX_TERRAIN_BRUSH_SAMPLES: u64 = 2_000_000;

/// Exact unedited solid/empty field corresponding to a [`TerrainVolume`].
/// Positive density is below the generated surface and negative density is
/// outside it, for both planar and cube-sphere terrain.
#[derive(Clone, Copy, Debug)]
pub enum TerrainBaseDensity {
    Planar {
        height_scale: f32,
        fbm: Fbm2D,
        warped: Option<WarpedFbm2D>,
    },
    CubeSphere(PlanetTerrainQuery),
}

impl TerrainBaseDensity {
    pub fn new(volume: &TerrainVolume) -> Result<Self, String> {
        volume.validate().map_err(|error| error.to_string())?;
        if volume.topology == TerrainTopology::CubeSphere {
            return PlanetTerrainQuery::new(volume).map(Self::CubeSphere);
        }
        let fbm = Fbm2D::new(
            Seed(volume.seed),
            FbmParams {
                octaves: volume.octaves,
                frequency: volume.frequency,
                amplitude: 1.0,
                lacunarity: volume.lacunarity,
                gain: volume.gain,
                offset: [0.0; 3],
                normalize: true,
            },
        )
        .map_err(|error| error.to_string())?;
        let warped = (volume.domain_warp_amplitude > 0.0)
            .then(|| {
                WarpedFbm2D::new(
                    fbm,
                    WarpParams {
                        amplitude: volume.domain_warp_amplitude,
                        frequency: volume.domain_warp_frequency,
                    },
                )
            })
            .transpose()
            .map_err(|error| error.to_string())?;
        Ok(Self::Planar {
            height_scale: volume.height_scale,
            fbm,
            warped,
        })
    }

    pub fn sample(&self, world: [f64; 3]) -> f32 {
        match self {
            Self::Planar {
                height_scale,
                fbm,
                warped,
            } => {
                let surface = warped.as_ref().map_or_else(
                    || fbm.sample_wide(world[0], world[2]),
                    |sampler| sampler.sample_wide(world[0], world[2]),
                ) * *height_scale;
                surface - world[1] as f32
            }
            Self::CubeSphere(query) => -query.altitude(world) as f32,
        }
    }
}

/// Versioned sparse-density configuration shared by editing, meshing and
/// persistence. Density greater than or equal to `iso_level` is solid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DensityTerrainConfig {
    /// Distance between adjacent density samples in logical world units.
    pub voxel_size: f64,
    /// Number of cells on each side of a sparse chunk.
    pub chunk_cells: u16,
    /// Surface threshold. Values at or above this threshold are solid.
    pub iso_level: f32,
    /// Fallback used outside materialised chunks when no base sampler is
    /// supplied. Negative values represent empty space by convention.
    pub default_density: f32,
}

impl Default for DensityTerrainConfig {
    fn default() -> Self {
        Self {
            voxel_size: 1.0,
            chunk_cells: 16,
            iso_level: 0.0,
            default_density: -1.0,
        }
    }
}

impl DensityTerrainConfig {
    pub fn validate(&self) -> Result<(), TerrainEditError> {
        if !self.voxel_size.is_finite() || self.voxel_size <= 0.0 {
            return Err(TerrainEditError::InvalidConfig(
                "voxel_size must be finite and positive".into(),
            ));
        }
        if !(2..=64).contains(&self.chunk_cells) {
            return Err(TerrainEditError::InvalidConfig(
                "chunk_cells must be in 2..=64".into(),
            ));
        }
        if !self.iso_level.is_finite() || !self.default_density.is_finite() {
            return Err(TerrainEditError::InvalidConfig(
                "density thresholds must be finite".into(),
            ));
        }
        Ok(())
    }
}

/// Stable address of one density chunk in the global voxel lattice.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct DensityChunkKey {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

impl DensityChunkKey {
    pub const fn new(x: i64, y: i64, z: i64) -> Self {
        Self { x, y, z }
    }

    pub fn origin(self, config: &DensityTerrainConfig) -> [f64; 3] {
        let span = config.voxel_size * f64::from(config.chunk_cells);
        [
            self.x as f64 * span,
            self.y as f64 * span,
            self.z as f64 * span,
        ]
    }
}

/// Shape used to attenuate a terrain brush near its boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerrainBrushFalloff {
    Constant,
    Linear,
    #[default]
    Smooth,
}

/// Density operation performed by a terrain brush.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TerrainBrushMode {
    Add,
    Subtract,
    Smooth,
    SetDensity(f32),
}

/// Bounded spherical edit in logical f64 coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainBrush {
    pub center: [f64; 3],
    pub radius: f64,
    pub strength: f32,
    #[serde(default)]
    pub falloff: TerrainBrushFalloff,
    pub mode: TerrainBrushMode,
    /// Optional material palette entry painted on affected samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<u16>,
}

impl TerrainBrush {
    pub fn validate(&self) -> Result<(), TerrainEditError> {
        if self.center.iter().any(|value| !value.is_finite()) {
            return Err(TerrainEditError::InvalidBrush(
                "brush center must be finite".into(),
            ));
        }
        if !self.radius.is_finite() || self.radius <= 0.0 {
            return Err(TerrainEditError::InvalidBrush(
                "brush radius must be finite and positive".into(),
            ));
        }
        if !self.strength.is_finite() || self.strength <= 0.0 || self.strength > 1_000_000.0 {
            return Err(TerrainEditError::InvalidBrush(
                "brush strength must be finite and in (0, 1000000]".into(),
            ));
        }
        if matches!(self.mode, TerrainBrushMode::Smooth) && self.strength > 1.0 {
            return Err(TerrainEditError::InvalidBrush(
                "smooth brush strength must be in (0, 1]".into(),
            ));
        }
        if matches!(self.mode, TerrainBrushMode::SetDensity(value) if !value.is_finite()) {
            return Err(TerrainEditError::InvalidBrush(
                "set-density target must be finite".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerrainEditDelta {
    pub edit_id: u64,
    pub changed_samples: u64,
    pub affected_chunks: Vec<DensityChunkKey>,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TerrainEditError {
    #[error("invalid editable terrain configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid terrain brush: {0}")]
    InvalidBrush(String),
    #[error("terrain brush would inspect {requested} samples, exceeding the limit of {limit}")]
    BrushBudgetExceeded { requested: u64, limit: u64 },
    #[error("terrain edit coordinate exceeds the i64 voxel lattice")]
    CoordinateOverflow,
    #[error("terrain edit storage error: {0}")]
    Storage(String),
    #[error("terrain edit payload is corrupt: {0}")]
    Corrupt(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct DensityChunk {
    pub(crate) key: DensityChunkKey,
    pub(crate) revision: u64,
    pub(crate) densities: Vec<f32>,
    pub(crate) materials: Vec<u16>,
}

/// Sparse editable density field. Untouched chunks consume no memory.
#[derive(Clone, Debug)]
pub struct EditableTerrain {
    config: DensityTerrainConfig,
    pub(crate) chunks: BTreeMap<DensityChunkKey, DensityChunk>,
    mesh_revisions: BTreeMap<DensityChunkKey, u64>,
    pub(crate) dirty_meshes: BTreeSet<DensityChunkKey>,
    unsaved_chunks: BTreeSet<DensityChunkKey>,
    next_edit_id: u64,
}

impl EditableTerrain {
    pub fn new(config: DensityTerrainConfig) -> Result<Self, TerrainEditError> {
        config.validate()?;
        Ok(Self {
            config,
            chunks: BTreeMap::new(),
            mesh_revisions: BTreeMap::new(),
            dirty_meshes: BTreeSet::new(),
            unsaved_chunks: BTreeSet::new(),
            next_edit_id: 1,
        })
    }

    pub fn config(&self) -> &DensityTerrainConfig {
        &self.config
    }

    pub fn materialized_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Stable addresses of chunks that contain authored density samples.
    /// This intentionally excludes mesh-only replacement chunks.
    pub fn materialized_chunk_keys(&self) -> impl Iterator<Item = DensityChunkKey> + '_ {
        self.chunks.keys().copied()
    }

    pub fn chunk_revision(&self, key: DensityChunkKey) -> u64 {
        self.mesh_revisions.get(&key).copied().unwrap_or(0)
    }

    pub fn density_at_lattice(&self, point: [i64; 3]) -> f32 {
        self.sample_existing(point)
            .map_or(self.config.default_density, |(density, _)| density)
    }

    pub fn material_at_lattice(&self, point: [i64; 3]) -> u16 {
        self.sample_existing(point)
            .map_or(0, |(_, material)| material)
    }

    /// Apply one brush while lazily initialising touched chunks from a base
    /// density sampler. The sampler receives logical f64 coordinates.
    pub fn apply_brush(
        &mut self,
        brush: &TerrainBrush,
        base_density: impl Fn([f64; 3]) -> f32,
    ) -> Result<TerrainEditDelta, TerrainEditError> {
        brush.validate()?;
        let min = std::array::from_fn(|axis| brush.center[axis] - brush.radius);
        let max = std::array::from_fn(|axis| brush.center[axis] + brush.radius);
        let lattice_min = self.world_to_lattice(min)?;
        let lattice_max = self.world_to_lattice(max)?;
        let dimensions = std::array::from_fn::<u64, 3, _>(|axis| {
            lattice_max[axis]
                .abs_diff(lattice_min[axis])
                .saturating_add(1)
        });
        let requested = dimensions
            .into_iter()
            .try_fold(1_u64, u64::checked_mul)
            .unwrap_or(u64::MAX);
        if requested > MAX_TERRAIN_BRUSH_SAMPLES {
            return Err(TerrainEditError::BrushBudgetExceeded {
                requested,
                limit: MAX_TERRAIN_BRUSH_SAMPLES,
            });
        }

        let mut changes = Vec::new();
        for z in lattice_min[2]..=lattice_max[2] {
            for y in lattice_min[1]..=lattice_max[1] {
                for x in lattice_min[0]..=lattice_max[0] {
                    let point = [x, y, z];
                    let world = self.lattice_to_world(point);
                    let distance = squared_distance(world, brush.center).sqrt();
                    if distance > brush.radius {
                        continue;
                    }
                    let normalized = (1.0 - distance / brush.radius) as f32;
                    let weight = match brush.falloff {
                        TerrainBrushFalloff::Constant => 1.0,
                        TerrainBrushFalloff::Linear => normalized,
                        TerrainBrushFalloff::Smooth => {
                            normalized * normalized * (3.0 - 2.0 * normalized)
                        }
                    };
                    let current = self.sample_with(point, &base_density).0;
                    let next = match brush.mode {
                        TerrainBrushMode::Add => current + brush.strength * weight,
                        TerrainBrushMode::Subtract => current - brush.strength * weight,
                        TerrainBrushMode::SetDensity(target) => {
                            current + (target - current) * (brush.strength * weight).min(1.0)
                        }
                        TerrainBrushMode::Smooth => {
                            let average = AXIS_NEIGHBORS
                                .iter()
                                .map(|offset| {
                                    self.sample_with(add_lattice(point, *offset), &base_density)
                                        .0
                                })
                                .sum::<f32>()
                                / AXIS_NEIGHBORS.len() as f32;
                            current + (average - current) * brush.strength * weight
                        }
                    };
                    let current_material = self.sample_with(point, &base_density).1;
                    let next_material = brush.material.unwrap_or(current_material);
                    if (next - current).abs() > f32::EPSILON || next_material != current_material {
                        changes.push((point, next, next_material));
                    }
                }
            }
        }

        let changed_samples = changes.len() as u64;
        let mut modified_owners = BTreeSet::new();
        let mut affected_meshes = BTreeSet::new();
        for (point, density, material) in changes {
            let owner = self.set_sample(point, density, material, &base_density);
            modified_owners.insert(owner);
            for z in -1..=1 {
                for y in -1..=1 {
                    for x in -1..=1 {
                        affected_meshes.insert(DensityChunkKey::new(
                            owner.x + x,
                            owner.y + y,
                            owner.z + z,
                        ));
                    }
                }
            }
        }
        for key in &modified_owners {
            self.unsaved_chunks.insert(*key);
        }
        for key in &affected_meshes {
            let revision = self.mesh_revisions.entry(*key).or_default();
            *revision = revision.wrapping_add(1).max(1);
            self.dirty_meshes.insert(*key);
        }

        let edit_id = self.next_edit_id;
        self.next_edit_id = self.next_edit_id.wrapping_add(1).max(1);
        Ok(TerrainEditDelta {
            edit_id,
            changed_samples,
            affected_chunks: affected_meshes.into_iter().collect(),
            bounds_min: min,
            bounds_max: max,
        })
    }

    pub fn take_dirty_mesh_chunks(&mut self, limit: usize) -> Vec<DensityChunkKey> {
        let selected = self
            .dirty_meshes
            .iter()
            .copied()
            .take(limit.max(1))
            .collect::<Vec<_>>();
        for key in &selected {
            self.dirty_meshes.remove(key);
        }
        selected
    }

    /// Queue a density chunk for polygonisation without changing any samples.
    ///
    /// Hosts use this to materialise the complete volumetric replacement for
    /// an overlapping height-field/planet LOD patch before retiring that
    /// patch. Repeated requests coalesce and therefore cannot grow the queue
    /// indefinitely.
    pub fn request_mesh_rebuild(&mut self, key: DensityChunkKey) {
        if self.dirty_meshes.insert(key) {
            let revision = self.mesh_revisions.entry(key).or_default();
            *revision = revision.wrapping_add(1).max(1);
        }
    }

    /// Put a previously dequeued chunk back into the rebuild queue while
    /// retaining its revision. This is used when a host-side GPU/ECS commit
    /// fails and the last known-good replacement must stay active.
    pub fn requeue_mesh_chunk(&mut self, key: DensityChunkKey) {
        self.dirty_meshes.insert(key);
    }

    pub fn pending_mesh_rebuilds(&self) -> usize {
        self.dirty_meshes.len()
    }

    pub(crate) fn take_unsaved_chunks(&mut self) -> Vec<DensityChunkKey> {
        std::mem::take(&mut self.unsaved_chunks)
            .into_iter()
            .collect()
    }

    pub(crate) fn restore_unsaved(&mut self, keys: impl IntoIterator<Item = DensityChunkKey>) {
        self.unsaved_chunks.extend(keys);
    }

    pub(crate) fn install_chunk(&mut self, chunk: DensityChunk) {
        // A meshed chunk samples the positive boundary of its neighbours.
        // Loading one persisted owner must therefore invalidate all adjacent
        // mesh chunks, not only the owner, or restored edits can crack at
        // chunk boundaries.
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    let key = DensityChunkKey::new(
                        chunk.key.x.saturating_add(x),
                        chunk.key.y.saturating_add(y),
                        chunk.key.z.saturating_add(z),
                    );
                    let revision = self.mesh_revisions.entry(key).or_default();
                    *revision = (*revision).max(chunk.revision).max(1);
                    self.dirty_meshes.insert(key);
                }
            }
        }
        self.chunks.insert(chunk.key, chunk);
    }

    pub(crate) fn sample_density_with(
        &self,
        point: [i64; 3],
        base_density: &impl Fn([f64; 3]) -> f32,
    ) -> f32 {
        self.sample_with(point, base_density).0
    }

    pub(crate) fn lattice_to_world(&self, point: [i64; 3]) -> [f64; 3] {
        std::array::from_fn(|axis| point[axis] as f64 * self.config.voxel_size)
    }

    fn world_to_lattice(&self, point: [f64; 3]) -> Result<[i64; 3], TerrainEditError> {
        let mut lattice = [0_i64; 3];
        for axis in 0..3 {
            let value = (point[axis] / self.config.voxel_size).floor();
            if value < i64::MIN as f64 || value > i64::MAX as f64 {
                return Err(TerrainEditError::CoordinateOverflow);
            }
            lattice[axis] = value as i64;
        }
        Ok(lattice)
    }

    fn sample_with(&self, point: [i64; 3], base_density: &impl Fn([f64; 3]) -> f32) -> (f32, u16) {
        self.sample_existing(point).unwrap_or_else(|| {
            let density = base_density(self.lattice_to_world(point));
            let density = if density.is_finite() {
                density
            } else {
                self.config.default_density
            };
            (density, 0)
        })
    }

    fn sample_existing(&self, point: [i64; 3]) -> Option<(f32, u16)> {
        let (key, local) = split_lattice(point, self.config.chunk_cells);
        let chunk = self.chunks.get(&key)?;
        let index = sample_index(local, self.config.chunk_cells);
        Some((chunk.densities[index], chunk.materials[index]))
    }

    fn set_sample(
        &mut self,
        point: [i64; 3],
        density: f32,
        material: u16,
        base_density: &impl Fn([f64; 3]) -> f32,
    ) -> DensityChunkKey {
        let (key, local) = split_lattice(point, self.config.chunk_cells);
        if !self.chunks.contains_key(&key) {
            let cells = i64::from(self.config.chunk_cells);
            let start = [key.x * cells, key.y * cells, key.z * cells];
            let sample_count = usize::from(self.config.chunk_cells).pow(3);
            let mut densities = Vec::with_capacity(sample_count);
            for z in 0..cells {
                for y in 0..cells {
                    for x in 0..cells {
                        let lattice = [start[0] + x, start[1] + y, start[2] + z];
                        let value = base_density(self.lattice_to_world(lattice));
                        densities.push(if value.is_finite() {
                            value
                        } else {
                            self.config.default_density
                        });
                    }
                }
            }
            self.chunks.insert(
                key,
                DensityChunk {
                    key,
                    revision: 0,
                    densities,
                    materials: vec![0; sample_count],
                },
            );
        }
        if let Some(chunk) = self.chunks.get_mut(&key) {
            let index = sample_index(local, self.config.chunk_cells);
            chunk.densities[index] = density;
            chunk.materials[index] = material;
            chunk.revision = chunk.revision.wrapping_add(1).max(1);
        }
        key
    }
}

const AXIS_NEIGHBORS: [[i64; 3]; 6] = [
    [-1, 0, 0],
    [1, 0, 0],
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
];

fn add_lattice(left: [i64; 3], right: [i64; 3]) -> [i64; 3] {
    std::array::from_fn(|axis| left[axis].saturating_add(right[axis]))
}

fn split_lattice(point: [i64; 3], chunk_cells: u16) -> (DensityChunkKey, [u16; 3]) {
    let cells = i64::from(chunk_cells);
    let chunk: [i64; 3] = std::array::from_fn(|axis| point[axis].div_euclid(cells));
    let local: [u16; 3] = std::array::from_fn(|axis| point[axis].rem_euclid(cells) as u16);
    (DensityChunkKey::new(chunk[0], chunk[1], chunk[2]), local)
}

fn sample_index(local: [u16; 3], chunk_cells: u16) -> usize {
    let cells = usize::from(chunk_cells);
    usize::from(local[0]) + cells * (usize::from(local[1]) + cells * usize::from(local[2]))
}

fn squared_distance(left: [f64; 3], right: [f64; 3]) -> f64 {
    (0..3)
        .map(|axis| {
            let delta = left[axis] - right[axis];
            delta * delta
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_below(point: [f64; 3]) -> f32 {
        (2.0 - point[1]) as f32
    }

    #[test]
    fn subtract_brush_excavates_and_reports_exact_changes() {
        let mut terrain = EditableTerrain::new(DensityTerrainConfig::default()).unwrap();
        let delta = terrain
            .apply_brush(
                &TerrainBrush {
                    center: [0.0, 0.0, 0.0],
                    radius: 2.0,
                    strength: 8.0,
                    falloff: TerrainBrushFalloff::Constant,
                    mode: TerrainBrushMode::Subtract,
                    material: None,
                },
                solid_below,
            )
            .unwrap();

        assert!(delta.changed_samples > 1);
        assert!(terrain.density_at_lattice([0, 0, 0]) < 0.0);
        assert!(terrain.pending_mesh_rebuilds() > 0);
    }

    #[test]
    fn negative_coordinates_use_euclidean_chunk_addressing() {
        let mut terrain = EditableTerrain::new(DensityTerrainConfig::default()).unwrap();
        terrain
            .apply_brush(
                &TerrainBrush {
                    center: [-17.0, -1.0, -17.0],
                    radius: 0.6,
                    strength: 2.0,
                    falloff: TerrainBrushFalloff::Constant,
                    mode: TerrainBrushMode::Add,
                    material: Some(3),
                },
                |_| -1.0,
            )
            .unwrap();

        assert!(terrain
            .chunks
            .contains_key(&DensityChunkKey::new(-2, -1, -2)));
    }

    #[test]
    fn oversized_brush_is_rejected_before_allocation() {
        let mut terrain = EditableTerrain::new(DensityTerrainConfig::default()).unwrap();
        let result = terrain.apply_brush(
            &TerrainBrush {
                center: [0.0; 3],
                radius: 10_000.0,
                strength: 1.0,
                falloff: TerrainBrushFalloff::Smooth,
                mode: TerrainBrushMode::Subtract,
                material: None,
            },
            |_| 1.0,
        );
        assert!(matches!(
            result,
            Err(TerrainEditError::BrushBudgetExceeded { .. })
        ));
        assert_eq!(terrain.materialized_chunk_count(), 0);
    }

    #[test]
    fn rebuild_requests_coalesce_and_failed_commits_can_requeue() {
        let mut terrain = EditableTerrain::new(DensityTerrainConfig::default()).unwrap();
        let key = DensityChunkKey::new(2, 3, 4);
        terrain.request_mesh_rebuild(key);
        let revision = terrain.chunk_revision(key);
        terrain.request_mesh_rebuild(key);
        assert_eq!(terrain.chunk_revision(key), revision);
        assert_eq!(terrain.take_dirty_mesh_chunks(8), vec![key]);
        terrain.requeue_mesh_chunk(key);
        assert_eq!(terrain.take_dirty_mesh_chunks(8), vec![key]);
        assert_eq!(terrain.chunk_revision(key), revision);
    }
}
