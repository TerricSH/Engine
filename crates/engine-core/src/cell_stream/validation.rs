use std::collections::{BTreeMap, BTreeSet};

use engine_asset::partition::WorldPartition;
use engine_scene::Scene;

use super::CellStreamError;

/// Validate partition cell scenes for streaming compatibility.
///
/// This is the shared rule set enforced by `sandbox project check` and by
/// [`crate::cell_stream::CellStreamingDriver::new`]:
///
/// - every cell's scene must be present in `scenes` (keyed by scene ID),
/// - cell scenes must not contain `engine.script` components,
/// - persistent entity IDs must be unique **across** cells (a load-time
///   merge would fail otherwise), and
/// - a cell that does not reference the startup scene must not share
///   persistent entity IDs with it (the startup scene is already live when
///   streaming begins; a cell that intentionally reuses the startup scene's
///   content must reference the startup scene itself, in which case the
///   driver adopts the already-live entities).
pub fn validate_partition_cell_scenes(
    partition: &WorldPartition,
    startup_scene_id: &str,
    scenes: &BTreeMap<String, &Scene>,
) -> Result<(), CellStreamError> {
    let mut owner_of: BTreeMap<&str, &str> = BTreeMap::new();
    for (cell_id, cell) in &partition.cells {
        let Some(scene) = scenes.get(&cell.scene).copied() else {
            return Err(CellStreamError::UnknownCellScene {
                cell_id: cell_id.clone(),
                scene_id: cell.scene.clone(),
            });
        };
        for entity in &scene.entities {
            if entity.components.contains_key("engine.script") {
                return Err(CellStreamError::ScriptComponentInCell {
                    cell_id: cell_id.clone(),
                    entity_id: entity.persistent_id.clone(),
                });
            }
            let persistent_id = entity.persistent_id.as_str();
            if let Some(first_cell) = owner_of.insert(persistent_id, cell_id) {
                if first_cell != cell_id {
                    return Err(CellStreamError::DuplicatePersistentIdAcrossCells {
                        first_cell: first_cell.to_string(),
                        second_cell: cell_id.clone(),
                        persistent_id: entity.persistent_id.clone(),
                    });
                }
            }
        }
        if cell.scene != startup_scene_id {
            if let Some(startup_scene) = scenes.get(startup_scene_id).copied() {
                let startup_ids: BTreeSet<&str> = startup_scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.as_str())
                    .collect();
                if let Some(conflict) = scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.as_str())
                    .find(|id| startup_ids.contains(id))
                {
                    return Err(CellStreamError::StartupSceneIdConflict {
                        cell_id: cell_id.clone(),
                        persistent_id: conflict.to_string(),
                        startup_scene_id: startup_scene_id.to_string(),
                    });
                }
            }
        }
    }
    Ok(())
}
