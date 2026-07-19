use engine_scene::{
    validate_scene_for_authoring, ComponentRecord, ComponentRegistry, EntityRecord, Scene,
    SceneSettings,
};
use engine_serialize::{ComponentTypeId, PersistentId, Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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

/// Atomically replace scene-level authoring settings.
///
/// The command captures the exact prior settings and rejects stale execute or
/// undo attempts. `CommandHistory` performs the canonical Scene -> World and
/// render-graph preflight before accepting the result.
pub struct SetSceneSettings {
    expected: SceneSettings,
    replacement: SceneSettings,
    applied: bool,
}

impl SetSceneSettings {
    pub fn prepare(scene: &Scene, replacement: SceneSettings) -> Self {
        Self {
            expected: scene.scene_settings.clone(),
            replacement,
            applied: false,
        }
    }

    pub fn replacement(&self) -> &SceneSettings {
        &self.replacement
    }
}

impl Command for SetSceneSettings {
    fn name(&self) -> &str {
        "Set Scene Settings"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if self.applied {
            return Err(EditorError::SceneCommandRejected {
                operation: self.name().to_string(),
                reason: "settings command is already applied".to_string(),
            });
        }
        if scene.scene_settings != self.expected {
            return Err(EditorError::SceneCommandRejected {
                operation: self.name().to_string(),
                reason: "scene settings changed after the command was prepared".to_string(),
            });
        }
        scene.scene_settings = self.replacement.clone();
        self.applied = true;
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if !self.applied {
            return Err(EditorError::SceneCommandRejected {
                operation: format!("undo {}", self.name()),
                reason: "settings command is not applied".to_string(),
            });
        }
        if scene.scene_settings != self.replacement {
            return Err(EditorError::SceneCommandRejected {
                operation: format!("undo {}", self.name()),
                reason: "scene settings changed before undo".to_string(),
            });
        }
        scene.scene_settings = self.expected.clone();
        self.applied = false;
        Ok(())
    }
}

/// One atomic undo entry composed of multiple scene commands.
///
/// Forward execution rolls back already-applied children if a later child
/// fails. Undo runs in reverse order and restores already-undone children if
/// a later undo fails, so the scene never observes a half-applied transaction.
pub struct CommandBatch {
    name: String,
    commands: Vec<Box<dyn Command>>,
}

impl CommandBatch {
    pub fn new(name: impl Into<String>, commands: Vec<Box<dyn Command>>) -> Self {
        Self {
            name: name.into(),
            commands,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

impl Command for CommandBatch {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let before = scene.clone();
        let mut applied = 0;
        for index in 0..self.commands.len() {
            if let Err(error) = self.commands[index].execute(scene) {
                for rollback in (0..applied).rev() {
                    let _ = self.commands[rollback].undo(scene);
                }
                *scene = before;
                return Err(error);
            }
            applied += 1;
        }
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let before = scene.clone();
        let mut undone: Vec<usize> = Vec::new();
        for index in (0..self.commands.len()).rev() {
            if let Err(error) = self.commands[index].undo(scene) {
                for restore in undone.into_iter().rev() {
                    let _ = self.commands[restore].execute(scene);
                }
                *scene = before;
                return Err(error);
            }
            undone.push(index);
        }
        Ok(())
    }
}

// -------------------------------------------------------------------
// CommandHistory – undo / redo stack
// -------------------------------------------------------------------

pub(crate) struct HistoryEntry {
    pub(crate) command: Box<dyn Command>,
    before_state: u64,
    after_state: u64,
}

/// Tracks a linear undo/redo history and the exact saved scene state.
pub struct CommandHistory {
    pub(crate) undone: Vec<HistoryEntry>,
    pub(crate) done: Vec<HistoryEntry>,
    max_undo: usize,
    current_state: u64,
    clean_state: Option<u64>,
    next_state: u64,
    push_serial: u64,
    component_registry: Option<Arc<ComponentRegistry>>,
}

impl CommandHistory {
    /// Create an empty history with a default undo limit of 256.
    pub fn new() -> Self {
        Self {
            undone: Vec::new(),
            done: Vec::new(),
            max_undo: 256,
            current_state: 0,
            clean_state: Some(0),
            next_state: 1,
            push_serial: 0,
            component_registry: None,
        }
    }

    pub(crate) fn install_component_registry(
        &mut self,
        scene: &Scene,
        component_registry: Arc<ComponentRegistry>,
    ) -> Result<(), EditorError> {
        validate_scene_for_authoring(scene, Some(component_registry.as_ref())).map_err(
            |error| EditorError::SceneCommandRejected {
                operation: "install component registry".to_string(),
                reason: error.to_string(),
            },
        )?;
        self.component_registry = Some(component_registry);
        Ok(())
    }

    fn validate_result(&self, scene: &Scene, operation: String) -> Result<(), EditorError> {
        validate_scene_for_authoring(scene, self.component_registry.as_deref()).map_err(|error| {
            EditorError::SceneCommandRejected {
                operation,
                reason: error.to_string(),
            }
        })
    }

    /// Execute `cmd` on `scene`, push it onto the done stack, and clear
    /// the redo stack.
    pub fn push(
        &mut self,
        mut cmd: Box<dyn Command>,
        scene: &mut Scene,
    ) -> Result<(), EditorError> {
        let before = scene.clone();
        if let Err(error) = cmd.execute(scene) {
            *scene = before;
            return Err(error);
        }
        if let Err(error) = self.validate_result(scene, format!("execute `{}`", cmd.name())) {
            *scene = before;
            return Err(error);
        }
        let before_state = self.current_state;
        let after_state = self.next_state;
        self.next_state = self.next_state.wrapping_add(1);
        self.current_state = after_state;
        self.done.push(HistoryEntry {
            command: cmd,
            before_state,
            after_state,
        });
        self.undone.clear();
        self.push_serial = self.push_serial.wrapping_add(1);

        // Trim the oldest commands when we exceed the limit.
        while self.done.len() > self.max_undo {
            self.done.remove(0);
        }
        Ok(())
    }

    /// Undo the most recent command.
    pub fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if let Some(mut entry) = self.done.pop() {
            let before = scene.clone();
            let result = entry.command.undo(scene).and_then(|()| {
                self.validate_result(scene, format!("undo `{}`", entry.command.name()))
            });
            match result {
                Ok(()) => {
                    self.current_state = entry.before_state;
                    self.undone.push(entry);
                }
                Err(error) => {
                    *scene = before;
                    // A transient or externally-induced failure must not
                    // silently delete the only record of the operation.
                    self.done.push(entry);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Redo the last-undone command.
    pub fn redo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if let Some(mut entry) = self.undone.pop() {
            let before = scene.clone();
            let result = entry.command.execute(scene).and_then(|()| {
                self.validate_result(scene, format!("redo `{}`", entry.command.name()))
            });
            match result {
                Ok(()) => {
                    self.current_state = entry.after_state;
                    self.done.push(entry);
                }
                Err(error) => {
                    *scene = before;
                    self.undone.push(entry);
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
        self.clean_state != Some(self.current_state)
    }

    /// Clear the dirty flag (typically after a successful save).
    pub fn mark_clean(&mut self) {
        self.clean_state = Some(self.current_state);
    }

    /// Mark externally restored authoring data as requiring an explicit save.
    pub fn mark_dirty(&mut self) {
        self.clean_state = None;
    }

    /// Remove all commands from both stacks and reset the dirty flag.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.current_state = self.next_state;
        self.next_state = self.next_state.wrapping_add(1);
        self.clean_state = Some(self.current_state);
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
    let mut pending = vec![parent_id.clone()];
    let mut visited = BTreeSet::from([parent_id.clone()]);
    while let Some(parent) = pending.pop() {
        for entity in scene.entities.iter().rev() {
            if entity.parent.as_deref() == Some(parent.as_str())
                && visited.insert(entity.persistent_id.clone())
            {
                ids.push(entity.persistent_id.clone());
                pending.push(entity.persistent_id.clone());
            }
        }
    }
    ids
}

// -------------------------------------------------------------------
// Component copy / paste model
// -------------------------------------------------------------------

const COMPONENT_CLIPBOARD_FORMAT_VERSION: u32 = 1;

/// A typed, serializable snapshot used by Copy/Paste Component Values.
///
/// [`ComponentRecord`] deliberately does not contain its registry key, so the
/// clipboard carries that key separately. This prevents a record copied from
/// one component type from being pasted over another component type merely
/// because both records happen to have compatible fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentClipboard {
    format_version: u32,
    type_id: ComponentTypeId,
    component: ComponentRecord,
}

impl ComponentClipboard {
    /// Capture one component without mutating the scene.
    pub fn capture(
        scene: &Scene,
        entity_id: &PersistentId,
        component_type: &ComponentTypeId,
    ) -> Result<Self, EditorError> {
        let entity = scene
            .entities
            .iter()
            .find(|entity| &entity.persistent_id == entity_id)
            .ok_or_else(|| EditorError::EntityNotFound(entity_id.clone()))?;
        let component = entity
            .components
            .get(component_type)
            .cloned()
            .ok_or_else(|| EditorError::ComponentNotFound(component_type.clone()))?;
        Self::from_record(component_type.clone(), component)
    }

    /// Construct a typed clipboard from an already-owned record.
    pub fn from_record(
        type_id: ComponentTypeId,
        component: ComponentRecord,
    ) -> Result<Self, EditorError> {
        let clipboard = Self {
            format_version: COMPONENT_CLIPBOARD_FORMAT_VERSION,
            type_id,
            component,
        };
        clipboard.validate()?;
        Ok(clipboard)
    }

    pub fn type_id(&self) -> &ComponentTypeId {
        &self.type_id
    }

    pub fn component(&self) -> &ComponentRecord {
        &self.component
    }

    /// Serialize the validated clipboard as portable RON text.
    pub fn to_ron(&self) -> Result<String, EditorError> {
        self.validate()?;
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|error| EditorError::ComponentClipboardSerialization(error.to_string()))
    }

    /// Parse and validate RON before any editor command can consume it.
    pub fn from_ron(serialized: &str) -> Result<Self, EditorError> {
        let clipboard: Self = ron::from_str(serialized)
            .map_err(|error| EditorError::ComponentClipboardSerialization(error.to_string()))?;
        clipboard.validate()?;
        Ok(clipboard)
    }

    pub fn validate(&self) -> Result<(), EditorError> {
        if self.format_version != COMPONENT_CLIPBOARD_FORMAT_VERSION {
            return Err(EditorError::InvalidComponentClipboard(format!(
                "unsupported format version {}",
                self.format_version
            )));
        }
        if self.type_id.trim().is_empty() || self.type_id.trim() != self.type_id {
            return Err(EditorError::InvalidComponentClipboard(
                "component type ID must be non-empty and contain no surrounding whitespace".into(),
            ));
        }
        if self
            .component
            .fields
            .keys()
            .any(|field| field.trim().is_empty())
        {
            return Err(EditorError::InvalidComponentClipboard(
                "component field names must be non-empty".into(),
            ));
        }
        Ok(())
    }
}

/// Atomically replace the serialized values of one existing component.
///
/// This command never removes or inserts a component key, so required
/// components remain present. [`prepare`](Self::prepare) is the entry point
/// for clipboard paste: it verifies that the clipboard and target have the
/// exact same type and captures a destination snapshot. Both execution and
/// undo reject stale scene state before making any mutation.
pub struct ReplaceComponent {
    entity_id: PersistentId,
    component_type: ComponentTypeId,
    expected: Option<ComponentRecord>,
    replacement: ComponentRecord,
    applied: bool,
}

impl ReplaceComponent {
    /// Create a deferred same-key replacement from a trusted record.
    ///
    /// This is useful for Reset Component values generated by the component
    /// catalog. Clipboard paste should use [`prepare`](Self::prepare), which
    /// additionally verifies the clipboard's type metadata.
    pub fn new(
        entity_id: PersistentId,
        component_type: ComponentTypeId,
        replacement: ComponentRecord,
    ) -> Self {
        Self {
            entity_id,
            component_type,
            expected: None,
            replacement,
            applied: false,
        }
    }

    /// Prepare a paste against an exact read-only snapshot of the target.
    pub fn prepare(
        scene: &Scene,
        entity_id: PersistentId,
        target_component_type: ComponentTypeId,
        clipboard: &ComponentClipboard,
    ) -> Result<Self, EditorError> {
        clipboard.validate()?;
        if clipboard.type_id != target_component_type {
            return Err(EditorError::InvalidComponentClipboard(format!(
                "cannot paste component type '{}' over '{}'",
                clipboard.type_id, target_component_type
            )));
        }
        let entity = scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == entity_id)
            .ok_or_else(|| EditorError::EntityNotFound(entity_id.clone()))?;
        let expected = entity
            .components
            .get(&target_component_type)
            .cloned()
            .ok_or_else(|| EditorError::ComponentNotFound(target_component_type.clone()))?;
        Ok(Self {
            entity_id,
            component_type: target_component_type,
            expected: Some(expected),
            replacement: clipboard.component.clone(),
            applied: false,
        })
    }

    pub fn replacement(&self) -> &ComponentRecord {
        &self.replacement
    }
}

impl Command for ReplaceComponent {
    fn name(&self) -> &str {
        "Replace Component Values"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if self.applied {
            return Err(EditorError::InvalidComponentClipboard(
                "component replacement is already applied".into(),
            ));
        }
        let component = find_entity_mut(scene, &self.entity_id)?
            .components
            .get_mut(&self.component_type)
            .ok_or_else(|| EditorError::ComponentNotFound(self.component_type.clone()))?;
        if let Some(expected) = &self.expected {
            if component != expected {
                return Err(EditorError::InvalidComponentClipboard(format!(
                    "component '{}' changed after replacement was prepared",
                    self.component_type
                )));
            }
        } else {
            self.expected = Some(component.clone());
        }
        *component = self.replacement.clone();
        self.applied = true;
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        if !self.applied {
            return Err(EditorError::InvalidComponentClipboard(
                "component replacement was not executed".into(),
            ));
        }
        let expected = self.expected.as_ref().ok_or_else(|| {
            EditorError::InvalidComponentClipboard(
                "component replacement has no original snapshot".into(),
            )
        })?;
        let component = find_entity_mut(scene, &self.entity_id)?
            .components
            .get_mut(&self.component_type)
            .ok_or_else(|| EditorError::ComponentNotFound(self.component_type.clone()))?;
        if component != &self.replacement {
            return Err(EditorError::InvalidComponentClipboard(format!(
                "component '{}' changed before replacement undo",
                self.component_type
            )));
        }
        *component = expected.clone();
        self.applied = false;
        Ok(())
    }
}

// -------------------------------------------------------------------
// Entity copy / paste model
// -------------------------------------------------------------------

const ENTITY_CLIPBOARD_FORMAT_VERSION: u32 = 1;

/// A self-contained, serializable set of copied entity subtrees.
///
/// The records retain each copied root's original external parent so callers
/// can either preserve its hierarchy placement or explicitly paste under a
/// different parent. Parents and [`Value::Entity`] references within the
/// copied set are remapped to fresh persistent IDs when a paste is prepared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityClipboard {
    format_version: u32,
    root_ids: Vec<PersistentId>,
    entities: Vec<EntityRecord>,
}

impl EntityClipboard {
    /// Capture complete subtrees rooted at `entity_ids` without mutating the
    /// scene. Selecting both an ancestor and one of its descendants records
    /// that subtree only once.
    pub fn capture(scene: &Scene, entity_ids: &[PersistentId]) -> Result<Self, EditorError> {
        if entity_ids.is_empty() {
            return Err(EditorError::InvalidEntityClipboard(
                "at least one root entity is required".into(),
            ));
        }
        let by_id = scene
            .entities
            .iter()
            .map(|entity| (entity.persistent_id.as_str(), entity))
            .collect::<BTreeMap<_, _>>();
        let requested = entity_ids.iter().cloned().collect::<BTreeSet<_>>();
        if requested.len() != entity_ids.len() {
            return Err(EditorError::InvalidEntityClipboard(
                "root entity list contains duplicates".into(),
            ));
        }
        for entity_id in entity_ids {
            if !by_id.contains_key(entity_id.as_str()) {
                return Err(EditorError::EntityNotFound(entity_id.clone()));
            }
        }

        let mut root_ids = Vec::new();
        for entity_id in entity_ids {
            let mut cursor = by_id[entity_id.as_str()].parent.as_deref();
            let mut visited = BTreeSet::from([entity_id.as_str()]);
            let mut has_requested_ancestor = false;
            while let Some(parent_id) = cursor {
                if !visited.insert(parent_id) {
                    return Err(EditorError::InvalidHierarchy(format!(
                        "cycle encountered while copying '{entity_id}'"
                    )));
                }
                if requested.contains(parent_id) {
                    has_requested_ancestor = true;
                    break;
                }
                cursor = by_id
                    .get(parent_id)
                    .and_then(|parent| parent.parent.as_deref());
            }
            if !has_requested_ancestor {
                root_ids.push(entity_id.clone());
            }
        }

        let mut captured_ids = BTreeSet::new();
        let mut entities = Vec::new();
        for root_id in &root_ids {
            capture_subtree_records(
                scene,
                root_id,
                &mut BTreeSet::new(),
                &mut captured_ids,
                &mut entities,
            )?;
        }
        Self::from_records(root_ids, entities)
    }

    /// Construct a clipboard from serialized records and validate that every
    /// record belongs to exactly one declared root subtree.
    pub fn from_records(
        root_ids: Vec<PersistentId>,
        entities: Vec<EntityRecord>,
    ) -> Result<Self, EditorError> {
        let clipboard = Self {
            format_version: ENTITY_CLIPBOARD_FORMAT_VERSION,
            root_ids,
            entities,
        };
        clipboard.validate()?;
        Ok(clipboard)
    }

    pub fn root_ids(&self) -> &[PersistentId] {
        &self.root_ids
    }

    pub fn entities(&self) -> &[EntityRecord] {
        &self.entities
    }

    /// Serialize this clipboard to the RON text used by the platform
    /// clipboard and editor tests.
    pub fn to_ron(&self) -> Result<String, EditorError> {
        self.validate()?;
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|error| EditorError::EntityClipboardSerialization(error.to_string()))
    }

    /// Parse and fully validate RON clipboard text before it can be pasted.
    pub fn from_ron(serialized: &str) -> Result<Self, EditorError> {
        let clipboard: Self = ron::from_str(serialized)
            .map_err(|error| EditorError::EntityClipboardSerialization(error.to_string()))?;
        clipboard.validate()?;
        Ok(clipboard)
    }

    pub fn validate(&self) -> Result<(), EditorError> {
        if self.format_version != ENTITY_CLIPBOARD_FORMAT_VERSION {
            return Err(EditorError::InvalidEntityClipboard(format!(
                "unsupported format version {}",
                self.format_version
            )));
        }
        if self.root_ids.is_empty() || self.entities.is_empty() {
            return Err(EditorError::InvalidEntityClipboard(
                "clipboard must contain at least one rooted entity".into(),
            ));
        }
        let roots = self.root_ids.iter().cloned().collect::<BTreeSet<_>>();
        if roots.len() != self.root_ids.len() || roots.iter().any(String::is_empty) {
            return Err(EditorError::InvalidEntityClipboard(
                "root IDs must be non-empty and unique".into(),
            ));
        }
        let mut records = BTreeMap::new();
        for entity in &self.entities {
            if entity.persistent_id.is_empty() {
                return Err(EditorError::InvalidEntityClipboard(
                    "entity persistent IDs cannot be empty".into(),
                ));
            }
            if records
                .insert(entity.persistent_id.as_str(), entity)
                .is_some()
            {
                return Err(EditorError::InvalidEntityClipboard(format!(
                    "duplicate entity ID '{}'",
                    entity.persistent_id
                )));
            }
        }
        for root_id in &self.root_ids {
            let root = records.get(root_id.as_str()).ok_or_else(|| {
                EditorError::InvalidEntityClipboard(format!(
                    "root '{root_id}' has no entity record"
                ))
            })?;
            if root
                .parent
                .as_ref()
                .is_some_and(|parent| records.contains_key(parent.as_str()))
            {
                return Err(EditorError::InvalidEntityClipboard(format!(
                    "root '{root_id}' has a parent inside the copied set"
                )));
            }
        }
        for entity in &self.entities {
            let mut cursor = entity.persistent_id.as_str();
            let mut visited = BTreeSet::new();
            loop {
                if !visited.insert(cursor) {
                    return Err(EditorError::InvalidEntityClipboard(format!(
                        "entity hierarchy contains a cycle at '{cursor}'"
                    )));
                }
                if roots.contains(cursor) {
                    break;
                }
                let record = records.get(cursor).ok_or_else(|| {
                    EditorError::InvalidEntityClipboard(format!(
                        "entity '{cursor}' is not part of a copied root subtree"
                    ))
                })?;
                cursor = record.parent.as_deref().ok_or_else(|| {
                    EditorError::InvalidEntityClipboard(format!(
                        "entity '{}' does not descend from a declared root",
                        entity.persistent_id
                    ))
                })?;
                if !records.contains_key(cursor) {
                    return Err(EditorError::InvalidEntityClipboard(format!(
                        "non-root entity '{}' references external parent '{cursor}'",
                        entity.persistent_id
                    )));
                }
            }
        }
        Ok(())
    }
}

fn capture_subtree_records(
    scene: &Scene,
    entity_id: &PersistentId,
    visiting: &mut BTreeSet<PersistentId>,
    captured: &mut BTreeSet<PersistentId>,
    records: &mut Vec<EntityRecord>,
) -> Result<(), EditorError> {
    if !visiting.insert(entity_id.clone()) {
        return Err(EditorError::InvalidHierarchy(format!(
            "cycle encountered while copying '{entity_id}'"
        )));
    }
    if captured.insert(entity_id.clone()) {
        let entity = scene
            .entities
            .iter()
            .find(|entity| &entity.persistent_id == entity_id)
            .cloned()
            .ok_or_else(|| EditorError::EntityNotFound(entity_id.clone()))?;
        records.push(entity);
        for child in scene
            .entities
            .iter()
            .filter(|child| child.parent.as_ref() == Some(entity_id))
        {
            capture_subtree_records(scene, &child.persistent_id, visiting, captured, records)?;
        }
    }
    visiting.remove(entity_id);
    Ok(())
}

/// Controls how copied root records are attached when a paste is prepared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityPasteParent {
    /// Keep each root's original external parent. Missing parents are errors.
    PreserveOriginal,
    /// Detach every copied root into the scene root.
    SceneRoot,
    /// Attach every copied root under this entity.
    Entity(PersistentId),
}

/// An undoable insertion prepared from validated [`EntityClipboard`] data.
///
/// Preparation is read-only and allocates all fresh IDs up front. Execution
/// revalidates the destination, making a stale paste plan fail without any
/// partial scene mutation.
pub struct PasteEntityRecords {
    records: Vec<EntityRecord>,
    root_ids: Vec<PersistentId>,
    insertion_index: usize,
}

impl PasteEntityRecords {
    /// Prepare a paste at the end of the scene's serialized entity sequence.
    pub fn prepare(
        scene: &Scene,
        clipboard: &EntityClipboard,
        parent: EntityPasteParent,
    ) -> Result<Self, EditorError> {
        Self::prepare_at(scene, clipboard, parent, scene.entities.len())
    }

    fn prepare_at(
        scene: &Scene,
        clipboard: &EntityClipboard,
        parent: EntityPasteParent,
        insertion_index: usize,
    ) -> Result<Self, EditorError> {
        clipboard.validate()?;
        if insertion_index > scene.entities.len() {
            return Err(EditorError::InvalidEntityClipboard(
                "paste insertion point is outside the scene".into(),
            ));
        }
        let existing = scene
            .entities
            .iter()
            .map(|entity| entity.persistent_id.clone())
            .collect::<BTreeSet<_>>();
        if let EntityPasteParent::Entity(parent_id) = &parent {
            if !existing.contains(parent_id) {
                return Err(EditorError::EntityNotFound(parent_id.clone()));
            }
        }

        let mut used = existing;
        let mut id_map = BTreeMap::new();
        for entity in &clipboard.entities {
            let new_id = allocate_copy_id(&entity.persistent_id, &mut used);
            id_map.insert(entity.persistent_id.clone(), new_id);
        }
        let roots = clipboard.root_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut records = Vec::with_capacity(clipboard.entities.len());
        for entity in &clipboard.entities {
            let mut pasted = entity.clone();
            pasted.persistent_id = id_map[&entity.persistent_id].clone();
            pasted.parent = if roots.contains(&entity.persistent_id) {
                match &parent {
                    EntityPasteParent::PreserveOriginal => match &entity.parent {
                        Some(parent_id) if !used.contains(parent_id) => {
                            return Err(EditorError::EntityNotFound(parent_id.clone()));
                        }
                        original => original.clone(),
                    },
                    EntityPasteParent::SceneRoot => None,
                    EntityPasteParent::Entity(parent_id) => Some(parent_id.clone()),
                }
            } else {
                entity
                    .parent
                    .as_ref()
                    .and_then(|parent_id| id_map.get(parent_id))
                    .cloned()
                    .ok_or_else(|| {
                        EditorError::InvalidEntityClipboard(format!(
                            "cannot remap parent of '{}'",
                            entity.persistent_id
                        ))
                    })?
                    .into()
            };
            for component in pasted.components.values_mut() {
                for value in component.fields.values_mut() {
                    remap_entity_references(value, &id_map);
                }
            }
            records.push(pasted);
        }
        let root_ids = clipboard
            .root_ids
            .iter()
            .map(|root_id| id_map[root_id].clone())
            .collect();
        let command = Self {
            records,
            root_ids,
            insertion_index,
        };
        command.validate_destination(scene)?;
        Ok(command)
    }

    pub fn pasted_root_ids(&self) -> &[PersistentId] {
        &self.root_ids
    }

    pub fn pasted_records(&self) -> &[EntityRecord] {
        &self.records
    }

    fn validate_destination(&self, scene: &Scene) -> Result<(), EditorError> {
        if self.insertion_index > scene.entities.len() {
            return Err(EditorError::InvalidEntityClipboard(
                "paste insertion point is no longer valid".into(),
            ));
        }
        let existing = scene
            .entities
            .iter()
            .map(|entity| entity.persistent_id.as_str())
            .collect::<BTreeSet<_>>();
        let pasted = self
            .records
            .iter()
            .map(|entity| entity.persistent_id.as_str())
            .collect::<BTreeSet<_>>();
        if pasted.len() != self.records.len() {
            return Err(EditorError::InvalidEntityClipboard(
                "prepared paste contains duplicate IDs".into(),
            ));
        }
        for entity in &self.records {
            if existing.contains(entity.persistent_id.as_str()) {
                return Err(EditorError::EntityAlreadyExists(
                    entity.persistent_id.clone(),
                ));
            }
            if let Some(parent_id) = &entity.parent {
                if !existing.contains(parent_id.as_str()) && !pasted.contains(parent_id.as_str()) {
                    return Err(EditorError::EntityNotFound(parent_id.clone()));
                }
            }
        }
        Ok(())
    }
}

impl Command for PasteEntityRecords {
    fn name(&self) -> &str {
        "Paste Entities"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        self.validate_destination(scene)?;
        scene.entities.splice(
            self.insertion_index..self.insertion_index,
            self.records.iter().cloned(),
        );
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let end = self.insertion_index.saturating_add(self.records.len());
        if end > scene.entities.len() || scene.entities[self.insertion_index..end] != self.records {
            return Err(EditorError::InvalidHierarchy(
                "pasted entity sequence changed before undo".into(),
            ));
        }
        scene.entities.drain(self.insertion_index..end);
        Ok(())
    }
}

/// Undoable recursive duplication of exactly one entity subtree.
pub struct DuplicateEntitySubtree {
    paste: PasteEntityRecords,
}

impl DuplicateEntitySubtree {
    /// Build a deterministic duplicate plan without mutating `scene`.
    pub fn prepare(scene: &Scene, root_id: &PersistentId) -> Result<Self, EditorError> {
        let clipboard = EntityClipboard::capture(scene, std::slice::from_ref(root_id))?;
        let copied_ids = clipboard
            .entities
            .iter()
            .map(|entity| entity.persistent_id.as_str())
            .collect::<BTreeSet<_>>();
        let insertion_index = scene
            .entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| copied_ids.contains(entity.persistent_id.as_str()))
            .map(|(index, _)| index + 1)
            .max()
            .ok_or_else(|| EditorError::EntityNotFound(root_id.clone()))?;
        Ok(Self {
            paste: PasteEntityRecords::prepare_at(
                scene,
                &clipboard,
                EntityPasteParent::PreserveOriginal,
                insertion_index,
            )?,
        })
    }

    pub fn duplicated_root_id(&self) -> &PersistentId {
        &self.paste.root_ids[0]
    }

    pub fn duplicated_records(&self) -> &[EntityRecord] {
        &self.paste.records
    }
}

impl Command for DuplicateEntitySubtree {
    fn name(&self) -> &str {
        "Duplicate Entity Subtree"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        self.paste.execute(scene)
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        self.paste.undo(scene)
    }
}

fn allocate_copy_id(base: &str, used: &mut BTreeSet<PersistentId>) -> PersistentId {
    let first = format!("{base}-copy");
    if used.insert(first.clone()) {
        return first;
    }
    for suffix in 2_u64.. {
        let candidate = format!("{base}-copy-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("the u64 copy-ID suffix space cannot be exhausted in memory")
}

fn remap_entity_references(value: &mut Value, id_map: &BTreeMap<PersistentId, PersistentId>) {
    match value {
        Value::Entity(entity_id) => {
            if let Some(remapped) = id_map.get(entity_id) {
                *entity_id = remapped.clone();
            }
        }
        Value::List(values) => {
            for value in values {
                remap_entity_references(value, id_map);
            }
        }
        Value::Map(values) => {
            for value in values.values_mut() {
                remap_entity_references(value, id_map);
            }
        }
        _ => {}
    }
}

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

fn sibling_ids(scene: &Scene, parent: Option<&PersistentId>) -> Vec<PersistentId> {
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
        let old_value = comp.fields.get(&self.field_name).cloned().ok_or_else(|| {
            EditorError::ComponentFieldNotFound {
                component_type: self.component_type.clone(),
                field_name: self.field_name.clone(),
            }
        })?;
        comp.fields
            .insert(self.field_name.clone(), self.new_value.clone());
        self.old_value = Some(old_value);
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let entity = find_entity_mut(scene, &self.entity_id)?;
        let comp = entity
            .components
            .get_mut(&self.component_type)
            .ok_or_else(|| EditorError::ComponentNotFound(self.component_type.clone()))?;
        let old_value = self
            .old_value
            .take()
            .ok_or_else(|| EditorError::SceneCommandRejected {
                operation: "undo `Set Component Field`".to_string(),
                reason: "the command has no captured previous value".to_string(),
            })?;
        comp.fields.insert(self.field_name.clone(), old_value);
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
    use engine_scene::Component;

    fn entity(id: &str, parent: Option<&str>) -> EntityRecord {
        EntityRecord {
            persistent_id: id.into(),
            parent: parent.map(str::to_owned),
            name: Some(id.into()),
            enabled: true,
            components: BTreeMap::new(),
        }
    }

    fn hierarchy_scene() -> Scene {
        let mut scene = engine_scene::sample_scene();
        scene.scene_settings.active_camera = None;
        scene.entities = vec![
            entity("external", None),
            entity("root", Some("external")),
            entity("other-root", None),
            entity("child", Some("root")),
            entity("child-copy", None),
            entity("grandchild", Some("child")),
            entity("tail", None),
        ];
        let component = ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("internal".into(), Value::Entity("child".into())),
                (
                    "nested".into(),
                    Value::Map(BTreeMap::from([(
                        "target".into(),
                        Value::List(vec![Value::Entity("grandchild".into())]),
                    )])),
                ),
                ("external".into(), Value::Entity("external".into())),
            ]),
        };
        scene.entities[1]
            .components
            .insert("test.references".into(), component);
        scene
    }

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

    struct MutatesThenFails;

    impl Command for MutatesThenFails {
        fn name(&self) -> &str {
            "Mutates Then Fails"
        }

        fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
            scene.entities.push(entity("partial", None));
            Err(EditorError::InitFailed("forward rejected".into()))
        }

        fn undo(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
            Ok(())
        }
    }

    struct UndoMutatesThenFails;

    impl Command for UndoMutatesThenFails {
        fn name(&self) -> &str {
            "Undo Mutates Then Fails"
        }

        fn execute(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
            Ok(())
        }

        fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
            scene.entities.clear();
            Err(EditorError::InitFailed("undo rejected".into()))
        }
    }

    struct UndoProducesInvalidScene;

    impl Command for UndoProducesInvalidScene {
        fn name(&self) -> &str {
            "Undo Produces Invalid Scene"
        }

        fn execute(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
            Ok(())
        }

        fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
            scene.entities[1]
                .components
                .get_mut("engine.renderable")
                .unwrap()
                .fields
                .insert("visible".into(), Value::Str("not-a-bool".into()));
            Ok(())
        }
    }

    struct RedoProducesInvalidScene {
        executions: usize,
    }

    impl Command for RedoProducesInvalidScene {
        fn name(&self) -> &str {
            "Redo Produces Invalid Scene"
        }

        fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
            self.executions += 1;
            if self.executions > 1 {
                scene.entities[1]
                    .components
                    .get_mut("engine.renderable")
                    .unwrap()
                    .fields
                    .insert("visible".into(), Value::Str("not-a-bool".into()));
            }
            Ok(())
        }

        fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
            scene.entities[1]
                .components
                .get_mut("engine.renderable")
                .unwrap()
                .fields
                .insert("visible".into(), Value::Bool(true));
            Ok(())
        }
    }

    struct TestExternal {
        _value: u64,
    }

    impl engine_scene::Component for TestExternal {
        const TYPE_ID: &'static str = "test.validated_external";
    }

    struct WrongExternal;

    impl engine_scene::Component for WrongExternal {
        const TYPE_ID: &'static str = "test.wrong_external";
    }

    fn test_external_storage() -> Box<dyn engine_scene::ComponentStorageDyn> {
        Box::new(engine_scene::SparseSet::<TestExternal>::new())
    }

    fn deserialize_test_external(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
        match fields.get("value") {
            Some(Value::UInt(value)) => Box::new(TestExternal { _value: *value }),
            _ => Box::new(WrongExternal),
        }
    }

    fn strict_test_registry() -> Arc<ComponentRegistry> {
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        registry
            .register(engine_scene::ComponentExtension {
                meta: engine_scene::ComponentMeta {
                    type_id: TestExternal::TYPE_ID,
                    display_name: "Validated External",
                    schema_version: (0, 1, 0),
                    has_editor: true,
                    script_access: engine_scene::ScriptAccess::None,
                },
                storage_factory: test_external_storage,
                serialize: None,
                deserialize: Some(deserialize_test_external),
            })
            .unwrap();
        Arc::new(registry)
    }

    #[test]
    fn externally_restored_document_can_be_marked_dirty_without_fake_command() {
        let mut history = CommandHistory::new();
        history.mark_dirty();
        assert!(history.is_dirty());
        assert!(!history.can_undo());
    }

    #[test]
    fn undoing_the_first_push_returns_to_the_initial_clean_state() {
        let mut scene = hierarchy_scene();
        let original = scene.clone();
        let mut history = CommandHistory::new();

        history
            .push(
                Box::new(SetEntityName::new(
                    "root".into(),
                    Some("renamed root".into()),
                )),
                &mut scene,
            )
            .unwrap();
        assert!(history.is_dirty());

        history.undo(&mut scene).unwrap();

        assert_eq!(scene, original);
        assert!(!history.is_dirty());
        assert!(!history.can_undo());
        assert!(history.can_redo());
    }

    #[test]
    fn save_checkpoint_tracks_exact_state_across_undo_and_redo() {
        let mut scene = hierarchy_scene();
        let mut history = CommandHistory::new();

        history
            .push(
                Box::new(SetEntityName::new("root".into(), Some("saved name".into()))),
                &mut scene,
            )
            .unwrap();
        history.mark_clean();
        let saved_scene = scene.clone();
        assert!(!history.is_dirty());

        history
            .push(
                Box::new(SetEntityName::new(
                    "root".into(),
                    Some("unsaved name".into()),
                )),
                &mut scene,
            )
            .unwrap();
        assert!(history.is_dirty());

        history.undo(&mut scene).unwrap();
        assert_eq!(scene, saved_scene);
        assert!(!history.is_dirty());

        history.redo(&mut scene).unwrap();
        assert!(history.is_dirty());

        history.undo(&mut scene).unwrap();
        assert_eq!(scene, saved_scene);
        assert!(!history.is_dirty());

        history.undo(&mut scene).unwrap();
        assert!(history.is_dirty());

        history.redo(&mut scene).unwrap();
        assert_eq!(scene, saved_scene);
        assert!(!history.is_dirty());
    }

    #[test]
    fn branching_after_undo_cannot_reuse_a_discarded_clean_state() {
        let mut scene = hierarchy_scene();
        let mut history = CommandHistory::new();

        history
            .push(
                Box::new(SetEntityName::new("root".into(), Some("first".into()))),
                &mut scene,
            )
            .unwrap();
        history
            .push(
                Box::new(SetEntityName::new(
                    "root".into(),
                    Some("discarded clean branch".into()),
                )),
                &mut scene,
            )
            .unwrap();
        history.mark_clean();
        assert!(!history.is_dirty());

        history.undo(&mut scene).unwrap();
        assert!(history.is_dirty());
        assert!(history.can_redo());

        history
            .push(
                Box::new(SetEntityName::new(
                    "root".into(),
                    Some("replacement branch".into()),
                )),
                &mut scene,
            )
            .unwrap();

        assert!(history.is_dirty());
        assert!(!history.can_redo());
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
    fn command_history_restores_scene_after_partial_forward_and_undo_failures() {
        let mut scene = hierarchy_scene();
        let original = scene.clone();
        let mut history = CommandHistory::new();
        assert!(history
            .push(Box::new(MutatesThenFails), &mut scene)
            .is_err());
        assert_eq!(scene, original);
        assert!(!history.can_undo());

        history
            .push(Box::new(UndoMutatesThenFails), &mut scene)
            .unwrap();
        let before_undo = scene.clone();
        assert!(history.undo(&mut scene).is_err());
        assert_eq!(scene, before_undo);
        assert!(history.can_undo());
    }

    #[test]
    fn set_component_field_cannot_blind_insert_a_new_field() {
        let mut scene = engine_scene::sample_scene();
        let original = scene.clone();
        let mut history = CommandHistory::new();
        let result = history.push(
            Box::new(SetComponentField::new(
                "cube-01".into(),
                "engine.renderable".into(),
                "invented_field".into(),
                Value::Bool(true),
            )),
            &mut scene,
        );

        assert!(matches!(
            result,
            Err(EditorError::ComponentFieldNotFound { .. })
        ));
        assert_eq!(scene, original);
        assert!(!history.can_undo());
    }

    #[test]
    fn invalid_core_field_type_is_rejected_after_execute_without_history() {
        let mut scene = engine_scene::sample_scene();
        let original = scene.clone();
        let mut history = CommandHistory::new();
        let result = history.push(
            Box::new(SetComponentField::new(
                "cube-01".into(),
                "engine.renderable".into(),
                "visible".into(),
                Value::Str("not-a-bool".into()),
            )),
            &mut scene,
        );

        assert!(matches!(
            result,
            Err(EditorError::SceneCommandRejected { .. })
        ));
        assert_eq!(scene, original);
        assert!(!history.can_undo());
        assert!(!history.is_dirty());
    }

    #[test]
    fn extension_deserialize_failure_is_rejected_by_runtime_registry_preflight() {
        let mut scene = engine_scene::sample_scene();
        scene.entities[1].components.insert(
            TestExternal::TYPE_ID.into(),
            ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([("value".into(), Value::UInt(7))]),
            },
        );
        // Script fields remain arbitrary Scene-only metadata and must not be
        // handed to the ECS extension registry.
        scene.entities[1].components.insert(
            "engine.script".into(),
            ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("assembly_id".into(), Value::Str("game".into())),
                    ("class_name".into(), Value::Str("Player".into())),
                    ("user_field".into(), Value::Map(BTreeMap::new())),
                ]),
            },
        );
        let registry = strict_test_registry();
        let mut editor = crate::EditorScene::new_with_component_registry(scene, registry).unwrap();
        let original = editor.scene.clone();

        let result = editor.execute(Box::new(SetComponentField::new(
            "cube-01".into(),
            TestExternal::TYPE_ID.into(),
            "value".into(),
            Value::Str("invalid".into()),
        )));

        assert!(matches!(
            result,
            Err(EditorError::SceneCommandRejected { .. })
        ));
        assert_eq!(editor.scene, original);
        assert!(!editor.history.can_undo());
    }

    #[test]
    fn validation_failure_during_undo_and_redo_is_atomic_and_keeps_stack_side() {
        let mut scene = engine_scene::sample_scene();
        let mut history = CommandHistory::new();
        history
            .push(Box::new(UndoProducesInvalidScene), &mut scene)
            .unwrap();
        history.mark_clean();
        let before_undo = scene.clone();

        assert!(matches!(
            history.undo(&mut scene),
            Err(EditorError::SceneCommandRejected { .. })
        ));
        assert_eq!(scene, before_undo);
        assert!(history.can_undo());
        assert!(!history.can_redo());
        assert!(!history.is_dirty());

        let mut scene = engine_scene::sample_scene();
        let mut history = CommandHistory::new();
        history
            .push(
                Box::new(RedoProducesInvalidScene { executions: 0 }),
                &mut scene,
            )
            .unwrap();
        history.undo(&mut scene).unwrap();
        history.mark_clean();
        let before_redo = scene.clone();

        assert!(matches!(
            history.redo(&mut scene),
            Err(EditorError::SceneCommandRejected { .. })
        ));
        assert_eq!(scene, before_redo);
        assert!(!history.can_undo());
        assert!(history.can_redo());
        assert!(!history.is_dirty());
    }

    #[test]
    fn command_batch_failure_is_atomic_even_when_a_child_partially_mutates() {
        let mut scene = hierarchy_scene();
        let original = scene.clone();
        let mut batch = CommandBatch::new(
            "Atomic batch",
            vec![
                Box::new(SetEntityName::new("root".into(), Some("changed".into()))),
                Box::new(MutatesThenFails),
            ],
        );
        assert!(batch.execute(&mut scene).is_err());
        assert_eq!(scene, original);
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

    #[test]
    fn parent_command_is_undoable_and_rejects_cycles() {
        let mut scene = engine_scene::sample_scene();
        let mut history = CommandHistory::new();
        history
            .push(
                Box::new(SetEntityParent::new(
                    "cube-01".to_string(),
                    Some("camera-main".to_string()),
                )),
                &mut scene,
            )
            .unwrap();
        assert_eq!(
            scene
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "cube-01")
                .unwrap()
                .parent
                .as_deref(),
            Some("camera-main")
        );
        assert!(history
            .push(
                Box::new(SetEntityParent::new(
                    "camera-main".to_string(),
                    Some("cube-01".to_string()),
                )),
                &mut scene,
            )
            .is_err());
        history.undo(&mut scene).unwrap();
        assert!(scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .parent
            .is_none());
    }

    fn component_record(schema: (u16, u16, u16), enabled: bool, label: &str) -> ComponentRecord {
        ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(schema.0, schema.1, schema.2),
            enabled,
            fields: BTreeMap::from([
                ("label".into(), Value::Str(label.into())),
                (
                    "nested".into(),
                    Value::Map(BTreeMap::from([(
                        "items".into(),
                        Value::List(vec![Value::Bool(enabled), Value::Float32(3.5)]),
                    )])),
                ),
            ]),
        }
    }

    fn component_clipboard_scene() -> Scene {
        let mut source = entity("source", None);
        source.components.insert(
            "test.clipboard_values".into(),
            component_record((2, 3, 4), false, "copied"),
        );
        let mut target = entity("target", None);
        target.components.insert(
            "test.clipboard_values".into(),
            component_record((0, 1, 0), true, "original"),
        );
        target.components.insert(
            "test.other_component".into(),
            component_record((0, 2, 0), true, "camera"),
        );
        let mut scene = engine_scene::sample_scene();
        scene.scene_settings.active_camera = None;
        scene.entities = vec![source, target];
        scene
    }

    fn component<'a>(
        scene: &'a Scene,
        entity_id: &str,
        component_type: &str,
    ) -> &'a ComponentRecord {
        &scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == entity_id)
            .unwrap()
            .components[component_type]
    }

    fn component_at_mut<'a>(
        scene: &'a mut Scene,
        entity_id: &str,
        component_type: &str,
    ) -> &'a mut ComponentRecord {
        scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == entity_id)
            .unwrap()
            .components
            .get_mut(component_type)
            .unwrap()
    }

    #[test]
    fn component_clipboard_ron_round_trip_preserves_the_complete_record() {
        let scene = component_clipboard_scene();
        let clipboard =
            ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
                .unwrap();
        assert_eq!(clipboard.type_id(), "test.clipboard_values");
        assert_eq!(
            clipboard.component(),
            component(&scene, "source", "test.clipboard_values")
        );

        let serialized = clipboard.to_ron().unwrap();
        let decoded = ComponentClipboard::from_ron(&serialized).unwrap();
        assert_eq!(decoded, clipboard);
        assert_eq!(
            decoded.component().schema_version,
            engine_serialize::SchemaVersion::new(2, 3, 4)
        );
        assert!(!decoded.component().enabled);
        assert_eq!(
            decoded.component().fields,
            component(&scene, "source", "test.clipboard_values").fields
        );
    }

    #[test]
    fn malformed_component_clipboard_ron_is_rejected() {
        let scene = component_clipboard_scene();
        let clipboard =
            ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
                .unwrap();
        let serialized = clipboard.to_ron().unwrap();

        let unsupported = serialized.replacen("format_version: 1", "format_version: 99", 1);
        assert!(matches!(
            ComponentClipboard::from_ron(&unsupported),
            Err(EditorError::InvalidComponentClipboard(_))
        ));
        let empty_type = serialized.replacen("\"test.clipboard_values\"", "\"\"", 1);
        assert!(matches!(
            ComponentClipboard::from_ron(&empty_type),
            Err(EditorError::InvalidComponentClipboard(_))
        ));
        let unknown_field = serialized.replacen('(', "(unknown: true,", 1);
        assert!(matches!(
            ComponentClipboard::from_ron(&unknown_field),
            Err(EditorError::ComponentClipboardSerialization(_))
        ));
        assert!(matches!(
            ComponentClipboard::from_ron("this is not RON"),
            Err(EditorError::ComponentClipboardSerialization(_))
        ));
    }

    #[test]
    fn component_values_paste_across_entities_has_exact_undo_and_redo() {
        let mut scene = component_clipboard_scene();
        let original = component(&scene, "target", "test.clipboard_values").clone();
        let copied = component(&scene, "source", "test.clipboard_values").clone();
        let original_component_count = scene.entities[1].components.len();
        let clipboard =
            ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
                .unwrap();
        let command = ReplaceComponent::prepare(
            &scene,
            "target".into(),
            "test.clipboard_values".into(),
            &clipboard,
        )
        .unwrap();
        assert_eq!(command.replacement(), &copied);

        let mut history = CommandHistory::new();
        history.push(Box::new(command), &mut scene).unwrap();
        assert_eq!(
            component(&scene, "target", "test.clipboard_values"),
            &copied
        );
        assert_eq!(scene.entities[1].components.len(), original_component_count);
        assert!(scene.entities[1]
            .components
            .contains_key("test.clipboard_values"));
        assert_eq!(
            component(&scene, "source", "test.clipboard_values"),
            &copied
        );

        history.undo(&mut scene).unwrap();
        assert_eq!(
            component(&scene, "target", "test.clipboard_values"),
            &original
        );
        history.redo(&mut scene).unwrap();
        assert_eq!(
            component(&scene, "target", "test.clipboard_values"),
            &copied
        );
    }

    #[test]
    fn component_clipboard_cannot_replace_a_different_type() {
        let scene = component_clipboard_scene();
        let clipboard =
            ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
                .unwrap();
        assert!(matches!(
            ReplaceComponent::prepare(
                &scene,
                "target".into(),
                "test.other_component".into(),
                &clipboard,
            ),
            Err(EditorError::InvalidComponentClipboard(_))
        ));
    }

    #[test]
    fn deferred_component_reset_is_an_undoable_same_key_replacement() {
        let mut scene = component_clipboard_scene();
        let original = component(&scene, "target", "test.clipboard_values").clone();
        let reset = component_record((5, 0, 1), true, "reset");
        let mut history = CommandHistory::new();
        history
            .push(
                Box::new(ReplaceComponent::new(
                    "target".into(),
                    "test.clipboard_values".into(),
                    reset.clone(),
                )),
                &mut scene,
            )
            .unwrap();
        assert_eq!(component(&scene, "target", "test.clipboard_values"), &reset);
        history.undo(&mut scene).unwrap();
        assert_eq!(
            component(&scene, "target", "test.clipboard_values"),
            &original
        );
        history.redo(&mut scene).unwrap();
        assert_eq!(component(&scene, "target", "test.clipboard_values"), &reset);
    }

    #[test]
    fn stale_component_replace_and_undo_fail_without_partial_mutation() {
        let mut scene = component_clipboard_scene();
        let clipboard =
            ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
                .unwrap();
        let command = ReplaceComponent::prepare(
            &scene,
            "target".into(),
            "test.clipboard_values".into(),
            &clipboard,
        )
        .unwrap();
        component_at_mut(&mut scene, "target", "test.clipboard_values")
            .fields
            .insert("external-edit".into(), Value::Bool(true));
        let before_failed_execute = scene.clone();
        let mut history = CommandHistory::new();
        assert!(history.push(Box::new(command), &mut scene).is_err());
        assert_eq!(scene, before_failed_execute);
        assert!(!history.can_undo());

        let command = ReplaceComponent::prepare(
            &scene,
            "target".into(),
            "test.clipboard_values".into(),
            &clipboard,
        )
        .unwrap();
        history.push(Box::new(command), &mut scene).unwrap();
        component_at_mut(&mut scene, "target", "test.clipboard_values")
            .fields
            .insert("post-paste-edit".into(), Value::Bool(true));
        let before_failed_undo = scene.clone();
        assert!(history.undo(&mut scene).is_err());
        assert_eq!(scene, before_failed_undo);
        assert!(history.can_undo());
    }

    #[test]
    fn clipboard_round_trip_captures_each_selected_subtree_once() {
        let scene = hierarchy_scene();
        let clipboard = EntityClipboard::capture(
            &scene,
            &["grandchild".into(), "root".into(), "child".into()],
        )
        .unwrap();
        assert_eq!(clipboard.root_ids(), &["root"]);
        assert_eq!(
            clipboard
                .entities()
                .iter()
                .map(|entity| entity.persistent_id.as_str())
                .collect::<Vec<_>>(),
            ["root", "child", "grandchild"]
        );

        let serialized = clipboard.to_ron().unwrap();
        assert_eq!(EntityClipboard::from_ron(&serialized).unwrap(), clipboard);
        assert!(
            EntityClipboard::from_ron("(format_version: 99, root_ids: [], entities: [])").is_err()
        );
    }

    #[test]
    fn duplicate_subtree_remaps_hierarchy_and_nested_entity_references() {
        let mut scene = hierarchy_scene();
        let original = scene.clone();
        let command = DuplicateEntitySubtree::prepare(&scene, &"root".into()).unwrap();
        assert_eq!(command.duplicated_root_id(), "root-copy");
        assert_eq!(
            command
                .duplicated_records()
                .iter()
                .map(|entity| entity.persistent_id.as_str())
                .collect::<Vec<_>>(),
            ["root-copy", "child-copy-2", "grandchild-copy"]
        );
        let mut history = CommandHistory::new();
        history.push(Box::new(command), &mut scene).unwrap();
        let duplicated = scene.clone();

        let root = scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "root-copy")
            .unwrap();
        assert_eq!(root.parent.as_deref(), Some("external"));
        assert_eq!(
            scene
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "child-copy-2")
                .unwrap()
                .parent
                .as_deref(),
            Some("root-copy")
        );
        assert_eq!(
            scene
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "grandchild-copy")
                .unwrap()
                .parent
                .as_deref(),
            Some("child-copy-2")
        );
        let fields = &root.components["test.references"].fields;
        assert_eq!(fields["internal"], Value::Entity("child-copy-2".into()));
        assert_eq!(
            fields["nested"],
            Value::Map(BTreeMap::from([(
                "target".into(),
                Value::List(vec![Value::Entity("grandchild-copy".into())])
            )]))
        );
        assert_eq!(fields["external"], Value::Entity("external".into()));

        history.undo(&mut scene).unwrap();
        assert_eq!(scene, original);
        history.redo(&mut scene).unwrap();
        assert_eq!(scene, duplicated);
    }

    #[test]
    fn paste_supports_explicit_parent_and_is_undoable() {
        let mut scene = hierarchy_scene();
        let clipboard = EntityClipboard::capture(&scene, &["root".into()]).unwrap();
        let command =
            PasteEntityRecords::prepare(&scene, &clipboard, EntityPasteParent::SceneRoot).unwrap();
        let pasted_root = command.pasted_root_ids()[0].clone();
        let original = scene.clone();
        let mut history = CommandHistory::new();
        history.push(Box::new(command), &mut scene).unwrap();
        let pasted = scene.clone();
        assert!(scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == pasted_root)
            .unwrap()
            .parent
            .is_none());

        history.undo(&mut scene).unwrap();
        assert_eq!(scene, original);
        history.redo(&mut scene).unwrap();
        assert_eq!(scene, pasted);
    }

    #[test]
    fn stale_paste_plan_fails_without_scene_or_history_mutation() {
        let mut scene = hierarchy_scene();
        let clipboard = EntityClipboard::capture(&scene, &["root".into()]).unwrap();
        let command =
            PasteEntityRecords::prepare(&scene, &clipboard, EntityPasteParent::SceneRoot).unwrap();
        let conflict = command.pasted_records()[0].clone();
        scene.entities.push(conflict);
        let before = scene.clone();
        let mut history = CommandHistory::new();
        assert!(history.push(Box::new(command), &mut scene).is_err());
        assert_eq!(scene, before);
        assert!(!history.can_undo());
    }

    fn sibling_scene() -> Scene {
        let mut scene = engine_scene::sample_scene();
        scene.scene_settings.active_camera = None;
        scene.entities = vec![
            entity("a", None),
            entity("a-child", Some("a")),
            entity("b", None),
            entity("b-child", Some("b")),
            entity("c", None),
        ];
        scene
    }

    #[test]
    fn every_sibling_move_has_exact_undo_and_redo() {
        for (entity_id, movement, expected) in [
            ("b", SiblingMove::Up, vec!["b", "a", "c"]),
            ("b", SiblingMove::Down, vec!["a", "c", "b"]),
            ("c", SiblingMove::First, vec!["c", "a", "b"]),
            ("a", SiblingMove::Last, vec!["b", "c", "a"]),
        ] {
            let mut scene = sibling_scene();
            let original = scene.clone();
            let untouched_children = [scene.entities[1].clone(), scene.entities[3].clone()];
            let mut history = CommandHistory::new();
            history
                .push(
                    Box::new(MoveEntitySibling::new(entity_id.into(), movement)),
                    &mut scene,
                )
                .unwrap();
            assert_eq!(
                sibling_ids(&scene, None),
                expected.into_iter().map(str::to_owned).collect::<Vec<_>>()
            );
            assert_eq!(scene.entities[1], untouched_children[0]);
            assert_eq!(scene.entities[3], untouched_children[1]);
            let moved = scene.clone();

            history.undo(&mut scene).unwrap();
            assert_eq!(scene, original);
            history.redo(&mut scene).unwrap();
            assert_eq!(scene, moved);
        }
    }

    #[test]
    fn sibling_boundary_and_stale_undo_fail_atomically() {
        let mut scene = sibling_scene();
        let original = scene.clone();
        let mut history = CommandHistory::new();
        assert!(history
            .push(
                Box::new(MoveEntitySibling::new("a".into(), SiblingMove::Up)),
                &mut scene,
            )
            .is_err());
        assert_eq!(scene, original);
        assert!(!history.can_undo());

        history
            .push(
                Box::new(MoveEntitySibling::new("b".into(), SiblingMove::Up)),
                &mut scene,
            )
            .unwrap();
        scene.entities.swap(0, 2);
        let stale = scene.clone();
        assert!(history.undo(&mut scene).is_err());
        assert_eq!(scene, stale);
        assert!(history.can_undo());
    }

    #[test]
    fn add_and_recursive_remove_preserve_order_and_camera_through_history() {
        let mut scene = hierarchy_scene();
        scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "child")
            .unwrap()
            .components
            .insert(
                "engine.camera".into(),
                ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::new(),
                },
            );
        scene.scene_settings.active_camera = Some("child".into());
        let original = scene.clone();
        let mut history = CommandHistory::new();
        history
            .push(Box::new(RemoveEntity::new("root".into())), &mut scene)
            .unwrap();
        assert!(scene.scene_settings.active_camera.is_none());
        assert!(!scene.entities.iter().any(|entity| matches!(
            entity.persistent_id.as_str(),
            "root" | "child" | "grandchild"
        )));
        history.undo(&mut scene).unwrap();
        assert_eq!(scene, original);
        history.redo(&mut scene).unwrap();
        history.undo(&mut scene).unwrap();
        assert_eq!(scene, original);

        let before_duplicate = scene.clone();
        assert!(history
            .push(
                Box::new(AddEntity::new(entity("external", None))),
                &mut scene
            )
            .is_err());
        assert_eq!(scene, before_duplicate);
    }

    #[test]
    fn scene_settings_are_undoable_and_reject_stale_undo() {
        let mut scene = engine_scene::sample_scene();
        let original = scene.scene_settings.clone();
        let mut replacement = original.clone();
        replacement.fixed_timestep_seconds = 1.0 / 120.0;
        replacement.default_render_layer = "Gameplay".into();
        let mut history = CommandHistory::new();
        history
            .push(
                Box::new(SetSceneSettings::prepare(&scene, replacement.clone())),
                &mut scene,
            )
            .unwrap();
        assert_eq!(scene.scene_settings, replacement);
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.scene_settings, original);
        history.redo(&mut scene).unwrap();
        assert_eq!(scene.scene_settings, replacement);

        scene.scene_settings.ambient[0] = 0.5;
        let stale = scene.clone();
        assert!(history.undo(&mut scene).is_err());
        assert_eq!(scene, stale);
        assert!(history.can_undo());
    }

    #[test]
    fn invalid_scene_settings_never_enter_history() {
        let mut scene = engine_scene::sample_scene();
        let original = scene.clone();
        let mut invalid = scene.scene_settings.clone();
        invalid.fixed_timestep_seconds = f32::NAN;
        let mut history = CommandHistory::new();
        assert!(history
            .push(
                Box::new(SetSceneSettings::prepare(&scene, invalid)),
                &mut scene,
            )
            .is_err());
        assert_eq!(scene, original);
        assert!(!history.can_undo());
    }
}
