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

impl TerrainEditStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save_modified(&self, terrain: &mut EditableTerrain) -> Result<usize, TerrainEditError> {
        fs::create_dir_all(&self.root).map_err(storage_error)?;
        let keys = terrain.take_unsaved_chunks();
        let mut saved = 0;
        for (offset, key) in keys.iter().copied().enumerate() {
            let Some(chunk) = terrain.chunks.get(&key) else {
                continue;
            };
            if let Err(error) = self.write_chunk(terrain.config(), chunk) {
                terrain.restore_unsaved(keys[offset..].iter().copied());
                return Err(error);
            }
            saved += 1;
        }
        Ok(saved)
    }

    pub fn load_latest(&self) -> Result<Option<EditableTerrain>, TerrainEditError> {
        if !self.root.exists() {
            return Ok(None);
        }
        let mut latest = BTreeMap::<DensityChunkKey, StoredDensityChunk>::new();
        for entry in fs::read_dir(&self.root).map_err(storage_error)? {
            let entry = entry.map_err(storage_error)?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("ted") {
                continue;
            }
            let stored = read_chunk(&entry.path())?;
            let replace = latest
                .get(&stored.chunk.key)
                .is_none_or(|current| stored.chunk.revision > current.chunk.revision);
            if replace {
                latest.insert(stored.chunk.key, stored);
            }
        }
        let Some(config) = latest.values().next().map(|stored| stored.config.clone()) else {
            return Ok(None);
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
        Ok(Some(terrain))
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
            return Ok(());
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
}
