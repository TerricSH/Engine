use super::*;

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
        for index in 0..self.commands.len() {
            if let Err(error) = self.commands[index].execute(scene) {
                for rollback in (0..index).rev() {
                    let _ = self.commands[rollback].undo(scene);
                }
                *scene = before;
                return Err(error);
            }
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
