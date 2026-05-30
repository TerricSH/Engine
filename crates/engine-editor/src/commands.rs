use engine_scene::{ComponentRecord, EntityRecord, Scene};
use engine_serialize::{ComponentTypeId, PersistentId, Value};

use crate::EditorError;

// -------------------------------------------------------------------
// Command trait
// -------------------------------------------------------------------

/// A single undoable operation on a [`Scene`].
pub trait Command: Send {
    /// Human-readable label (shown in the undo stack UI).
    fn name(&self) -> &str;

    /// Apply the forward transformation to `scene`.
    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError>;

    /// Revert the transformation, restoring `scene` to the state before
    /// [`execute`] was called.
    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError>;
}

// -------------------------------------------------------------------
// CommandHistory – undo / redo stack
// -------------------------------------------------------------------

/// Tracks a linear undo/redo history of [`Command`]s and a dirty flag.
pub struct CommandHistory {
    pub(crate) undone: Vec<Box<dyn Command>>,
    pub(crate) done: Vec<Box<dyn Command>>,
    max_undo: usize,
    dirty: bool,
}

impl CommandHistory {
    /// Create an empty history with a default undo limit of 256.
    pub fn new() -> Self {
        Self {
            undone: Vec::new(),
            done: Vec::new(),
            max_undo: 256,
            dirty: false,
        }
    }

    /// Execute `cmd` on `scene`, push it onto the done stack, and clear
    /// the redo stack.
    pub fn push(
        &mut self,
        mut cmd: Box<dyn Command>,
        scene: &mut Scene,
    ) -> Result<(), EditorError> {
        cmd.execute(scene)?;
        self.done.push(cmd);
        self.undone.clear();
        self.dirty = true;

        // Trim the oldest commands when we exceed the limit.
        while self.done.len() > self.max_undo {
            self.done.remove(0);
        }
        Ok(())
    }

    /// Undo the most recent command.
    pub fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if let Some(mut cmd) = self.done.pop() {
            cmd.undo(scene)?;
            self.undone.push(cmd);
            self.dirty = true;
        }
        Ok(())
    }

    /// Redo the last-undone command.
    pub fn redo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if let Some(mut cmd) = self.undone.pop() {
            cmd.execute(scene)?;
            self.done.push(cmd);
            self.dirty = true;
        }
        Ok(())
    }

    /// Returns `true` when there are commands available for undo.
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// Returns `true` when there are commands available for redo.
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Whether the history has been dirtied since the last [`mark_clean`].
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag (typically after a successful save).
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Remove all commands from both stacks and reset the dirty flag.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.dirty = false;
    }
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------------------------------------------------------
// Helper: find a mutable entity reference by PersistentId
// -------------------------------------------------------------------

pub(crate) fn find_entity_mut<'a>(
    scene: &'a mut Scene,
    id: &PersistentId,
) -> Result<&'a mut EntityRecord, EditorError> {
    scene
        .entities
        .iter_mut()
        .find(|e| e.persistent_id == *id)
        .ok_or_else(|| EditorError::EntityNotFound(id.clone()))
}

/// Collect all descendant IDs of `parent_id` (recursive, breadth-first).
pub(crate) fn collect_descendant_ids(scene: &Scene, parent_id: &PersistentId) -> Vec<PersistentId> {
    let mut ids = Vec::new();
    for entity in &scene.entities {
        if entity.parent.as_deref() == Some(parent_id.as_str()) {
            ids.push(entity.persistent_id.clone());
            ids.extend(collect_descendant_ids(scene, &entity.persistent_id));
        }
    }
    ids
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
// SetComponentField
// -------------------------------------------------------------------

pub struct SetComponentField {
    entity_id: PersistentId,
    component_type: ComponentTypeId,
    field_name: String,
    old_value: Option<Value>,
    new_value: Value,
}

impl SetComponentField {
    pub fn new(
        entity_id: PersistentId,
        component_type: ComponentTypeId,
        field_name: String,
        new_value: Value,
    ) -> Self {
        Self {
            entity_id,
            component_type,
            field_name,
            old_value: None,
            new_value,
        }
    }
}

impl Command for SetComponentField {
    fn name(&self) -> &str {
        "Set Component Field"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        let comp = entity
            .components
            .get_mut(&self.component_type)
            .ok_or_else(|| EditorError::ComponentNotFound(self.component_type.clone()))?;
        self.old_value = comp
            .fields
            .insert(self.field_name.clone(), self.new_value.clone());
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        let comp = entity
            .components
            .get_mut(&self.component_type)
            .ok_or_else(|| EditorError::ComponentNotFound(self.component_type.clone()))?;
        match self.old_value.take() {
            Some(val) => {
                comp.fields.insert(self.field_name.clone(), val);
            }
            None => {
                comp.fields.remove(&self.field_name);
            }
        }
        Ok(())
    }
}

// -------------------------------------------------------------------
// AddEntity
// -------------------------------------------------------------------

pub struct AddEntity {
    entity: EntityRecord,
}

impl AddEntity {
    pub fn new(entity: EntityRecord) -> Self {
        Self { entity }
    }
}

impl Command for AddEntity {
    fn name(&self) -> &str {
        "Add Entity"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        scene.entities.push(self.entity.clone());
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        scene
            .entities
            .retain(|e| e.persistent_id != self.entity.persistent_id);
        Ok(())
    }
}

// -------------------------------------------------------------------
// RemoveEntity (with recursive child removal)
// -------------------------------------------------------------------

pub struct RemoveEntity {
    removed: Vec<EntityRecord>,
}

impl RemoveEntity {
    /// Create a command that records which entities will be removed.
    /// Call [`capture`] *before* executing to snapshot the subtree.
    pub fn new(entity_id: &PersistentId, scene: &Scene) -> Self {
        let mut removed = Vec::new();

        // Capture the target entity.
        if let Some(entity) = scene
            .entities
            .iter()
            .find(|e| e.persistent_id == *entity_id)
        {
            removed.push(entity.clone());
        }

        // Capture all descendants recursively.
        let descendant_ids = collect_descendant_ids(scene, entity_id);
        for id in &descendant_ids {
            if let Some(entity) = scene.entities.iter().find(|e| e.persistent_id == *id) {
                removed.push(entity.clone());
            }
        }

        Self { removed }
    }
}

impl Command for RemoveEntity {
    fn name(&self) -> &str {
        "Remove Entity"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let ids: Vec<&PersistentId> = self.removed.iter().map(|r| &r.persistent_id).collect();
        scene.entities.retain(|e| !ids.contains(&&e.persistent_id));
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        for record in self.removed.iter().rev() {
            scene.entities.push(record.clone());
        }
        Ok(())
    }
}

// -------------------------------------------------------------------
// AddComponent
// -------------------------------------------------------------------

pub struct AddComponent {
    entity_id: PersistentId,
    component_type: ComponentTypeId,
    component: ComponentRecord,
}

impl AddComponent {
    pub fn new(
        entity_id: PersistentId,
        component_type: ComponentTypeId,
        component: ComponentRecord,
    ) -> Self {
        Self {
            entity_id,
            component_type,
            component,
        }
    }
}

impl Command for AddComponent {
    fn name(&self) -> &str {
        "Add Component"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        entity
            .components
            .insert(self.component_type.clone(), self.component.clone());
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        entity.components.remove(&self.component_type);
        Ok(())
    }
}

// -------------------------------------------------------------------
// RemoveComponent
// -------------------------------------------------------------------

pub struct RemoveComponent {
    entity_id: PersistentId,
    component_type: ComponentTypeId,
    was: Option<ComponentRecord>,
}

impl RemoveComponent {
    pub fn new(entity_id: PersistentId, component_type: ComponentTypeId) -> Self {
        Self {
            entity_id,
            component_type,
            was: None,
        }
    }
}

impl Command for RemoveComponent {
    fn name(&self) -> &str {
        "Remove Component"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        self.was = entity.components.remove(&self.component_type);
        if self.was.is_none() {
            return Err(EditorError::ComponentNotFound(self.component_type.clone()));
        }
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if let Some(comp) = self.was.clone() {
            let entity = find_entity_mut(scene, &self.entity_id)?;
            entity.components.insert(self.component_type.clone(), comp);
        }
        Ok(())
    }
}
