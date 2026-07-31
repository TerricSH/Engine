use std::collections::BTreeSet;

use engine_scene::{Component, PrefabInstanceRef, Scene};
use engine_serialize::PersistentId;

use crate::commands::{Command, CommandBatch, RemoveComponent};
use crate::EditorError;

use super::util::instance_id;
use super::PrefabAuthoringError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefabUnpackMode {
    /// Remove the linkage for the selected prefab node while preserving nested
    /// prefab instance links.
    Instance,
    /// Remove linkage from the selected prefab node and every nested prefab in
    /// its scene subtree.
    Completely,
}

pub struct PrefabUnpackPlan {
    entity_ids: Vec<PersistentId>,
    command: CommandBatch,
}

impl PrefabUnpackPlan {
    pub fn entity_ids(&self) -> &[PersistentId] {
        &self.entity_ids
    }

    pub fn into_command(self) -> Box<dyn Command> {
        Box::new(self.command)
    }
}

/// Prepare an atomic, undoable unpack operation. This only removes explicit
/// prefab linkage; entity/component data remains unchanged.
pub fn prepare_unpack_prefab(
    scene: &Scene,
    selected_entity_id: &PersistentId,
    mode: PrefabUnpackMode,
) -> Result<PrefabUnpackPlan, PrefabAuthoringError> {
    let selected = scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == selected_entity_id)
        .ok_or_else(|| EditorError::EntityNotFound(selected_entity_id.clone()))?;
    let selected_instance = instance_id(selected).ok_or_else(|| {
        PrefabAuthoringError::InvalidRequest(format!(
            "entity '{}' is not a prefab instance",
            selected_entity_id
        ))
    })?;
    let direct_ids = scene
        .entities
        .iter()
        .filter(|entity| instance_id(entity).as_deref() == Some(selected_instance.as_str()))
        .map(|entity| entity.persistent_id.clone())
        .collect::<BTreeSet<_>>();
    let root_candidates = scene
        .entities
        .iter()
        .filter(|entity| direct_ids.contains(&entity.persistent_id))
        .filter(|entity| {
            entity
                .parent
                .as_ref()
                .is_none_or(|parent| !direct_ids.contains(parent))
        })
        .map(|entity| entity.persistent_id.clone())
        .collect::<Vec<_>>();
    if root_candidates.len() != 1 {
        return Err(PrefabAuthoringError::InvalidPrefab(format!(
            "instance '{}' has {} scene roots",
            selected_instance,
            root_candidates.len()
        )));
    }

    let ids = match mode {
        PrefabUnpackMode::Instance => direct_ids,
        PrefabUnpackMode::Completely => {
            let subtree = collect_scene_subtree_ids(scene, &root_candidates[0]);
            scene
                .entities
                .iter()
                .filter(|entity| {
                    subtree.contains(&entity.persistent_id)
                        && entity.components.contains_key(PrefabInstanceRef::TYPE_ID)
                })
                .map(|entity| entity.persistent_id.clone())
                .collect()
        }
    };
    let entity_ids = scene
        .entities
        .iter()
        .filter(|entity| ids.contains(&entity.persistent_id))
        .map(|entity| entity.persistent_id.clone())
        .collect::<Vec<_>>();
    let commands = entity_ids
        .iter()
        .map(|entity_id| {
            Box::new(RemoveComponent::new(
                entity_id.clone(),
                PrefabInstanceRef::TYPE_ID.to_string(),
            )) as Box<dyn Command>
        })
        .collect();
    Ok(PrefabUnpackPlan {
        entity_ids,
        command: CommandBatch::new("Unpack Prefab", commands),
    })
}

fn collect_scene_subtree_ids(scene: &Scene, root: &PersistentId) -> BTreeSet<PersistentId> {
    let mut result = BTreeSet::new();
    let mut pending = vec![root.clone()];
    while let Some(parent) = pending.pop() {
        if !result.insert(parent.clone()) {
            continue;
        }
        pending.extend(
            scene
                .entities
                .iter()
                .filter(|entity| entity.parent.as_ref() == Some(&parent))
                .map(|entity| entity.persistent_id.clone()),
        );
    }
    result
}
