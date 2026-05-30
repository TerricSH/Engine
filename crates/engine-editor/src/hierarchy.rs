use std::collections::BTreeMap;

use engine_scene::{EntityRecord, Scene};
use engine_serialize::PersistentId;

use crate::commands::{AddEntity, Command, RemoveEntity, SetEntityName};
use crate::editor_ui::EditorUi;

// -------------------------------------------------------------------
// HierarchyPanel
// -------------------------------------------------------------------

/// Entity hierarchy panel that lists all entities in a tree grouped by
/// parent-child relationships.
///
/// The panel's [`ui`] method renders the tree and returns a list of
/// [`Command`]s that the caller should execute on the scene.
pub struct HierarchyPanel {
    visible: bool,
    name: String,
    /// Currently selected entity (managed internally; read by the editor).
    pub(crate) selected: Option<PersistentId>,
    /// Entity whose name is being renamed in-place, if any.
    rename_target: Option<PersistentId>,
    /// Incrementing counter used to generate unique IDs for new entities.
    next_entity_id: u64,
}

impl HierarchyPanel {
    /// Create a new hierarchy panel.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            visible: true,
            name: name.into(),
            selected: None,
            rename_target: None,
            next_entity_id: 1,
        }
    }

    /// The currently selected entity ID, if any.
    pub fn selected(&self) -> Option<&PersistentId> {
        self.selected.as_ref()
    }

    /// Programmatically set the selection.
    pub fn set_selected(&mut self, id: Option<PersistentId>) {
        self.selected = id;
    }

    /// Panel name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the panel is visible.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Show or hide the panel.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Render the entity hierarchy.
    ///
    /// Returns a list of commands that the caller must apply to the
    /// scene via [`EditorScene::execute`].
    pub fn ui(&mut self, ui: &mut EditorUi, scene: &Scene) -> Vec<Box<dyn Command>> {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();

        // ── Header ────────────────────────────────────────────────
        ui.collapsing_header("Hierarchy", true);

        // ── Create / Delete buttons ──────────────────────────────
        if ui.button("+ Create Entity") {
            let id = format!("entity-{:04}", self.next_entity_id);
            self.next_entity_id += 1;
            let entity = EntityRecord {
                persistent_id: id.clone(),
                parent: self.selected.clone(),
                name: Some("New Entity".to_string()),
                enabled: true,
                components: BTreeMap::new(),
            };
            commands.push(Box::new(AddEntity::new(entity)));
            self.selected = Some(id);
        }

        if self.selected.is_some() && ui.button("Delete Selected") {
            if let Some(ref sel) = self.selected.clone() {
                // Capture subtree BEFORE removing.
                let remove = RemoveEntity::new(sel, scene);
                commands.push(Box::new(remove));
                self.selected = None;
                self.rename_target = None;
            }
        }

        ui.separator();

        // ── Build parent→children adjacency ──────────────────────
        let mut children: BTreeMap<Option<PersistentId>, Vec<&EntityRecord>> = BTreeMap::new();
        for entity in &scene.entities {
            children
                .entry(entity.parent.clone())
                .or_default()
                .push(entity);
        }

        // ── Render root entities ─────────────────────────────────
        if let Some(roots) = children.get(&None) {
            for entity in roots {
                self.render_entity(ui, entity, &children, 0, &mut commands);
            }
        }

        commands
    }

    // ── Internal helpers ─────────────────────────────────────────

    /// Recursively render a single entity and its children.
    fn render_entity(
        &mut self,
        ui: &mut EditorUi,
        entity: &EntityRecord,
        children: &BTreeMap<Option<PersistentId>, Vec<&EntityRecord>>,
        indent: usize,
        commands: &mut Vec<Box<dyn Command>>,
    ) {
        // Indentation prefix
        let _ = indent;

        let label = entity
            .name
            .clone()
            .unwrap_or_else(|| entity.persistent_id.clone());

        // Determine if this entity is the currently selected one.
        let is_selected = self
            .selected
            .as_ref()
            .map_or(false, |s| *s == entity.persistent_id);

        // ── Select button ────────────────────────────────────────
        // Highlight the selected entity with a visual marker.
        let display = if is_selected {
            format!("▶ {label}")
        } else {
            label.clone()
        };

        if ui.button(&display) {
            self.selected = Some(entity.persistent_id.clone());
            self.rename_target = None;
        }

        // ── Inline rename (if this entity is the rename target) ──
        if self.rename_target.as_deref() == Some(&entity.persistent_id) {
            let current_name = entity.name.clone().unwrap_or_default();
            if let Some(edited) = ui.text_field("##rename", &current_name) {
                let new_name = if edited.is_empty() {
                    None
                } else {
                    Some(edited)
                };
                commands.push(Box::new(SetEntityName::new(
                    entity.persistent_id.clone(),
                    new_name,
                )));
                self.rename_target = None;
            }
        }

        // ── Double-click to rename (detected via button toggle) ──
        // For now we support rename via an explicit context action:
        // If selected and user presses 'R' (simulated via button),
        // we activate rename. In a real UI this would be double-click.
        if is_selected && ui.button("Rename") {
            self.rename_target = Some(entity.persistent_id.clone());
        }

        // ── Recursive children ────────────────────────────────
        if let Some(kids) = children.get(&Some(entity.persistent_id.clone())) {
            for child in kids {
                self.render_entity(ui, child, children, indent + 1, commands);
            }
        }
    }
}
