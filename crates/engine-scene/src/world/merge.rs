//! Live-world merge primitives for streaming foundations.
//!
//! [`World::merge_scene`] inserts a second scene's entities into an already
//! live world, and [`World::destroy_subtree_by_persistent_ids`] removes whole
//! persistent-ID subtrees for a future cell unload path. Both are atomic from
//! the caller's perspective: validation runs before any mutation, and a
//! registry-strict merge that produces component diagnostics rolls back every
//! entity it created.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use engine_serialize::{PersistentId, Value};
use thiserror::Error;

use crate::components::Transform;
use crate::scene::{Scene, SceneLoadDiagnostic};
use crate::{Component, Entity};

use super::World;

/// Failure returned by [`World::merge_scene`].
///
/// Every structural variant is detected before the world is mutated, so the
/// caller's live world is left untouched. [`MergeError::ComponentLoad`] is
/// detected during population (registry-strict worlds only) and is followed
/// by a full rollback of the entities the merge created.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MergeError {
    /// A scene entity record has an empty `persistent_id`.
    #[error("merged scene contains an empty persistent_id")]
    EmptyPersistentId,
    /// The merged scene repeats a persistent ID internally (sorted, deduplicated).
    #[error("merged scene contains duplicate persistent_id(s): {}", .0.join(", "))]
    DuplicatePersistentIds(Vec<PersistentId>),
    /// Persistent IDs that already belong to live world entities (in scene order).
    ///
    /// Callers that intentionally combine scenes with overlapping IDs must
    /// pre-namespace one scene's IDs before merging.
    #[error(
        "merged scene persistent_id(s) already exist in the world: {}",
        .0.join(", ")
    )]
    ConflictingPersistentIds(Vec<PersistentId>),
    /// A parent reference that resolves neither inside the merged scene nor
    /// to a live world entity by persistent ID.
    #[error(
        "merged entity '{entity_id}' references parent '{parent_id}', which exists neither in the merged scene nor in the world"
    )]
    UnknownParent {
        entity_id: PersistentId,
        parent_id: PersistentId,
    },
    /// The merged scene's parent chain forms a cycle within its own entity set.
    #[error("merged scene hierarchy contains a parent cycle at entity '{0}'")]
    ParentCycle(PersistentId),
    /// Registry-strict component population produced diagnostics; the merge
    /// was rolled back and the world is unchanged.
    #[error("one or more merged scene components could not be restored")]
    ComponentLoad {
        diagnostics: Vec<SceneLoadDiagnostic>,
    },
}

/// Failure returned by [`World::destroy_subtree_by_persistent_ids`].
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DestroySubtreeError {
    /// A root ID was empty.
    #[error("persistent entity id cannot be empty")]
    EmptyPersistentId,
    /// Root IDs that do not resolve to a live persistent entity (sorted, deduplicated).
    #[error("unknown persistent entity id(s): {}", .0.join(", "))]
    UnknownPersistentIds(Vec<PersistentId>),
}

/// Outcome of [`World::destroy_subtree_by_persistent_ids`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DestroyedSubtree {
    /// Persistent IDs of every destroyed entity that had one (sorted, deduplicated).
    pub persistent_ids: Vec<PersistentId>,
    /// Total number of destroyed entities, including descendants that had no
    /// persistent ID.
    pub destroyed: usize,
}

impl World {
    /// Merge every entity of `scene` into this live world.
    ///
    /// This is the live-world counterpart of [`from_scene`](World::from_scene):
    /// the same two-pass allocate-then-populate flow runs against the existing
    /// world, so merged entities receive ordinary ECS handles and typed
    /// components, and their persistent IDs become resolvable through
    /// [`entity_by_persistent_id`](World::entity_by_persistent_id). Returns the
    /// created entities in scene order.
    ///
    /// Semantics:
    ///
    /// - **Persistent-ID conflicts are a hard error.** If any scene
    ///   `persistent_id` is empty, repeated inside the scene, or already
    ///   assigned to a live world entity, the merge fails listing the
    ///   conflicts and the world is not modified. Callers combining scenes
    ///   with overlapping IDs must pre-namespace one scene's IDs.
    /// - **Parenting.** `EntityRecord.parent` (and the legacy Transform
    ///   `parent` field) may reference an entity inside the merged scene or a
    ///   live world entity by persistent ID — the latter attaches merged
    ///   content under an existing world node. References to anything else
    ///   are rejected before mutation. Parent cycles inside the merged scene
    ///   are rejected; a merge cannot create a cycle through existing world
    ///   entities because it never re-parents them.
    /// - **Scene-level metadata is not merged.** The world's existing scene
    ///   settings, scene ID, schema version, and dependency list are left
    ///   untouched; merging is content insertion, not scene replacement.
    /// - **Component rules mirror scene loading.** Without a component
    ///   registry, unknown component records are tolerated exactly as in
    ///   [`from_scene`](World::from_scene). With a registry installed,
    ///   population is strict and any diagnostic fails the merge after
    ///   rolling back every entity it created. Scene-only component types
    ///   (for example `engine.script`) must be stripped or registered by the
    ///   caller first, just as project validation does for scene loads.
    pub fn merge_scene(&mut self, scene: &Scene) -> Result<Vec<Entity>, MergeError> {
        validate_merge(scene, self)?;
        let (created, diagnostics) = self.insert_scene_entities(scene);
        if diagnostics.is_empty() {
            return Ok(created);
        }
        // Roll back: a strict registry surfaced component failures, and a
        // half-merged subtree must not remain in the live world.
        for entity in created {
            self.destroy_entity(entity);
        }
        Err(MergeError::ComponentLoad { diagnostics })
    }

    /// Destroy the subtrees rooted at the given persistent IDs.
    ///
    /// Every named root must resolve to a live persistent entity; unknown or
    /// empty IDs fail before any mutation. Each root and all of its
    /// descendants — followed through both `Transform.parent` handles and the
    /// persistent `entity_parents` table, so entities without an enabled
    /// Transform are included — are destroyed through
    /// [`destroy_entity`](World::destroy_entity), which also cleans the
    /// persistent-ID maps. Overlapping roots are deduplicated. Entities
    /// outside the named subtrees are never destroyed; as with
    /// `destroy_entity`, any survivor whose parent reference pointed at a
    /// destroyed entity is re-rooted (its parent is cleared).
    pub fn destroy_subtree_by_persistent_ids(
        &mut self,
        root_ids: &[PersistentId],
    ) -> Result<DestroyedSubtree, DestroySubtreeError> {
        if root_ids.iter().any(|id| id.is_empty()) {
            return Err(DestroySubtreeError::EmptyPersistentId);
        }
        let mut roots = Vec::with_capacity(root_ids.len());
        let mut unknown = Vec::new();
        for id in root_ids {
            match self.entity_by_persistent_id(id) {
                Some(entity) => roots.push(entity),
                None => unknown.push(id.clone()),
            }
        }
        if !unknown.is_empty() {
            unknown.sort();
            unknown.dedup();
            return Err(DestroySubtreeError::UnknownPersistentIds(unknown));
        }

        // Parent -> children adjacency, built once so traversal stays linear.
        let mut children_of: HashMap<Entity, Vec<Entity>> = HashMap::new();
        for (child, transform) in self.query_all::<Transform>() {
            if let Some(parent) = transform.parent {
                children_of.entry(parent).or_default().push(child);
            }
        }
        for (idx, parent_id) in self.entity_parents.iter().enumerate() {
            let Some(parent_id) = parent_id else {
                continue;
            };
            let Some(parent) = self.persistent_to_entity.get(parent_id).copied() else {
                continue;
            };
            let Some(child) = self.entities.live_entity_at(idx as u32) else {
                continue;
            };
            children_of.entry(parent).or_default().push(child);
        }

        let mut members = Vec::new();
        let mut seen: HashSet<Entity> = HashSet::new();
        let mut frontier: Vec<Entity> = roots.into_iter().collect();
        while let Some(entity) = frontier.pop() {
            if !seen.insert(entity) {
                continue;
            }
            members.push(entity);
            if let Some(children) = children_of.get(&entity) {
                frontier.extend(children.iter().copied());
            }
        }

        let mut report = DestroyedSubtree::default();
        for entity in members {
            if let Some(id) = self.persistent_id(entity).map(str::to_owned) {
                report.persistent_ids.push(id);
            }
            if self.destroy_entity(entity) {
                report.destroyed += 1;
            }
        }
        report.persistent_ids.sort();
        report.persistent_ids.dedup();
        Ok(report)
    }
}

/// Validate a merge before any world mutation.
///
/// Rejects empty/duplicate/conflicting persistent IDs, unresolvable parent
/// references, and parent cycles inside the merged scene. Component-level
/// problems are not validated here; they surface during population.
fn validate_merge(scene: &Scene, world: &World) -> Result<(), MergeError> {
    let mut scene_ids = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for record in &scene.entities {
        if record.persistent_id.is_empty() {
            return Err(MergeError::EmptyPersistentId);
        }
        if !scene_ids.insert(record.persistent_id.as_str()) {
            duplicates.insert(record.persistent_id.clone());
        }
    }
    if !duplicates.is_empty() {
        return Err(MergeError::DuplicatePersistentIds(
            duplicates.into_iter().collect(),
        ));
    }

    let conflicts: Vec<PersistentId> = scene
        .entities
        .iter()
        .filter(|record| {
            world
                .persistent_to_entity
                .contains_key(&record.persistent_id)
        })
        .map(|record| record.persistent_id.clone())
        .collect();
    if !conflicts.is_empty() {
        return Err(MergeError::ConflictingPersistentIds(conflicts));
    }

    let parent_exists = |world: &World, parent_id: &str| {
        scene_ids.contains(parent_id) || world.entity_by_persistent_id(parent_id).is_some()
    };
    for record in &scene.entities {
        if let Some(parent_id) = record.parent.as_ref() {
            if !parent_exists(world, parent_id) {
                return Err(MergeError::UnknownParent {
                    entity_id: record.persistent_id.clone(),
                    parent_id: parent_id.clone(),
                });
            }
        }
        // Older scenes may encode the parent only inside an enabled
        // Transform record; apply the same resolution rule to that field.
        if let Some(transform) = record.components.get(Transform::TYPE_ID) {
            if transform.enabled {
                if let Some(Value::Entity(parent_id)) = transform.fields.get("parent") {
                    if !parent_exists(world, parent_id) {
                        return Err(MergeError::UnknownParent {
                            entity_id: record.persistent_id.clone(),
                            parent_id: parent_id.clone(),
                        });
                    }
                }
            }
        }
    }

    detect_parent_cycle(scene, &scene_ids)
}

/// Reject parent cycles inside the merged scene's own entity set.
///
/// A merge cannot create a cycle through existing world entities because it
/// never re-parents them, so only intra-scene chains need checking.
fn detect_parent_cycle(scene: &Scene, scene_ids: &BTreeSet<&str>) -> Result<(), MergeError> {
    let parent_of: BTreeMap<&str, &str> = scene
        .entities
        .iter()
        .filter_map(|record| {
            let parent = record.parent.as_deref()?;
            scene_ids
                .contains(parent)
                .then_some((record.persistent_id.as_str(), parent))
        })
        .collect();

    let mut cleared: BTreeSet<&str> = BTreeSet::new();
    for &start in parent_of.keys() {
        if cleared.contains(start) {
            continue;
        }
        let mut chain: BTreeSet<&str> = BTreeSet::new();
        let mut cursor = start;
        while let Some(&parent) = parent_of.get(cursor) {
            if !chain.insert(cursor) {
                return Err(MergeError::ParentCycle(cursor.to_string()));
            }
            if cleared.contains(parent) {
                break;
            }
            cursor = parent;
        }
        cleared.extend(chain);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use engine_serialize::{AssetId, SchemaVersion, Value};

    use super::{DestroySubtreeError, MergeError};
    use crate::components::{Name, Transform};
    use crate::registry::ComponentRegistry;
    use crate::scene::{sample_scene, ComponentRecord, EntityRecord, Scene, SceneLoadDiagnostic};
    use crate::{Component, World};

    fn record(id: &str, parent: Option<&str>) -> EntityRecord {
        EntityRecord {
            persistent_id: id.to_string(),
            parent: parent.map(str::to_string),
            name: Some(id.to_string()),
            enabled: true,
            components: BTreeMap::new(),
        }
    }

    fn transform_record(fields: BTreeMap<String, Value>) -> ComponentRecord {
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields,
        }
    }

    fn scene_with_records(id: &str, records: Vec<EntityRecord>) -> Scene {
        let mut scene = sample_scene();
        scene.scene_id = id.to_string();
        scene.entities = records;
        scene
    }

    #[test]
    fn merge_scene_into_empty_world_matches_from_scene() {
        let scene = sample_scene();
        let merged_world = {
            let mut world = World::new();
            let created = world.merge_scene(&scene).expect("merge into empty world");
            assert_eq!(created.len(), scene.entities.len());
            world
        };
        let loaded_world = World::from_scene(&scene);

        assert_eq!(merged_world.alive_count(), loaded_world.alive_count());
        let mut merged_entities = merged_world.to_scene().entities;
        let mut loaded_entities = loaded_world.to_scene().entities;
        merged_entities.sort_by(|a, b| a.persistent_id.cmp(&b.persistent_id));
        loaded_entities.sort_by(|a, b| a.persistent_id.cmp(&b.persistent_id));
        assert_eq!(merged_entities, loaded_entities);
    }

    #[test]
    fn merge_scene_adds_disjoint_scene_to_live_world() {
        let mut world = World::from_scene(&sample_scene());
        let second = scene_with_records(
            "cell-east",
            vec![
                record("east-root", None),
                record("east-child", Some("east-root")),
            ],
        );

        let created = world.merge_scene(&second).expect("disjoint merge");
        assert_eq!(created.len(), 2);
        assert_eq!(world.alive_count(), 4);
        for id in ["camera-main", "cube-01", "east-root", "east-child"] {
            assert!(
                world.entity_by_persistent_id(id).is_some(),
                "missing entity {id}"
            );
        }
        let root = world.entity_by_persistent_id("east-root").expect("root");
        let child = world.entity_by_persistent_id("east-child").expect("child");
        assert_eq!(
            world.get::<Name>(child).map(|n| n.0.as_str()),
            Some("east-child")
        );
        // Records without Transform keep their hierarchy in the side table.
        let roundtripped = world.to_scene();
        let child_record = roundtripped
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "east-child")
            .expect("child record");
        assert_eq!(child_record.parent.as_deref(), Some("east-root"));
        assert!(world.get::<Transform>(root).is_none());
    }

    #[test]
    fn merge_scene_rejects_conflicting_ids_without_mutating_world() {
        let mut world = World::from_scene(&sample_scene());
        let cube = world.entity_by_persistent_id("cube-01").expect("cube");
        let conflicting = scene_with_records(
            "cell-copy",
            vec![
                record("cube-01", None),
                record("cube-01-copy", Some("cube-01")),
            ],
        );

        let error = world.merge_scene(&conflicting).expect_err("conflict");
        assert_eq!(
            error,
            MergeError::ConflictingPersistentIds(vec!["cube-01".to_string()])
        );
        assert_eq!(world.alive_count(), 2);
        assert_eq!(world.entity_by_persistent_id("cube-01"), Some(cube));
        assert!(world.entity_by_persistent_id("cube-01-copy").is_none());
    }

    #[test]
    fn merge_scene_rejects_duplicate_and_empty_ids() {
        let mut world = World::new();
        let duplicated = scene_with_records(
            "dup",
            vec![
                record("same", None),
                record("same", None),
                record("same", None),
            ],
        );
        assert_eq!(
            world.merge_scene(&duplicated).expect_err("duplicates"),
            MergeError::DuplicatePersistentIds(vec!["same".to_string()])
        );
        assert_eq!(world.alive_count(), 0);

        let mut empty = scene_with_records("empty", vec![record("ok", None)]);
        empty.entities.push(record("", None));
        assert_eq!(
            world.merge_scene(&empty).expect_err("empty id"),
            MergeError::EmptyPersistentId
        );
        assert_eq!(world.alive_count(), 0);
    }

    #[test]
    fn merge_scene_remaps_parents_within_merged_set() {
        let mut world = World::from_scene(&sample_scene());
        let mut child = record("merged-child", Some("merged-parent"));
        child.components.insert(
            Transform::TYPE_ID.to_string(),
            transform_record(BTreeMap::new()),
        );
        let scene = scene_with_records("cell", vec![record("merged-parent", None), child]);

        world.merge_scene(&scene).expect("parented merge");
        let parent = world
            .entity_by_persistent_id("merged-parent")
            .expect("parent");
        let child = world
            .entity_by_persistent_id("merged-child")
            .expect("child");
        assert_eq!(
            world.get::<Transform>(child).and_then(|t| t.parent),
            Some(parent)
        );

        let roundtripped = world.to_scene();
        let child_record = roundtripped
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "merged-child")
            .expect("child record");
        assert_eq!(
            child_record.parent.as_deref(),
            Some("merged-parent"),
            "remapped parent must serialize back to its persistent id"
        );
    }

    #[test]
    fn merge_scene_allows_parenting_onto_existing_world_entity() {
        let mut world = World::from_scene(&sample_scene());
        let mut attached = record("attached-turret", Some("cube-01"));
        attached.components.insert(
            Transform::TYPE_ID.to_string(),
            transform_record(BTreeMap::new()),
        );
        let scene = scene_with_records("attachments", vec![attached]);

        world.merge_scene(&scene).expect("cross-scene attachment");
        let cube = world.entity_by_persistent_id("cube-01").expect("cube");
        let turret = world
            .entity_by_persistent_id("attached-turret")
            .expect("turret");
        assert_eq!(
            world.get::<Transform>(turret).and_then(|t| t.parent),
            Some(cube)
        );
    }

    #[test]
    fn merge_scene_rejects_unknown_parents_without_mutating_world() {
        let mut world = World::from_scene(&sample_scene());
        let scene = scene_with_records("broken", vec![record("orphan", Some("missing-parent"))]);

        let error = world.merge_scene(&scene).expect_err("unknown parent");
        assert_eq!(
            error,
            MergeError::UnknownParent {
                entity_id: "orphan".to_string(),
                parent_id: "missing-parent".to_string(),
            }
        );
        assert_eq!(world.alive_count(), 2);
        assert!(world.entity_by_persistent_id("orphan").is_none());
    }

    #[test]
    fn merge_scene_rejects_unknown_parent_in_transform_field() {
        let mut world = World::new();
        let mut broken = record("legacy-orphan", None);
        broken.components.insert(
            Transform::TYPE_ID.to_string(),
            transform_record(BTreeMap::from([(
                "parent".to_string(),
                Value::Entity("not-anywhere".to_string()),
            )])),
        );
        let scene = scene_with_records("legacy", vec![broken]);

        let error = world
            .merge_scene(&scene)
            .expect_err("unknown transform parent");
        assert_eq!(
            error,
            MergeError::UnknownParent {
                entity_id: "legacy-orphan".to_string(),
                parent_id: "not-anywhere".to_string(),
            }
        );
        assert_eq!(world.alive_count(), 0);
    }

    #[test]
    fn merge_scene_rejects_parent_cycles() {
        let mut world = World::new();
        let cyclic = scene_with_records(
            "cyclic",
            vec![record("a", Some("b")), record("b", Some("a"))],
        );
        assert!(matches!(
            world.merge_scene(&cyclic),
            Err(MergeError::ParentCycle(_))
        ));
        assert_eq!(world.alive_count(), 0);

        let self_parented = scene_with_records("self", vec![record("loop", Some("loop"))]);
        assert!(matches!(
            world.merge_scene(&self_parented),
            Err(MergeError::ParentCycle(_))
        ));
        assert_eq!(world.alive_count(), 0);
    }

    #[test]
    fn merge_scene_preserves_world_scene_metadata() {
        let mut world = World::from_scene(&sample_scene());
        let original_settings = world.scene_settings().clone();
        let mut second = scene_with_records("cell-west", vec![record("west-root", None)]);
        second.scene_settings.gravity = Some([0.0, -3.7, 0.0]);
        second.scene_settings.ambient = [1.0, 1.0, 1.0, 1.0];
        second.dependencies = vec![AssetId::new("west-only-asset")];

        world.merge_scene(&second).expect("merge");
        assert_eq!(world.scene_settings(), &original_settings);
        assert_eq!(world.scene_id, "scene-gate04-valid");
        assert!(!world
            .scene_dependencies
            .iter()
            .any(|dependency| dependency.id == "west-only-asset"));
    }

    #[test]
    fn merge_scene_rolls_back_when_strict_registry_rejects_a_component() {
        let mut world = World::from_scene(&sample_scene());
        world.set_shared_component_registry(Arc::new(ComponentRegistry::new()));
        let alive_before = world.alive_count();

        let mut broken = record("strict-broken", None);
        broken.components.insert(
            "test.unknown".to_string(),
            transform_record(BTreeMap::from([("value".to_string(), Value::UInt(1))])),
        );
        let scene = scene_with_records("strict-cell", vec![record("strict-ok", None), broken]);

        let error = world.merge_scene(&scene).expect_err("strict failure");
        assert!(matches!(
            error,
            MergeError::ComponentLoad { ref diagnostics } if diagnostics.iter().any(
                |diagnostic| matches!(
                    diagnostic,
                    SceneLoadDiagnostic::UnknownComponent { component_type_id, .. }
                        if component_type_id == "test.unknown"
                )
            )
        ));
        // Full rollback: neither the broken entity nor its valid sibling stays.
        assert_eq!(world.alive_count(), alive_before);
        assert!(world.entity_by_persistent_id("strict-broken").is_none());
        assert!(world.entity_by_persistent_id("strict-ok").is_none());
    }

    #[test]
    fn merge_scene_keeps_disabled_component_records_for_roundtrip() {
        let mut world = World::new();
        let mut disabled = record("disabled-holder", None);
        let mut disabled_record = transform_record(BTreeMap::new());
        disabled_record.enabled = false;
        disabled_record.schema_version = SchemaVersion::new(4, 5, 6);
        disabled
            .components
            .insert("test.disabled".to_string(), disabled_record.clone());
        let scene = scene_with_records("cell", vec![disabled]);

        world
            .merge_scene(&scene)
            .expect("merge with disabled record");
        let roundtripped = world.to_scene();
        let holder = roundtripped
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "disabled-holder")
            .expect("holder record");
        assert_eq!(
            holder.components.get("test.disabled"),
            Some(&disabled_record)
        );
    }

    #[test]
    fn merged_world_roundtrips_through_to_scene_with_both_scenes() {
        let mut world = World::from_scene(&sample_scene());
        let second = scene_with_records(
            "cell-south",
            vec![
                record("south-root", None),
                record("south-leaf", Some("south-root")),
            ],
        );
        world.merge_scene(&second).expect("merge");

        let roundtripped = world.to_scene();
        assert_eq!(roundtripped.entities.len(), 4);
        for id in ["camera-main", "cube-01", "south-root", "south-leaf"] {
            assert!(
                roundtripped
                    .entities
                    .iter()
                    .any(|entity| entity.persistent_id == id),
                "missing roundtripped entity {id}"
            );
        }
        let diagnostics = crate::validation::validate_scene(&roundtripped);
        assert!(
            diagnostics.is_empty(),
            "merged roundtrip must remain a valid scene: {diagnostics:?}"
        );
    }

    fn hierarchy_scene() -> Scene {
        let with_transform = |id: &str, parent: Option<&str>| {
            let mut entity = record(id, parent);
            entity.components.insert(
                Transform::TYPE_ID.to_string(),
                transform_record(BTreeMap::new()),
            );
            entity
        };
        scene_with_records(
            "hierarchy",
            vec![
                with_transform("root", None),
                with_transform("child", Some("root")),
                with_transform("grandchild", Some("child")),
                with_transform("bystander", None),
            ],
        )
    }

    #[test]
    fn destroy_subtree_removes_descendants_and_cleans_persistent_maps() {
        let mut world = World::from_scene(&hierarchy_scene());

        let report = world
            .destroy_subtree_by_persistent_ids(&["root".to_string()])
            .expect("destroy root subtree");
        assert_eq!(report.destroyed, 3);
        assert_eq!(
            report.persistent_ids,
            vec![
                "child".to_string(),
                "grandchild".to_string(),
                "root".to_string()
            ]
        );
        assert_eq!(world.alive_count(), 1);
        for id in ["root", "child", "grandchild"] {
            assert!(world.entity_by_persistent_id(id).is_none(), "{id} survived");
        }
        assert!(world.entity_by_persistent_id("bystander").is_some());
        assert!(world.persistent_entities().all(|(id, _)| id == "bystander"));
    }

    #[test]
    fn destroy_subtree_validates_ids_before_mutating() {
        let mut world = World::from_scene(&hierarchy_scene());

        assert_eq!(
            world.destroy_subtree_by_persistent_ids(&[String::new()]),
            Err(DestroySubtreeError::EmptyPersistentId)
        );
        assert_eq!(
            world.destroy_subtree_by_persistent_ids(&[
                "root".to_string(),
                "missing".to_string(),
                "missing".to_string(),
            ]),
            Err(DestroySubtreeError::UnknownPersistentIds(vec![
                "missing".to_string()
            ]))
        );
        assert_eq!(world.alive_count(), 4);
        assert!(world.entity_by_persistent_id("root").is_some());
    }

    #[test]
    fn destroy_subtree_includes_transformless_descendants() {
        // `child` has no Transform: its hierarchy link only lives in the
        // persistent entity_parents table.
        let scene = scene_with_records(
            "transformless",
            vec![
                record("root", None),
                record("child", Some("root")),
                record("leaf", Some("child")),
            ],
        );
        let mut world = World::from_scene(&scene);
        assert!(world
            .get::<Transform>(world.entity_by_persistent_id("child").expect("child"))
            .is_none());

        let report = world
            .destroy_subtree_by_persistent_ids(&["root".to_string()])
            .expect("destroy transformless subtree");
        assert_eq!(report.destroyed, 3);
        assert_eq!(world.alive_count(), 0);
        assert!(world.persistent_entities().next().is_none());
    }

    #[test]
    fn destroy_subtree_deduplicates_overlapping_roots() {
        let mut world = World::from_scene(&hierarchy_scene());

        let report = world
            .destroy_subtree_by_persistent_ids(&["root".to_string(), "child".to_string()])
            .expect("overlapping roots");
        assert_eq!(report.destroyed, 3);
        assert_eq!(report.persistent_ids.len(), 3);
        assert_eq!(world.alive_count(), 1);
    }

    #[test]
    fn destroy_subtree_releases_persistent_ids_for_reuse() {
        let mut world = World::from_scene(&hierarchy_scene());
        world
            .destroy_subtree_by_persistent_ids(&["child".to_string()])
            .expect("destroy child subtree");
        assert_eq!(report_free_ids(&world), vec!["bystander", "root"]);

        let reused = world
            .create_persistent_entity("child")
            .expect("destroyed id is reusable");
        assert_eq!(world.persistent_id(reused), Some("child"));
        // The old root keeps its identity and hierarchy state.
        let root = world.entity_by_persistent_id("root").expect("root");
        assert_eq!(world.get::<Transform>(root).and_then(|t| t.parent), None);
    }

    fn report_free_ids(world: &World) -> Vec<String> {
        world
            .persistent_entities()
            .map(|(id, _)| id.to_string())
            .collect()
    }

    #[test]
    fn destroy_subtree_roundtrips_remaining_world_through_to_scene() {
        let mut world = World::from_scene(&sample_scene());
        let second = scene_with_records(
            "cell-north",
            vec![
                record("north-root", None),
                record("north-leaf", Some("north-root")),
            ],
        );
        world.merge_scene(&second).expect("merge");

        let report = world
            .destroy_subtree_by_persistent_ids(&["north-root".to_string()])
            .expect("unload cell");
        assert_eq!(report.destroyed, 2);

        let roundtripped = world.to_scene();
        assert_eq!(roundtripped.entities.len(), 2);
        assert!(roundtripped
            .entities
            .iter()
            .all(|entity| !entity.persistent_id.starts_with("north-")));
        let diagnostics = crate::validation::validate_scene(&roundtripped);
        assert!(
            diagnostics.is_empty(),
            "world must stay valid after subtree unload: {diagnostics:?}"
        );
    }
}
