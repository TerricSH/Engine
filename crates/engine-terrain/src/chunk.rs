use bincode::Options;
use serde::{Deserialize, Serialize};

use crate::TerrainVolume;

const TERRAIN_CHUNK_ID_MAGIC: [u8; 8] = *b"TCHNK001";
const TERRAIN_CHUNK_ID_VERSION: u16 = 1;

/// Stable identity for one authored terrain volume.
///
/// `0` is reserved for the legacy single-volume API. Engine hosts that can
/// stream more than one volume derive a non-zero identity from the owning
/// entity's persistent ID, keeping otherwise-identical face/quadtree
/// coordinates in independent namespaces.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TerrainVolumeId(u64);

impl TerrainVolumeId {
    /// Namespace used by the original single-volume constructors.
    pub const LEGACY: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn is_legacy(self) -> bool {
        self.0 == Self::LEGACY.0
    }

    /// Derive a deterministic runtime namespace from an ECS persistent ID.
    ///
    /// FNV-1a is intentionally used instead of `DefaultHasher`: its output is
    /// stable across processes and Rust releases, so regeneration and debug
    /// snapshots retain the same identity.
    pub fn from_persistent_id(persistent_id: &str) -> Self {
        Self::from_domain_and_bytes(b"persistent\0", persistent_id.as_bytes())
    }

    /// Derive a namespace for an entity that has no authored persistent ID.
    ///
    /// The domain tag prevents an authored ID such as `runtime:7:3` from
    /// aliasing the anonymous entity at index 7, generation 3.
    pub fn from_runtime_entity(index: u32, generation: u32) -> Self {
        let mut bytes = [0_u8; 8];
        bytes[..4].copy_from_slice(&index.to_le_bytes());
        bytes[4..].copy_from_slice(&generation.to_le_bytes());
        Self::from_domain_and_bytes(b"runtime\0", &bytes)
    }

    fn from_domain_and_bytes(domain: &[u8], bytes: &[u8]) -> Self {
        const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;

        let mut hash = OFFSET_BASIS;
        for byte in domain.iter().chain(bytes) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        // Keep zero exclusively for old callers and payloads.
        Self(if hash == 0 { 1 } else { hash })
    }
}

/// Cube face carried by a chunk identity. `Planar` preserves the existing
/// infinite-heightfield namespace; the other variants make all six planetary
/// quadtrees coexist in one streaming runtime without ID collisions.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum TerrainFace {
    #[default]
    Planar,
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl TerrainFace {
    pub const CUBE_FACES: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Planar => "plane",
            Self::PositiveX => "px",
            Self::NegativeX => "nx",
            Self::PositiveY => "py",
            Self::NegativeY => "ny",
            Self::PositiveZ => "pz",
            Self::NegativeZ => "nz",
        }
    }
}

/// Stable terrain work identity. It is intentionally unrelated to a world
/// partition cell: cells are loading units, while terrain chunks are
/// generation/LOD units and may use a different size and lifetime.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct TerrainChunkId {
    #[serde(default)]
    pub face: TerrainFace,
    pub x: i64,
    pub z: i64,
    pub lod: u8,
    #[serde(default)]
    pub volume_id: TerrainVolumeId,
}

#[derive(Serialize, Deserialize)]
struct LegacyTerrainChunkId {
    face: TerrainFace,
    x: i64,
    z: i64,
    lod: u8,
}

#[derive(Serialize, Deserialize)]
struct TerrainChunkIdEnvelope {
    magic: [u8; 8],
    version: u16,
    id: TerrainChunkId,
}

impl TerrainChunkId {
    pub const fn new(x: i64, z: i64, lod: u8) -> Self {
        Self {
            volume_id: TerrainVolumeId::LEGACY,
            face: TerrainFace::Planar,
            x,
            z,
            lod,
        }
    }

    pub const fn on_face(face: TerrainFace, x: i64, z: i64, lod: u8) -> Self {
        Self {
            volume_id: TerrainVolumeId::LEGACY,
            face,
            x,
            z,
            lod,
        }
    }

    pub const fn for_volume(volume_id: TerrainVolumeId, x: i64, z: i64, lod: u8) -> Self {
        Self {
            volume_id,
            face: TerrainFace::Planar,
            x,
            z,
            lod,
        }
    }

    pub const fn on_volume_face(
        volume_id: TerrainVolumeId,
        face: TerrainFace,
        x: i64,
        z: i64,
        lod: u8,
    ) -> Self {
        Self {
            volume_id,
            face,
            x,
            z,
            lod,
        }
    }

    pub const fn with_volume(mut self, volume_id: TerrainVolumeId) -> Self {
        self.volume_id = volume_id;
        self
    }

    /// Serialize the current chunk identity. Persisted bincode data should be
    /// decoded with [`from_bincode_compatible`](Self::from_bincode_compatible)
    /// because serde defaults cannot extend a positional bincode sequence.
    pub fn to_bincode(self) -> Result<Vec<u8>, Box<bincode::ErrorKind>> {
        strict_bincode_options().serialize(&TerrainChunkIdEnvelope {
            magic: TERRAIN_CHUNK_ID_MAGIC,
            version: TERRAIN_CHUNK_ID_VERSION,
            id: self,
        })
    }

    /// Decode both the current identity and the original face/x/z/lod binary
    /// layout. Legacy chunks enter [`TerrainVolumeId::LEGACY`]. A payload that
    /// begins with all or part of the current magic is never interpreted as a
    /// legacy payload, so corruption cannot silently change its namespace.
    pub fn from_bincode_compatible(bytes: &[u8]) -> Result<Self, Box<bincode::ErrorKind>> {
        if is_current_envelope_candidate(bytes) {
            let envelope: TerrainChunkIdEnvelope = strict_bincode_options().deserialize(bytes)?;
            if envelope.magic != TERRAIN_CHUNK_ID_MAGIC {
                return Err(Box::new(bincode::ErrorKind::Custom(
                    "invalid terrain chunk identity magic".to_owned(),
                )));
            }
            if envelope.version != TERRAIN_CHUNK_ID_VERSION {
                return Err(Box::new(bincode::ErrorKind::Custom(format!(
                    "unsupported terrain chunk identity version {}",
                    envelope.version
                ))));
            }
            return Ok(envelope.id);
        }

        let legacy: LegacyTerrainChunkId = strict_bincode_options().deserialize(bytes)?;
        Ok(Self::on_face(legacy.face, legacy.x, legacy.z, legacy.lod))
    }

    pub fn label(self) -> String {
        let coordinates = format!(
            "{}-{}-{}-lod{}",
            self.face.label(),
            self.x,
            self.z,
            self.lod
        );
        if self.volume_id.is_legacy() {
            coordinates
        } else {
            format!("v{:016x}-{coordinates}", self.volume_id.get())
        }
    }
}

fn strict_bincode_options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
}

fn is_current_envelope_candidate(bytes: &[u8]) -> bool {
    bytes.starts_with(&TERRAIN_CHUNK_ID_MAGIC)
        || (!bytes.is_empty() && TERRAIN_CHUNK_ID_MAGIC.starts_with(bytes))
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

/// Exact static triangle collision for curved chunks, excluding render-only
/// crack skirts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainTriangleCollisionData {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

impl TerrainTriangleCollisionData {
    pub fn estimated_bytes(&self) -> usize {
        self.positions.len() * 12 + self.triangles.len() * 12
    }
}

/// Parameters required to decode the continuous parent-LOD displacement
/// embedded in a generated mesh's vertex-normal magnitudes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainGeomorphData {
    pub delta_scale: f32,
    /// Planet center in the generated mesh's local coordinate frame.
    pub local_origin: [f32; 3],
}

/// Fully generated chunk ready for bounded main-thread commit.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainChunkData {
    pub id: TerrainChunkId,
    pub revision: u64,
    pub origin: [f64; 3],
    /// Local translation applied by the host when centering a chunk entity.
    /// Mesh and collision vertices are shifted by this value while the entity
    /// receives the inverse logical translation.
    pub local_center: [f32; 3],
    pub mesh: TerrainMeshData,
    pub geomorph: Option<TerrainGeomorphData>,
    pub collision: Option<TerrainCollisionData>,
    pub triangle_collision: Option<TerrainTriangleCollisionData>,
}

impl TerrainChunkData {
    pub fn estimated_bytes(&self) -> usize {
        self.mesh.estimated_bytes()
            + self
                .collision
                .as_ref()
                .map_or(0, TerrainCollisionData::estimated_bytes)
            + self
                .triangle_collision
                .as_ref()
                .map_or(0, TerrainTriangleCollisionData::estimated_bytes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_chunk_payload_and_constructors_use_the_original_namespace() {
        let restored: TerrainChunkId =
            serde_json::from_str(r#"{"face":"PositiveY","x":4,"z":7,"lod":2}"#).unwrap();
        assert_eq!(
            restored,
            TerrainChunkId::on_face(TerrainFace::PositiveY, 4, 7, 2)
        );
        assert_eq!(TerrainChunkId::new(-2, 3, 1).label(), "plane--2-3-lod1");
        assert!(restored.volume_id.is_legacy());

        let legacy_bytes = bincode::serialize(&LegacyTerrainChunkId {
            face: TerrainFace::PositiveY,
            x: 4,
            z: 7,
            lod: 2,
        })
        .unwrap();
        assert!(bincode::deserialize::<TerrainChunkId>(&legacy_bytes).is_err());
        assert_eq!(
            TerrainChunkId::from_bincode_compatible(&legacy_bytes).unwrap(),
            restored
        );
    }

    #[test]
    fn current_chunk_payload_round_trips_through_versioned_envelope() {
        let expected = TerrainChunkId::on_volume_face(
            TerrainVolumeId::from_persistent_id("planet:current"),
            TerrainFace::NegativeZ,
            -12,
            37,
            5,
        );
        let current_bytes = expected.to_bincode().unwrap();
        assert!(current_bytes.starts_with(&TERRAIN_CHUNK_ID_MAGIC));
        assert_eq!(
            TerrainChunkId::from_bincode_compatible(&current_bytes).unwrap(),
            expected
        );
    }

    #[test]
    fn truncated_current_chunk_payload_never_uses_legacy_decoder() {
        let expected = TerrainChunkId::on_volume_face(
            TerrainVolumeId::new(9),
            TerrainFace::PositiveX,
            2,
            8,
            3,
        );
        let bytes = expected.to_bincode().unwrap();

        for length in 1..bytes.len() {
            assert!(
                TerrainChunkId::from_bincode_compatible(&bytes[..length]).is_err(),
                "accepted truncated current payload at {length}/{} bytes",
                bytes.len()
            );
        }
    }

    #[test]
    fn current_and_legacy_chunk_payloads_reject_trailing_bytes() {
        let expected = TerrainChunkId::on_volume_face(
            TerrainVolumeId::new(11),
            TerrainFace::NegativeY,
            5,
            -3,
            4,
        );
        let mut current_bytes = expected.to_bincode().unwrap();
        current_bytes.push(0xaa);
        assert!(TerrainChunkId::from_bincode_compatible(&current_bytes).is_err());

        let mut legacy_bytes = bincode::serialize(&LegacyTerrainChunkId {
            face: TerrainFace::Planar,
            x: 5,
            z: -3,
            lod: 4,
        })
        .unwrap();
        legacy_bytes.push(0xaa);
        assert!(TerrainChunkId::from_bincode_compatible(&legacy_bytes).is_err());
    }

    #[test]
    fn persistent_volume_identity_is_stable_and_changes_chunk_labels() {
        let first = TerrainVolumeId::from_persistent_id("planet:first");
        let repeated = TerrainVolumeId::from_persistent_id("planet:first");
        let second = TerrainVolumeId::from_persistent_id("planet:second");
        assert_eq!(first, repeated);
        assert_ne!(first, second);
        assert!(!first.is_legacy());

        let coordinates = TerrainChunkId::on_face(TerrainFace::PositiveZ, 0, 0, 1);
        let first_chunk = coordinates.with_volume(first);
        let second_chunk = coordinates.with_volume(second);
        assert_ne!(first_chunk, second_chunk);
        assert_ne!(first_chunk.label(), second_chunk.label());
        assert_ne!(
            TerrainVolumeId::from_persistent_id("runtime:7:3"),
            TerrainVolumeId::from_runtime_entity(7, 3),
            "authored and anonymous identity domains must never alias by text"
        );
    }
}
