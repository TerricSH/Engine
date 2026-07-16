use engine_scene::{ComponentRecord, EntityRecord, Scene};
use engine_serialize::{ComponentTypeId, PersistentId, Value};

use crate::editor_ui::UiInteractionStamp;
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

/// An editor command tagged with the platform interaction that created it.
pub struct SequencedCommand {
    pub stamp: UiInteractionStamp,
    pub command: Box<dyn Command>,
}

impl SequencedCommand {
    pub fn new(stamp: UiInteractionStamp, command: Box<dyn Command>) -> Self {
        Self { stamp, command }
    }

    pub fn into_command(self) -> Box<dyn Command> {
        self.command
    }
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
    push_serial: u64,
}

impl CommandHistory {
    /// Create an empty history with a default undo limit of 256.
    pub fn new() -> Self {
        Self {
            undone: Vec::new(),
            done: Vec::new(),
            max_undo: 256,
            dirty: false,
            push_serial: 0,
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
        self.push_serial = self.push_serial.wrapping_add(1);

        // Trim the oldest commands when we exceed the limit.
        while self.done.len() > self.max_undo {
            self.done.remove(0);
        }
        Ok(())
    }

    /// Undo the most recent command.
    pub fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if let Some(mut cmd) = self.done.pop() {
            match cmd.undo(scene) {
                Ok(()) => {
                    self.undone.push(cmd);
                    self.dirty = true;
                }
                Err(error) => {
                    // A transient or externally-induced failure must not
                    // silently delete the only record of the operation.
                    self.done.push(cmd);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Redo the last-undone command.
    pub fn redo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if let Some(mut cmd) = self.undone.pop() {
            match cmd.execute(scene) {
                Ok(()) => {
                    self.done.push(cmd);
                    self.dirty = true;
                }
                Err(error) => {
                    self.undone.push(cmd);
                    return Err(error);
                }
            }
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

    /// Monotonic token changed after every successfully executed new command.
    ///
    /// Editor hosts use this to distinguish a text commit that produced a
    /// scene command from invalid input that was rejected before reaching the
    /// history. The token is intentionally unaffected by undo and redo.
    pub fn push_serial(&self) -> u64 {
        self.push_serial
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
    entity_id: PersistentId,
    removed: Vec<EntityRecord>,
    captured: bool,
}

impl RemoveEntity {
    /// Create a deferred removal. The subtree is captured on first execution,
    /// after all earlier ordered editor actions have reached the scene.
    pub fn new(entity_id: &PersistentId, _scene: &Scene) -> Self {
        Self {
            entity_id: entity_id.clone(),
            removed: Vec::new(),
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
            let target = scene
                .entities
                .iter()
                .find(|entity| entity.persistent_id == self.entity_id)
                .cloned()
                .ok_or_else(|| EditorError::EntityNotFound(self.entity_id.clone()))?;
            self.removed.push(target);
            for id in collect_descendant_ids(scene, &self.entity_id) {
                if let Some(entity) = scene
                    .entities
                    .iter()
                    .find(|entity| entity.persistent_id == id)
                {
                    self.removed.push(entity.clone());
                }
            }
            self.captured = true;
        }
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
    activated_camera: bool,
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
            activated_camera: false,
        }
    }
}

impl Command for AddComponent {
    fn name(&self) -> &str {
        "Add Component"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        if entity.components.contains_key(&self.component_type) {
            return Err(EditorError::ComponentAlreadyExists(
                self.component_type.clone(),
            ));
        }
        entity
            .components
            .insert(self.component_type.clone(), self.component.clone());
        self.activated_camera = false;
        if self.component_type == "engine.camera" && scene.scene_settings.active_camera.is_none() {
            scene.scene_settings.active_camera = Some(self.entity_id.clone());
            self.activated_camera = true;
        }
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        entity.components.remove(&self.component_type);
        if self.activated_camera
            && scene.scene_settings.active_camera.as_deref() == Some(self.entity_id.as_str())
        {
            scene.scene_settings.active_camera = None;
        }
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
    was_active_camera: bool,
}

impl RemoveComponent {
    pub fn new(entity_id: PersistentId, component_type: ComponentTypeId) -> Self {
        Self {
            entity_id,
            component_type,
            was: None,
            was_active_camera: false,
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
        self.was_active_camera = self.component_type == "engine.camera"
            && scene.scene_settings.active_camera.as_deref() == Some(self.entity_id.as_str());
        if self.was_active_camera {
            scene.scene_settings.active_camera = None;
        }
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if let Some(comp) = self.was.clone() {
            let entity = find_entity_mut(scene, &self.entity_id)?;
            entity.components.insert(self.component_type.clone(), comp);
        }
        if self.was_active_camera {
            scene.scene_settings.active_camera = Some(self.entity_id.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UndoFails;

    impl Command for UndoFails {
        fn name(&self) -> &str {
            "Undo Fails"
        }

        fn execute(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
            Ok(())
        }

        fn undo(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
            Err(EditorError::InitFailed("undo rejected".into()))
        }
    }

    struct RedoFails {
        executions: usize,
    }

    impl Command for RedoFails {
        fn name(&self) -> &str {
            "Redo Fails"
        }

        fn execute(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
            self.executions += 1;
            if self.executions > 1 {
                Err(EditorError::InitFailed("redo rejected".into()))
            } else {
                Ok(())
            }
        }

        fn undo(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
            Ok(())
        }
    }

    #[test]
    fn failed_undo_keeps_command_on_done_stack() {
        let mut history = CommandHistory::new();
        let mut scene = engine_scene::sample_scene();
        history.push(Box::new(UndoFails), &mut scene).unwrap();
        history.mark_clean();

        assert!(history.undo(&mut scene).is_err());
        assert!(history.can_undo());
        assert!(!history.can_redo());
        assert!(!history.is_dirty());
    }

    #[test]
    fn failed_redo_keeps_command_on_undone_stack() {
        let mut history = CommandHistory::new();
        let mut scene = engine_scene::sample_scene();
        history
            .push(Box::new(RedoFails { executions: 0 }), &mut scene)
            .unwrap();
        history.undo(&mut scene).unwrap();
        history.mark_clean();

        assert!(history.redo(&mut scene).is_err());
        assert!(!history.can_undo());
        assert!(history.can_redo());
        assert!(!history.is_dirty());
    }

    #[test]
    fn add_component_rejects_duplicates_without_overwriting_or_history() {
        let mut scene = engine_scene::sample_scene();
        let entity = scene
            .entities
            .iter()
            .find(|entity| !entity.components.is_empty())
            .unwrap();
        let entity_id = entity.persistent_id.clone();
        let (component_type, original) = entity.components.iter().next().unwrap();
        let component_type = component_type.clone();
        let original = original.clone();
        let mut replacement = original.clone();
        replacement.enabled = !replacement.enabled;
        let mut history = CommandHistory::new();

        let result = history.push(
            Box::new(AddComponent::new(
                entity_id.clone(),
                component_type.clone(),
                replacement,
            )),
            &mut scene,
        );
        assert!(matches!(
            result,
            Err(EditorError::ComponentAlreadyExists(type_id)) if type_id == component_type
        ));
        assert_eq!(
            scene
                .entities
                .iter()
                .find(|entity| entity.persistent_id == entity_id)
                .unwrap()
                .components[&component_type],
            original
        );
        assert!(!history.can_undo());
    }

    #[test]
    fn camera_component_commands_keep_active_camera_valid_through_undo_redo() {
        let mut scene = engine_scene::sample_scene();
        let entity_id = scene.entities[0].persistent_id.clone();
        scene.entities[0].components.remove("engine.camera");
        scene.scene_settings.active_camera = None;
        let camera = ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
        };
        let mut history = CommandHistory::new();

        history
            .push(
                Box::new(AddComponent::new(
                    entity_id.clone(),
                    "engine.camera".into(),
                    camera,
                )),
                &mut scene,
            )
            .unwrap();
        assert_eq!(
            scene.scene_settings.active_camera.as_deref(),
            Some(entity_id.as_str())
        );

        history.undo(&mut scene).unwrap();
        assert!(scene.scene_settings.active_camera.is_none());
        assert!(!scene.entities[0].components.contains_key("engine.camera"));

        history.redo(&mut scene).unwrap();
        assert_eq!(
            scene.scene_settings.active_camera.as_deref(),
            Some(entity_id.as_str())
        );
        history
            .push(
                Box::new(RemoveComponent::new(
                    entity_id.clone(),
                    "engine.camera".into(),
                )),
                &mut scene,
            )
            .unwrap();
        assert!(scene.scene_settings.active_camera.is_none());

        history.undo(&mut scene).unwrap();
        assert_eq!(
            scene.scene_settings.active_camera.as_deref(),
            Some(entity_id.as_str())
        );
        assert!(scene.entities[0].components.contains_key("engine.camera"));
    }
}
