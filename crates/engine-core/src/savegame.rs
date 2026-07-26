//! Versioned, integrity-checked save-game snapshots.
//!
//! Authoring scenes describe initial state. A save game captures the live ECS
//! scene plus runtime-only state that cannot be reconstructed from authoring
//! data (world-origin rebasing and moving rigid bodies). Game-specific state
//! is carried in a typed, engine-serializable key/value map.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use engine_scene::{Scene, SCENE_ONLY_COMPONENT_TYPES};
use engine_serialize::Value;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::EngineRuntime;

const SAVE_MAGIC: &[u8; 8] = b"ENGSAVE1";
pub const SAVE_GAME_SCHEMA_VERSION: u16 = 1;
pub const MAX_SAVE_GAME_BYTES: usize = 256 * 1024 * 1024;
const HEADER_BYTES: usize = SAVE_MAGIC.len() + 2 + 8 + 32;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

/// Transient physics state keyed by a persistent scene entity ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedPhysicsBody {
    pub entity_id: String,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub sleeping: bool,
}

impl SavedPhysicsBody {
    fn validate(&self) -> Result<(), SaveGameError> {
        if self.entity_id.is_empty() {
            return Err(SaveGameError::InvalidSnapshot(
                "saved physics body has an empty entity ID".into(),
            ));
        }
        if !self
            .position
            .iter()
            .chain(self.rotation.iter())
            .chain(self.linear_velocity.iter())
            .chain(self.angular_velocity.iter())
            .all(|value| value.is_finite())
        {
            return Err(SaveGameError::InvalidSnapshot(format!(
                "saved physics body '{}' contains a non-finite value",
                self.entity_id
            )));
        }
        let rotation_length_squared = self.rotation.iter().map(|value| value * value).sum::<f32>();
        if rotation_length_squared <= f32::EPSILON {
            return Err(SaveGameError::InvalidSnapshot(format!(
                "saved physics body '{}' has a zero-length rotation",
                self.entity_id
            )));
        }
        Ok(())
    }
}

/// Portable live-world checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveGameSnapshot {
    pub schema_version: u16,
    pub scene: Scene,
    /// Origin paired with the scene's already-relative component data.
    pub world_origin: [f64; 3],
    pub world_origin_shift_count: u64,
    /// `GameState::to_u32()` when the gameplay feature is active.
    pub game_state: Option<u32>,
    pub physics_bodies: Vec<SavedPhysicsBody>,
    /// Project-defined state such as inventory, objectives, and dialogue flags.
    pub custom_state: BTreeMap<String, Value>,
}

/// Result of installing a decoded checkpoint into a [`crate::game_loop::GameLoop`].
#[derive(Clone, Debug, PartialEq)]
pub struct SaveGameRestoreReport {
    pub restored_physics_bodies: usize,
    pub skipped_physics_bodies: Vec<String>,
    pub custom_state: BTreeMap<String, Value>,
}

impl SaveGameSnapshot {
    pub fn validate(&self) -> Result<(), SaveGameError> {
        if self.schema_version != SAVE_GAME_SCHEMA_VERSION {
            return Err(SaveGameError::UnsupportedVersion(self.schema_version));
        }
        if !self.world_origin.iter().all(|value| value.is_finite()) {
            return Err(SaveGameError::InvalidSnapshot(
                "world origin contains a non-finite value".into(),
            ));
        }
        if self.game_state.is_some_and(|state| state > 5) {
            return Err(SaveGameError::InvalidSnapshot(
                "game state is outside the supported range 0..=5".into(),
            ));
        }
        if self.custom_state.len() > 65_536 {
            return Err(SaveGameError::InvalidSnapshot(
                "custom state contains too many keys".into(),
            ));
        }
        for (key, value) in &self.custom_state {
            if key.is_empty() || key.len() > 1_024 {
                return Err(SaveGameError::InvalidSnapshot(
                    "custom state keys must contain 1..=1024 bytes".into(),
                ));
            }
            validate_value(value, key)?;
        }
        let mut entity_ids = BTreeSet::new();
        for body in &self.physics_bodies {
            body.validate()?;
            if !entity_ids.insert(body.entity_id.as_str()) {
                return Err(SaveGameError::InvalidSnapshot(format!(
                    "duplicate saved physics body '{}'",
                    body.entity_id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum SaveGameError {
    #[error("no live world is loaded")]
    NoWorld,
    #[error("save game has an invalid magic header")]
    InvalidMagic,
    #[error("unsupported save-game schema version {0}")]
    UnsupportedVersion(u16),
    #[error("save game is truncated")]
    Truncated,
    #[error("save game contains trailing bytes")]
    TrailingBytes,
    #[error("save-game payload exceeds the {MAX_SAVE_GAME_BYTES}-byte limit")]
    TooLarge,
    #[error("save-game payload checksum does not match")]
    ChecksumMismatch,
    #[error("failed to encode save game: {0}")]
    Encode(String),
    #[error("failed to decode save game: {0}")]
    Decode(String),
    #[error("invalid save-game snapshot: {0}")]
    InvalidSnapshot(String),
    #[error("failed to access save-game path {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("save-game scene restore failed: {0}")]
    SceneRestore(String),
}

/// Encode a snapshot with a fixed header and SHA-256 payload integrity check.
pub fn encode_save_game(snapshot: &SaveGameSnapshot) -> Result<Vec<u8>, SaveGameError> {
    snapshot.validate()?;
    let payload =
        bincode::serialize(snapshot).map_err(|error| SaveGameError::Encode(error.to_string()))?;
    if payload.len() > MAX_SAVE_GAME_BYTES {
        return Err(SaveGameError::TooLarge);
    }
    let digest = Sha256::digest(&payload);
    let mut bytes = Vec::with_capacity(HEADER_BYTES + payload.len());
    bytes.extend_from_slice(SAVE_MAGIC);
    bytes.extend_from_slice(&SAVE_GAME_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&digest);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

/// Verify and decode a save game before any live runtime state is mutated.
pub fn decode_save_game(bytes: &[u8]) -> Result<SaveGameSnapshot, SaveGameError> {
    if bytes.len() < HEADER_BYTES {
        return Err(SaveGameError::Truncated);
    }
    if &bytes[..SAVE_MAGIC.len()] != SAVE_MAGIC {
        return Err(SaveGameError::InvalidMagic);
    }
    let mut cursor = SAVE_MAGIC.len();
    let version = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap());
    cursor += 2;
    if version != SAVE_GAME_SCHEMA_VERSION {
        return Err(SaveGameError::UnsupportedVersion(version));
    }
    let payload_len = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap()) as usize;
    cursor += 8;
    if payload_len > MAX_SAVE_GAME_BYTES {
        return Err(SaveGameError::TooLarge);
    }
    let expected_hash = &bytes[cursor..cursor + 32];
    cursor += 32;
    let expected_end = cursor
        .checked_add(payload_len)
        .ok_or(SaveGameError::TooLarge)?;
    if bytes.len() < expected_end {
        return Err(SaveGameError::Truncated);
    }
    if bytes.len() > expected_end {
        return Err(SaveGameError::TrailingBytes);
    }
    let payload = &bytes[cursor..expected_end];
    if Sha256::digest(payload).as_slice() != expected_hash {
        return Err(SaveGameError::ChecksumMismatch);
    }
    let snapshot: SaveGameSnapshot =
        bincode::deserialize(payload).map_err(|error| SaveGameError::Decode(error.to_string()))?;
    snapshot.validate()?;
    Ok(snapshot)
}

/// Transactionally replace a save file in its destination directory.
///
/// The previous file is retained as `.bak` until the new file has been fully
/// written, flushed, and renamed. A failed replacement rolls the backup back.
pub fn write_save_game(
    path: impl AsRef<Path>,
    snapshot: &SaveGameSnapshot,
) -> Result<(), SaveGameError> {
    let path = path.as_ref();
    let bytes = encode_save_game(snapshot)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| SaveGameError::InvalidSnapshot("save path has no file name".into()))?;
    let temp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let backup = parent.join(format!("{file_name}.bak"));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|source| io_error(&temp, source))?;
        file.write_all(&bytes)
            .map_err(|source| io_error(&temp, source))?;
        file.sync_all().map_err(|source| io_error(&temp, source))?;
        drop(file);

        if path.exists() {
            if backup.exists() {
                fs::remove_file(&backup).map_err(|source| io_error(&backup, source))?;
            }
            fs::rename(path, &backup).map_err(|source| io_error(path, source))?;
        }
        if let Err(source) = fs::rename(&temp, path) {
            if backup.exists() {
                let _ = fs::rename(&backup, path);
            }
            return Err(io_error(path, source));
        }
        if backup.exists() {
            fs::remove_file(&backup).map_err(|source| io_error(&backup, source))?;
        }
        Ok(())
    })();
    if result.is_err() && temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn read_save_game(path: impl AsRef<Path>) -> Result<SaveGameSnapshot, SaveGameError> {
    let path = path.as_ref();
    let mut file = fs::File::open(path).map_err(|source| io_error(path, source))?;
    let size = file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len();
    if size > (HEADER_BYTES + MAX_SAVE_GAME_BYTES) as u64 {
        return Err(SaveGameError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    decode_save_game(&bytes)
}

pub(crate) fn capture_live_scene(runtime: &EngineRuntime) -> Result<Scene, SaveGameError> {
    let mut scene = runtime
        .with_world(|world| world.to_scene())
        .ok_or(SaveGameError::NoWorld)?;

    // Scene-only metadata (currently scripts) is deliberately absent from the
    // ECS. Merge it from the retained authored scene for every still-live ID.
    if let Some(authored) = runtime.scene_ref() {
        let authored_by_id = authored
            .entities
            .iter()
            .map(|entity| (entity.persistent_id.as_str(), entity))
            .collect::<BTreeMap<_, _>>();
        for live in &mut scene.entities {
            if let Some(authored) = authored_by_id.get(live.persistent_id.as_str()) {
                for type_id in SCENE_ONLY_COMPONENT_TYPES {
                    if let Some(component) = authored.components.get(*type_id) {
                        live.components
                            .insert((*type_id).to_string(), component.clone());
                    }
                }
            }
        }
    }
    Ok(scene)
}

fn validate_value(value: &Value, path: &str) -> Result<(), SaveGameError> {
    let finite = match value {
        Value::Float32(value) => value.is_finite(),
        Value::Float64(value) => value.is_finite(),
        Value::Vec3(values) => values.iter().all(|value| value.is_finite()),
        Value::Quat(values) | Value::Color(values) => values.iter().all(|value| value.is_finite()),
        Value::List(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_value(value, &format!("{path}[{index}]"))?;
            }
            true
        }
        Value::Map(values) => {
            for (key, value) in values {
                validate_value(value, &format!("{path}.{key}"))?;
            }
            true
        }
        _ => true,
    };
    if finite {
        Ok(())
    } else {
        Err(SaveGameError::InvalidSnapshot(format!(
            "custom state '{path}' contains a non-finite value"
        )))
    }
}

fn io_error(path: impl AsRef<Path>, source: std::io::Error) -> SaveGameError {
    SaveGameError::Io {
        path: path.as_ref().to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SaveGameSnapshot {
        SaveGameSnapshot {
            schema_version: SAVE_GAME_SCHEMA_VERSION,
            scene: engine_scene::sample_scene(),
            world_origin: [1024.0, 0.0, -512.0],
            world_origin_shift_count: 3,
            game_state: Some(3),
            physics_bodies: vec![SavedPhysicsBody {
                entity_id: "cube-01".into(),
                position: [1.0, 2.0, 3.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                linear_velocity: [4.0, 0.0, 0.0],
                angular_velocity: [0.0, 1.0, 0.0],
                sleeping: false,
            }],
            custom_state: BTreeMap::from([
                ("chapter".into(), Value::UInt(4)),
                ("has_suit".into(), Value::Bool(true)),
            ]),
        }
    }

    #[test]
    fn binary_roundtrip_and_checksum_rejection() {
        let expected = snapshot();
        let bytes = encode_save_game(&expected).unwrap();
        assert_eq!(decode_save_game(&bytes).unwrap(), expected);

        let mut corrupt = bytes;
        *corrupt.last_mut().unwrap() ^= 0x40;
        assert!(matches!(
            decode_save_game(&corrupt),
            Err(SaveGameError::ChecksumMismatch)
        ));
    }

    #[test]
    fn transactional_file_replace_keeps_latest_valid_snapshot() {
        let dir = std::env::temp_dir().join(format!(
            "engine-savegame-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let path = dir.join("quick.sav");
        let mut first = snapshot();
        write_save_game(&path, &first).unwrap();
        first.custom_state.insert("chapter".into(), Value::UInt(5));
        write_save_game(&path, &first).unwrap();
        assert_eq!(read_save_game(&path).unwrap(), first);
        assert!(!dir.join("quick.sav.bak").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_runtime_values_are_rejected_before_encoding() {
        let mut invalid = snapshot();
        invalid.physics_bodies[0].linear_velocity[0] = f32::NAN;
        assert!(matches!(
            encode_save_game(&invalid),
            Err(SaveGameError::InvalidSnapshot(_))
        ));
    }
}
