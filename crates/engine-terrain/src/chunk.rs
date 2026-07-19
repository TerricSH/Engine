use serde::{Deserialize, Serialize};

use crate::TerrainVolume;

/// Stable terrain work identity. It is intentionally unrelated to a world
/// partition cell: cells are loading units, while terrain chunks are
/// generation/LOD units and may use a different size and lifetime.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct TerrainChunkId {
    pub x: i64,
    pub z: i64,
    pub lod: u8,
}

impl TerrainChunkId {
    pub const fn new(x: i64, z: i64, lod: u8) -> Self {
        Self { x, z, lod }
    }

    pub fn label(self) -> String {
        format!("{}-{}-lod{}", self.x, self.z, self.lod)
    }
}

/// One desired chunk and its scheduling priority. Lower values are committed
/// first. `revision` lets hot regeneration invalidate in-flight output.
#[derive(Clone, Debug)]
pub struct TerrainChunkRequest {
    pub id: TerrainChunkId,
    pub revision: u64,
    pub priority: u32,
    pub volume: TerrainVolume,
}

/// Renderable CPU mesh. Positions are local to [`TerrainChunkData::origin`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainMeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

impl TerrainMeshData {
    pub fn estimated_bytes(&self) -> usize {
        self.positions.len() * 12
            + self.normals.len() * 12
            + self.uvs.len() * 8
            + self.indices.len() * 4
    }
}

/// Heightfield collision payload using row-major heights. Skirt vertices are
/// excluded so collision remains the exact sampled surface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainCollisionData {
    pub rows: u32,
    pub columns: u32,
    pub heights: Vec<f32>,
    /// Horizontal spacing between adjacent samples on X/Z.
    pub sample_spacing: f32,
}

impl TerrainCollisionData {
    pub fn estimated_bytes(&self) -> usize {
        self.heights.len() * std::mem::size_of::<f32>()
    }
}

/// Fully generated chunk ready for bounded main-thread commit.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainChunkData {
    pub id: TerrainChunkId,
    pub revision: u64,
    pub origin: [f64; 3],
    pub mesh: TerrainMeshData,
    pub collision: Option<TerrainCollisionData>,
}

impl TerrainChunkData {
    pub fn estimated_bytes(&self) -> usize {
        self.mesh.estimated_bytes()
            + self
                .collision
                .as_ref()
                .map_or(0, TerrainCollisionData::estimated_bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainChunkState {
    Queued,
    Generating,
    ReadyToCommit,
    Resident,
    Failed,
}
