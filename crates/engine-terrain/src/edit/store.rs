use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use bincode::Options;
use serde::{Deserialize, Serialize};

use super::{
    DensityChunk, DensityChunkKey, DensityTerrainConfig, EditableTerrain, TerrainEditError,
};

const EDIT_CHUNK_MAGIC: [u8; 8] = *b"TEDT0001";
const EDIT_CHUNK_VERSION: u16 = 1;
const MAX_EDIT_CHUNK_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct StoredDensityChunk {
    magic: [u8; 8],
    version: u16,
    config: DensityTerrainConfig,
    chunk: DensityChunk,
    checksum: u64,
}

/// Append-only directory store. Every modified chunk is written as an
/// immutable revision file so interrupted saves cannot destroy the previous
/// valid edit state.
#[derive(Clone, Debug)]
pub struct TerrainEditStore {
    root: PathBuf,
}

/// One revision file that was ignored while recovering the latest valid
/// state. Filesystem access failures remain fatal; only corrupt payloads are
/// skipped so permission and device errors cannot silently erase terrain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainEditLoadIssue {
    pub path: PathBuf,
    pub error: String,
}

/// Result of tolerant terrain recovery, including every ignored corrupt
/// revision for diagnostics and telemetry.
#[derive(Clone, Debug)]
pub struct TerrainEditLoadReport {
    pub terrain: Option<EditableTerrain>,
    pub skipped_revisions: Vec<TerrainEditLoadIssue>,
}

impl TerrainEditStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Persist every currently modified chunk.
    ///
    /// A chunk remains pending until its immutable revision file has been
    /// flushed and atomically renamed. Successfully committed chunks are
    /// acknowledged individually, so a later write error never drops them or
    /// the unprocessed tail from retry state. This operation is not a
    /// multi-chunk transaction: callers must invoke it before reporting an
    /// edit as durable, because a process failure before a chunk is committed
    /// cannot recover that in-memory change.
    pub fn save_modified(&self, terrain: &mut EditableTerrain) -> Result<usize, TerrainEditError> {
        fs::create_dir_all(&self.root).map_err(storage_error)?;
        let keys = terrain.unsaved_chunk_keys();
        let mut saved = 0;
        for key in keys {
            let Some(chunk) = terrain.chunks.get(&key) else {
                return Err(TerrainEditError::Storage(format!(
                    "pending density chunk {key:?} is missing"
                )));
            };
            let revision = chunk.revision;
            self.write_chunk(terrain.config(), chunk)?;
            terrain.acknowledge_saved_chunk(key, revision);
            saved += 1;
        }
        Ok(saved)
    }

    pub fn load_latest(&self) -> Result<Option<EditableTerrain>, TerrainEditError> {
        Ok(self.load_latest_report()?.terrain)
    }

    /// Load the highest valid revision of every chunk. A damaged newest file
    /// does not hide an older valid revision of the same chunk.
    pub fn load_latest_report(&self) -> Result<TerrainEditLoadReport, TerrainEditError> {
        if !self.root.exists() {
            return Ok(TerrainEditLoadReport {
                terrain: None,
                skipped_revisions: Vec::new(),
            });
        }
        let mut latest = BTreeMap::<DensityChunkKey, StoredDensityChunk>::new();
        let mut skipped_revisions = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(storage_error)? {
            let entry = entry.map_err(storage_error)?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("ted") {
                continue;
            }
            let path = entry.path();
            let stored = match read_chunk(&path) {
                Ok(stored) => stored,
                Err(TerrainEditError::Corrupt(error)) => {
                    skipped_revisions.push(TerrainEditLoadIssue { path, error });
                    continue;
                }
                Err(error) => return Err(error),
            };
            let replace = latest
                .get(&stored.chunk.key)
                .is_none_or(|current| stored.chunk.revision > current.chunk.revision);
            if replace {
                latest.insert(stored.chunk.key, stored);
            }
        }
        let Some(config) = latest.values().next().map(|stored| stored.config.clone()) else {
            return Ok(TerrainEditLoadReport {
                terrain: None,
                skipped_revisions,
            });
        };
        let mut terrain = EditableTerrain::new(config.clone())?;
        for stored in latest.into_values() {
            if stored.config != config {
                return Err(TerrainEditError::Corrupt(
                    "density chunk configurations do not match".into(),
                ));
            }
            terrain.install_chunk(stored.chunk);
        }
        Ok(TerrainEditLoadReport {
            terrain: Some(terrain),
            skipped_revisions,
        })
    }

    fn write_chunk(
        &self,
        config: &DensityTerrainConfig,
        chunk: &DensityChunk,
    ) -> Result<(), TerrainEditError> {
        let payload = strict_options()
            .serialize(&(config, chunk))
            .map_err(|error| TerrainEditError::Storage(error.to_string()))?;
        let stored = StoredDensityChunk {
            magic: EDIT_CHUNK_MAGIC,
            version: EDIT_CHUNK_VERSION,
            config: config.clone(),
            chunk: chunk.clone(),
            checksum: fnv1a(&payload),
        };
        let encoded = strict_options()
            .serialize(&stored)
            .map_err(|error| TerrainEditError::Storage(error.to_string()))?;
        if encoded.len() as u64 > MAX_EDIT_CHUNK_BYTES {
            return Err(TerrainEditError::Storage(
                "encoded density chunk exceeds the storage limit".into(),
            ));
        }
        let path = self.root.join(format!(
            "chunk_{}_{}_{}_{}.ted",
            chunk.key.x, chunk.key.y, chunk.key.z, chunk.revision
        ));
        if path.exists() {
            let existing = read_chunk(&path)?;
            if existing.config == *config && existing.chunk == *chunk {
                return Ok(());
            }
            return Err(TerrainEditError::Corrupt(format!(
                "{} conflicts with the pending density chunk revision",
                path.display()
            )));
        }
        let temporary = path.with_extension("ted.tmp");
        let mut file = File::create(&temporary).map_err(storage_error)?;
        file.write_all(&encoded).map_err(storage_error)?;
        file.sync_all().map_err(storage_error)?;
        fs::rename(&temporary, &path).map_err(storage_error)
    }
}

fn read_chunk(path: &Path) -> Result<StoredDensityChunk, TerrainEditError> {
    let metadata = fs::metadata(path).map_err(storage_error)?;
    if metadata.len() > MAX_EDIT_CHUNK_BYTES {
        return Err(TerrainEditError::Corrupt(format!(
            "{} exceeds the density chunk size limit",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .map_err(storage_error)?
        .take(MAX_EDIT_CHUNK_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(storage_error)?;
    let stored: StoredDensityChunk = strict_options()
        .deserialize(&bytes)
        .map_err(|error| TerrainEditError::Corrupt(error.to_string()))?;
    if stored.magic != EDIT_CHUNK_MAGIC || stored.version != EDIT_CHUNK_VERSION {
        return Err(TerrainEditError::Corrupt(
            "unsupported density edit chunk header".into(),
        ));
    }
    let payload = strict_options()
        .serialize(&(&stored.config, &stored.chunk))
        .map_err(|error| TerrainEditError::Corrupt(error.to_string()))?;
    if fnv1a(&payload) != stored.checksum {
        return Err(TerrainEditError::Corrupt(
            "density edit chunk checksum mismatch".into(),
        ));
    }
    Ok(stored)
}

fn strict_options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn storage_error(error: std::io::Error) -> TerrainEditError {
    TerrainEditError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TerrainBrush, TerrainBrushFalloff, TerrainBrushMode};

    fn edit_point(terrain: &mut EditableTerrain, center: [f64; 3]) {
        terrain
            .apply_brush(
                &TerrainBrush {
                    center,
                    radius: 0.25,
                    strength: 1.0,
                    falloff: TerrainBrushFalloff::Constant,
                    mode: TerrainBrushMode::Subtract,
                    material: None,
                },
                |_| 1.0,
            )
            .unwrap();
    }

    #[test]
    fn latest_chunk_revisions_round_trip() {
        let root = std::env::temp_dir().join(format!(
            "engine-terrain-edit-{}-{}",
            std::process::id(),
            fnv1a(b"latest_chunk_revisions_round_trip")
        ));
        let _ = fs::remove_dir_all(&root);
        let store = TerrainEditStore::new(&root);
        let mut terrain = EditableTerrain::new(DensityTerrainConfig::default()).unwrap();
        terrain
            .apply_brush(
                &TerrainBrush {
                    center: [1.0; 3],
                    radius: 1.0,
                    strength: 3.0,
                    falloff: TerrainBrushFalloff::Constant,
                    mode: TerrainBrushMode::Subtract,
                    material: Some(9),
                },
                |_| 1.0,
            )
            .unwrap();
        assert!(store.save_modified(&mut terrain).unwrap() > 0);
        let restored = store.load_latest().unwrap().unwrap();
        assert_eq!(restored.density_at_lattice([1, 1, 1]), -2.0);
        assert_eq!(restored.material_at_lattice([1, 1, 1]), 9);
        assert!(restored.pending_mesh_rebuilds() >= 27);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_newest_revision_falls_back_to_latest_valid_chunk() {
        let root = std::env::temp_dir().join(format!(
            "engine-terrain-edit-{}-{}",
            std::process::id(),
            fnv1a(b"corrupt_newest_revision_falls_back_to_latest_valid_chunk")
        ));
        let _ = fs::remove_dir_all(&root);
        let store = TerrainEditStore::new(&root);
        let mut terrain = EditableTerrain::new(DensityTerrainConfig {
            chunk_cells: 4,
            ..DensityTerrainConfig::default()
        })
        .unwrap();

        edit_point(&mut terrain, [1.0; 3]);
        assert_eq!(store.save_modified(&mut terrain).unwrap(), 1);
        edit_point(&mut terrain, [1.0; 3]);
        assert_eq!(store.save_modified(&mut terrain).unwrap(), 1);
        fs::write(root.join("chunk_0_0_0_2.ted"), b"corrupt").unwrap();

        let report = store.load_latest_report().unwrap();
        assert_eq!(report.skipped_revisions.len(), 1);
        let restored = report.terrain.unwrap();
        assert_eq!(restored.density_at_lattice([1, 1, 1]), 0.0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_error_only_acknowledges_committed_chunk_revisions() {
        let root = std::env::temp_dir().join(format!(
            "engine-terrain-edit-{}-{}",
            std::process::id(),
            fnv1a(b"save_error_only_acknowledges_committed_chunk_revisions")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let store = TerrainEditStore::new(&root);
        let mut terrain = EditableTerrain::new(DensityTerrainConfig {
            chunk_cells: 2,
            ..DensityTerrainConfig::default()
        })
        .unwrap();
        edit_point(&mut terrain, [1.0; 3]);
        edit_point(&mut terrain, [3.0, 1.0, 1.0]);
        assert_eq!(terrain.pending_persistence(), 2);

        let rejected = root.join("chunk_1_0_0_1.ted");
        fs::write(&rejected, b"corrupt").unwrap();
        assert!(store.save_modified(&mut terrain).is_err());
        assert_eq!(terrain.pending_persistence(), 1);
        assert!(root.join("chunk_0_0_0_1.ted").is_file());

        fs::remove_file(rejected).unwrap();
        assert_eq!(store.save_modified(&mut terrain).unwrap(), 1);
        assert_eq!(terrain.pending_persistence(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
