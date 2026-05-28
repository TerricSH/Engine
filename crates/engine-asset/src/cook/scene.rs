//! Scene cooking pipeline.
//!
//! Serialises a scene asset using bincode with a [`CookedAssetHeader`] wrapper.
//! The scene data itself is opaque binary as far as the cook module is
//! concerned — it is whatever the editor or runtime scene system produces.

use std::path::Path;

use engine_serialize::SchemaVersion;
use serde::{Deserialize, Serialize};

use super::error::CookError;
use super::{write_cooked_artifact, AssetType, CookResult};

/// A cooked scene asset header describing the serialised scene payload.
///
/// The actual scene content is stored as opaque bytes inside the `data`
/// field.  Downstream systems (e.g. `engine-scene`) are responsible for
/// interpreting it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CookedScene {
    /// Number of entities in the scene.
    pub entity_count: u32,
    /// Opaque scene payload bytes.
    pub data: Vec<u8>,
}

/// Cook a scene from a source scene file.
///
/// Reads the source file as raw bytes (the scene format is opaque at this
/// level), wraps it in a [`CookedScene`] container, and writes the cooked
/// artifact with its header.
///
/// # Parameters
///
/// * `source` – path to the source scene file (e.g. `.scene` or `.json`).
/// * `output` – path for the cooked `.cooked` file.
/// * `entity_count` – number of entities in the scene (for metadata).
pub fn cook_scene(
    source: &Path,
    output: &Path,
    entity_count: u32,
) -> Result<CookResult, CookError> {
    // Read the raw scene bytes.
    let scene_bytes = std::fs::read(source)?;

    let cooked = CookedScene {
        entity_count,
        data: scene_bytes,
    };

    let payload = bincode::serialize(&cooked)
        .map_err(|e| CookError::InvalidAsset(e.to_string()))?;

    let result = write_cooked_artifact(
        output,
        AssetType::Scene.kind_code(),
        &payload,
        SchemaVersion::new(0, 1, 0),
    )?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooked_scene_serde_roundtrip() {
        let scene = CookedScene {
            entity_count: 42,
            data: vec![0x01, 0x02, 0x03, 0x04],
        };

        let bytes = bincode::serialize(&scene).unwrap();
        let restored: CookedScene = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.entity_count, 42);
        assert_eq!(restored.data, vec![0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn empty_scene() {
        let scene = CookedScene {
            entity_count: 0,
            data: vec![],
        };

        let bytes = bincode::serialize(&scene).unwrap();
        let restored: CookedScene = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.entity_count, 0);
        assert!(restored.data.is_empty());
    }
}
