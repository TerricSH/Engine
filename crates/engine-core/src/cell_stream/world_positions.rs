use std::collections::{BTreeSet, HashMap, HashSet};

use engine_scene::{Entity, Scene, World};
use engine_serialize::PersistentId;
use glam::Vec3;

/// Hierarchy roots of a cell scene: entity records with no parent or with a
/// parent outside the scene's own persistent-ID set.
pub(super) fn cell_root_ids(scene: &Scene) -> Vec<PersistentId> {
    let own_ids: BTreeSet<&str> = scene
        .entities
        .iter()
        .map(|entity| entity.persistent_id.as_str())
        .collect();
    scene
        .entities
        .iter()
        .filter(|entity| {
            entity
                .parent
                .as_deref()
                .is_none_or(|parent| !own_ids.contains(parent))
        })
        .map(|entity| entity.persistent_id.clone())
        .collect()
}

/// Memoised world-space positions for `Transform`-bearing entities, resolved
/// by walking `Transform.parent` chains. Entities without a Transform resolve
/// to `None`; an ancestor without a Transform counts as an identity root and
/// a parent cycle breaks the chain, mirroring extraction's tolerance.
#[derive(Default)]
pub(super) struct WorldPositions {
    resolved: HashMap<Entity, Option<Vec3>>,
}

impl WorldPositions {
    pub(super) fn position(&mut self, world: &World, entity: Entity) -> Option<Vec3> {
        if let Some(cached) = self.resolved.get(&entity) {
            return *cached;
        }
        let mut visiting = HashSet::new();
        let position = self
            .matrix(world, entity, &mut visiting, true)
            .map(|matrix| matrix.transform_point3(Vec3::ZERO));
        self.resolved.insert(entity, position);
        position
    }

    /// World matrix of `entity`. Returns `None` only when the queried entity
    /// itself has no Transform; missing ancestors and cycles degrade to
    /// identity roots so a position is still produced.
    fn matrix(
        &mut self,
        world: &World,
        entity: Entity,
        visiting: &mut HashSet<Entity>,
        root_query: bool,
    ) -> Option<glam::Mat4> {
        use engine_scene::components::Transform;

        let Some(transform) = world.get::<Transform>(entity) else {
            return if root_query {
                None
            } else {
                Some(glam::Mat4::IDENTITY)
            };
        };
        let local = glam::Mat4::from_scale_rotation_translation(
            transform.scale,
            transform.rotation,
            transform.translation,
        );
        let Some(parent) = transform.parent else {
            return Some(local);
        };
        if !visiting.insert(entity) {
            return None;
        }
        let parent_matrix = self.matrix(world, parent, visiting, false);
        visiting.remove(&entity);
        Some(parent_matrix.unwrap_or(glam::Mat4::IDENTITY) * local)
    }
}
