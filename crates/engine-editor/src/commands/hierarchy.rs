use super::*;

// -------------------------------------------------------------------
// Sibling order
// -------------------------------------------------------------------

/// A relative hierarchy-order operation matching Unity's sibling commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiblingMove {
    Up,
    Down,
    First,
    Last,
}

struct SiblingOrderSnapshot {
    parent: Option<PersistentId>,
    before: Vec<PersistentId>,
    after: Vec<PersistentId>,
}

/// Reorders an entity among records with the same parent while leaving every
/// unrelated serialized entity slot untouched.
pub struct MoveEntitySibling {
    entity_id: PersistentId,
    movement: SiblingMove,
    snapshot: Option<SiblingOrderSnapshot>,
}

impl MoveEntitySibling {
    pub fn new(entity_id: PersistentId, movement: SiblingMove) -> Self {
        Self {
            entity_id,
            movement,
            snapshot: None,
        }
    }

    fn capture(&self, scene: &Scene) -> Result<SiblingOrderSnapshot, EditorError> {
        let entity = scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == self.entity_id)
            .ok_or_else(|| EditorError::EntityNotFound(self.entity_id.clone()))?;
        let parent = entity.parent.clone();
        let before = sibling_ids(scene, parent.as_ref());
        let current = before
            .iter()
            .position(|id| id == &self.entity_id)
            .ok_or_else(|| EditorError::EntityNotFound(self.entity_id.clone()))?;
        let destination = match self.movement {
            SiblingMove::Up => current.checked_sub(1),
            SiblingMove::Down => (current + 1 < before.len()).then_some(current + 1),
            SiblingMove::First => (current != 0).then_some(0),
            SiblingMove::Last => (current + 1 != before.len()).then_some(before.len() - 1),
        }
        .ok_or_else(|| {
            EditorError::InvalidHierarchy(format!(
                "entity '{}' is already at the requested sibling boundary",
                self.entity_id
            ))
        })?;
        let mut after = before.clone();
        let moved = after.remove(current);
        after.insert(destination, moved);
        Ok(SiblingOrderSnapshot {
            parent,
            before,
            after,
        })
    }
}

impl Command for MoveEntitySibling {
    fn name(&self) -> &str {
        match self.movement {
            SiblingMove::Up => "Move Entity Up",
            SiblingMove::Down => "Move Entity Down",
            SiblingMove::First => "Move Entity to First Sibling",
            SiblingMove::Last => "Move Entity to Last Sibling",
        }
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if self.snapshot.is_none() {
            self.snapshot = Some(self.capture(scene)?);
        }
        let snapshot = self.snapshot.as_ref().expect("captured above");
        apply_sibling_order(
            scene,
            snapshot.parent.as_ref(),
            &snapshot.before,
            &snapshot.after,
        )
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or_else(|| EditorError::InvalidHierarchy("sibling move was not executed".into()))?;
        apply_sibling_order(
            scene,
            snapshot.parent.as_ref(),
            &snapshot.after,
            &snapshot.before,
        )
    }
}

pub(super) fn sibling_ids(scene: &Scene, parent: Option<&PersistentId>) -> Vec<PersistentId> {
    scene
        .entities
        .iter()
        .filter(|entity| entity.parent.as_ref() == parent)
        .map(|entity| entity.persistent_id.clone())
        .collect()
}

fn apply_sibling_order(
    scene: &mut Scene,
    parent: Option<&PersistentId>,
    expected: &[PersistentId],
    desired: &[PersistentId],
) -> Result<(), EditorError> {
    let indices = scene
        .entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| entity.parent.as_ref() == parent)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let current = indices
        .iter()
        .map(|index| scene.entities[*index].persistent_id.clone())
        .collect::<Vec<_>>();
    if current != expected {
        return Err(EditorError::InvalidHierarchy(
            "sibling order changed before command application".into(),
        ));
    }
    if desired.len() != indices.len()
        || desired.iter().cloned().collect::<BTreeSet<_>>()
            != expected.iter().cloned().collect::<BTreeSet<_>>()
    {
        return Err(EditorError::InvalidHierarchy(
            "sibling order command does not contain the same entity set".into(),
        ));
    }
    let records = indices
        .iter()
        .map(|index| {
            let entity = &scene.entities[*index];
            (entity.persistent_id.clone(), entity.clone())
        })
        .collect::<BTreeMap<_, _>>();
    let replacements = desired
        .iter()
        .map(|id| {
            records.get(id).cloned().ok_or_else(|| {
                EditorError::InvalidHierarchy(format!("missing sibling record '{id}'"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (index, replacement) in indices.into_iter().zip(replacements) {
        scene.entities[index] = replacement;
    }
    Ok(())
}

// -------------------------------------------------------------------
// SetEntityName
// -------------------------------------------------------------------

pub struct SetEntityName {
    entity_id: PersistentId,
    old_name: Option<String>,
    new_name: Option<String>,
}

impl SetEntityName {
    pub fn new(entity_id: PersistentId, new_name: Option<String>) -> Self {
        Self {
            entity_id,
            old_name: None,
            new_name,
        }
    }
}

impl Command for SetEntityName {
    fn name(&self) -> &str {
        "Set Entity Name"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        self.old_name = entity.name.clone();
        entity.name = self.new_name.clone();
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        entity.name = self.old_name.clone();
        Ok(())
    }
}

// -------------------------------------------------------------------
// Hierarchy parenting
// -------------------------------------------------------------------

pub struct SetEntityParent {
    entity_id: PersistentId,
    old_parent: Option<PersistentId>,
    new_parent: Option<PersistentId>,
}

impl SetEntityParent {
    pub fn new(entity_id: PersistentId, new_parent: Option<PersistentId>) -> Self {
        Self {
            entity_id,
            old_parent: None,
            new_parent,
        }
    }
}

impl Command for SetEntityParent {
    fn name(&self) -> &str {
        "Set Entity Parent"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if self.new_parent.as_ref() == Some(&self.entity_id) {
            return Err(EditorError::InvalidHierarchy(
                "an entity cannot parent itself".to_string(),
            ));
        }
        if let Some(parent) = &self.new_parent {
            if !scene
                .entities
                .iter()
                .any(|entity| &entity.persistent_id == parent)
            {
                return Err(EditorError::EntityNotFound(parent.clone()));
            }
            if collect_descendant_ids(scene, &self.entity_id).contains(parent) {
                return Err(EditorError::InvalidHierarchy(format!(
                    "parenting '{}' under '{}' would create a cycle",
                    self.entity_id, parent
                )));
            }
        }
        let entity = find_entity_mut(scene, &self.entity_id)?;
        self.old_parent = entity.parent.clone();
        entity.parent = self.new_parent.clone();
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        find_entity_mut(scene, &self.entity_id)?.parent = self.old_parent.clone();
        Ok(())
    }
}

// -------------------------------------------------------------------
// Enabled state
// -------------------------------------------------------------------

pub struct SetEntityEnabled {
    entity_id: PersistentId,
    old_enabled: Option<bool>,
    new_enabled: bool,
}

impl SetEntityEnabled {
    pub fn new(entity_id: PersistentId, new_enabled: bool) -> Self {
        Self {
            entity_id,
            old_enabled: None,
            new_enabled,
        }
    }
}

impl Command for SetEntityEnabled {
    fn name(&self) -> &str {
        "Set Entity Enabled"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        self.old_enabled = Some(entity.enabled);
        entity.enabled = self.new_enabled;
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        if let Some(old_enabled) = self.old_enabled {
            entity.enabled = old_enabled;
        }
        Ok(())
    }
}

pub struct SetComponentEnabled {
    entity_id: PersistentId,
    component_type: ComponentTypeId,
    old_enabled: Option<bool>,
    new_enabled: bool,
}

impl SetComponentEnabled {
    pub fn new(
        entity_id: PersistentId,
        component_type: ComponentTypeId,
        new_enabled: bool,
    ) -> Self {
        Self {
            entity_id,
            component_type,
            old_enabled: None,
            new_enabled,
        }
    }
}

impl Command for SetComponentEnabled {
    fn name(&self) -> &str {
        "Set Component Enabled"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        let component = entity
            .components
            .get_mut(&self.component_type)
            .ok_or_else(|| EditorError::ComponentNotFound(self.component_type.clone()))?;
        self.old_enabled = Some(component.enabled);
        component.enabled = self.new_enabled;
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        let component = entity
            .components
            .get_mut(&self.component_type)
            .ok_or_else(|| EditorError::ComponentNotFound(self.component_type.clone()))?;
        if let Some(old_enabled) = self.old_enabled {
            component.enabled = old_enabled;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------
// AddEntity
// -------------------------------------------------------------------

pub struct AddEntity {
    entity: EntityRecord,
    insertion_index: Option<usize>,
}

impl AddEntity {
    pub fn new(entity: EntityRecord) -> Self {
        Self {
            entity,
            insertion_index: None,
        }
    }
}

impl Command for AddEntity {
    fn name(&self) -> &str {
        "Add Entity"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if self.entity.persistent_id.is_empty() {
            return Err(EditorError::InvalidHierarchy(
                "entity persistent ID cannot be empty".into(),
            ));
        }
        if scene
            .entities
            .iter()
            .any(|entity| entity.persistent_id == self.entity.persistent_id)
        {
            return Err(EditorError::EntityAlreadyExists(
                self.entity.persistent_id.clone(),
            ));
        }
        if self.entity.parent.as_ref() == Some(&self.entity.persistent_id) {
            return Err(EditorError::InvalidHierarchy(
                "an entity cannot parent itself".into(),
            ));
        }
        if let Some(parent_id) = &self.entity.parent {
            if !scene
                .entities
                .iter()
                .any(|entity| &entity.persistent_id == parent_id)
            {
                return Err(EditorError::EntityNotFound(parent_id.clone()));
            }
        }
        let index = self.insertion_index.unwrap_or(scene.entities.len());
        if index > scene.entities.len() {
            return Err(EditorError::InvalidHierarchy(
                "entity insertion point is no longer valid".into(),
            ));
        }
        self.insertion_index = Some(index);
        scene.entities.insert(index, self.entity.clone());
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let index = self.insertion_index.ok_or_else(|| {
            EditorError::InvalidHierarchy("entity addition was not executed".into())
        })?;
        if scene.entities.get(index) != Some(&self.entity) {
            return Err(EditorError::InvalidHierarchy(
                "added entity changed before undo".into(),
            ));
        }
        scene.entities.remove(index);
        Ok(())
    }
}

// -------------------------------------------------------------------
// RemoveEntity (with recursive child removal)
// -------------------------------------------------------------------

pub struct RemoveEntity {
    entity_id: PersistentId,
    removed: Vec<(usize, EntityRecord)>,
    active_camera: Option<PersistentId>,
    captured: bool,
}

impl RemoveEntity {
    /// Create a deferred removal. The subtree is captured on first execution,
    /// after all earlier ordered editor actions have reached the scene.
    pub fn new(entity_id: PersistentId) -> Self {
        Self {
            entity_id,
            removed: Vec::new(),
            active_camera: None,
            captured: false,
        }
    }
}

impl Command for RemoveEntity {
    fn name(&self) -> &str {
        "Remove Entity"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if !self.captured {
            let target_index = scene
                .entities
                .iter()
                .position(|entity| entity.persistent_id == self.entity_id)
                .ok_or_else(|| EditorError::EntityNotFound(self.entity_id.clone()))?;
            let mut removed_ids = BTreeSet::from([self.entity_id.clone()]);
            removed_ids.extend(collect_descendant_ids(scene, &self.entity_id));
            self.removed = scene
                .entities
                .iter()
                .enumerate()
                .filter(|(_, entity)| removed_ids.contains(&entity.persistent_id))
                .map(|(index, entity)| (index, entity.clone()))
                .collect();
            debug_assert!(self.removed.iter().any(|(index, _)| *index == target_index));
            self.active_camera = scene.scene_settings.active_camera.clone();
            self.captured = true;
        }
        for (index, record) in &self.removed {
            if scene.entities.get(*index) != Some(record) {
                return Err(EditorError::InvalidHierarchy(
                    "entity subtree changed before removal".into(),
                ));
            }
        }
        if scene
            .scene_settings
            .active_camera
            .as_ref()
            .is_some_and(|camera| {
                self.removed
                    .iter()
                    .any(|(_, entity)| &entity.persistent_id == camera)
            })
        {
            scene.scene_settings.active_camera = None;
        }
        for (index, _) in self.removed.iter().rev() {
            scene.entities.remove(*index);
        }
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if !self.captured {
            return Err(EditorError::InvalidHierarchy(
                "entity removal was not executed".into(),
            ));
        }
        for (_, record) in &self.removed {
            if scene
                .entities
                .iter()
                .any(|entity| entity.persistent_id == record.persistent_id)
            {
                return Err(EditorError::EntityAlreadyExists(
                    record.persistent_id.clone(),
                ));
            }
        }
        let mut projected_len = scene.entities.len();
        for (index, _) in &self.removed {
            if *index > projected_len {
                return Err(EditorError::InvalidHierarchy(
                    "removed entity insertion point is no longer valid".into(),
                ));
            }
            projected_len += 1;
        }
        for (index, record) in &self.removed {
            scene.entities.insert(*index, record.clone());
        }
        scene.scene_settings.active_camera = self.active_camera.clone();
        Ok(())
    }
}
