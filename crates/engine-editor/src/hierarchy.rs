use std::collections::BTreeMap;

use engine_scene::{ComponentRecord, EntityRecord, Scene};
use engine_serialize::{AssetId, PersistentId, SchemaVersion, Value};

use crate::commands::{AddEntity, Command, RemoveEntity, SequencedCommand, SetEntityName};
use crate::editor_ui::{EditorUi, UiInteractionPhase, UiInteractionStamp};

pub(crate) const TRANSFORM_COMPONENT_TYPE: &str = "engine.transform";
pub(crate) const RENDERABLE_COMPONENT_TYPE: &str = "engine.renderable";

/// A hierarchy selection change tagged with its pointer-release order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequencedSelection {
    pub stamp: UiInteractionStamp,
    pub selection: Option<PersistentId>,
}

/// Ordered semantic output of one hierarchy redraw.
pub struct HierarchyActions {
    pub commands: Vec<SequencedCommand>,
    pub selections: Vec<SequencedSelection>,
}

fn fallback_stamp(ui: &mut EditorUi) -> UiInteractionStamp {
    ui.take_last_interaction_stamp()
        .unwrap_or(UiInteractionStamp {
            sequence: u64::MAX,
            phase: UiInteractionPhase::AfterRawPointer,
        })
}

/// Canonical transform component used by entity creation and the inspector.
pub(crate) fn transform_component() -> ComponentRecord {
    let mut fields = BTreeMap::new();
    fields.insert("translation".to_string(), Value::Vec3([0.0, 0.0, 0.0]));
    fields.insert("rotation".to_string(), Value::Quat([0.0, 0.0, 0.0, 1.0]));
    fields.insert("scale".to_string(), Value::Vec3([1.0, 1.0, 1.0]));

    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

/// Canonical renderable component for the built-in cube and material.
pub(crate) fn renderable_component() -> ComponentRecord {
    let mut fields = BTreeMap::new();
    fields.insert("mesh".to_string(), Value::Asset(AssetId::new("mesh-cube")));
    fields.insert(
        "material".to_string(),
        Value::Asset(AssetId::new("mat-default")),
    );
    fields.insert("visible".to_string(), Value::Bool(true));
    fields.insert("cast_shadows".to_string(), Value::Bool(true));
    fields.insert(
        "render_layer".to_string(),
        Value::Str("Default".to_string()),
    );

    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

fn next_available_entity_id(scene: &Scene) -> PersistentId {
    let mut sequence = 1_u64;
    loop {
        let candidate = format!("entity-{sequence:04}");
        if scene
            .entities
            .iter()
            .all(|entity| entity.persistent_id != candidate)
        {
            return candidate;
        }
        sequence += 1;
    }
}

fn new_entity(scene: &Scene, parent: Option<PersistentId>, renderable: bool) -> EntityRecord {
    let mut components = BTreeMap::new();
    components.insert(TRANSFORM_COMPONENT_TYPE.to_string(), transform_component());
    if renderable {
        components.insert(
            RENDERABLE_COMPONENT_TYPE.to_string(),
            renderable_component(),
        );
    }

    EntityRecord {
        persistent_id: next_available_entity_id(scene),
        parent,
        name: Some(if renderable {
            "New Renderable".to_string()
        } else {
            "New Entity".to_string()
        }),
        enabled: true,
        components,
    }
}

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
}

impl HierarchyPanel {
    /// Create a new hierarchy panel.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            visible: true,
            name: name.into(),
            selected: None,
            rename_target: None,
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
        self.ui_with_authoring(ui, scene, true)
    }

    /// Render the hierarchy while optionally disabling authoring actions.
    /// Selection remains available in Play/Pause, but create, delete and
    /// rename controls are not rendered as misleading clickable buttons.
    pub fn ui_with_authoring(
        &mut self,
        ui: &mut EditorUi,
        scene: &Scene,
        authoring_enabled: bool,
    ) -> Vec<Box<dyn Command>> {
        self.ui_with_authoring_ordered(ui, scene, authoring_enabled)
            .commands
            .into_iter()
            .map(SequencedCommand::into_command)
            .collect()
    }

    /// Render hierarchy controls while retaining command and selection order.
    pub fn ui_with_authoring_ordered(
        &mut self,
        ui: &mut EditorUi,
        scene: &Scene,
        authoring_enabled: bool,
    ) -> HierarchyActions {
        let mut actions = HierarchyActions {
            commands: Vec::new(),
            selections: Vec::new(),
        };

        // ── Header ────────────────────────────────────────────────
        ui.collapsing_header("Hierarchy", true);

        // ── Create / Delete buttons ──────────────────────────────
        if authoring_enabled {
            if ui.button("Create Empty") {
                let entity = new_entity(scene, self.selected.clone(), false);
                let id = entity.persistent_id.clone();
                let stamp = fallback_stamp(ui);
                actions.commands.push(SequencedCommand::new(
                    stamp,
                    Box::new(AddEntity::new(entity)),
                ));
                actions.selections.push(SequencedSelection {
                    stamp,
                    selection: Some(id.clone()),
                });
            } else if ui.button("Create Renderable") {
                let entity = new_entity(scene, self.selected.clone(), true);
                let id = entity.persistent_id.clone();
                let stamp = fallback_stamp(ui);
                actions.commands.push(SequencedCommand::new(
                    stamp,
                    Box::new(AddEntity::new(entity)),
                ));
                actions.selections.push(SequencedSelection {
                    stamp,
                    selection: Some(id.clone()),
                });
            }

            if self.selected.is_some() && ui.button("Delete Selected") {
                if let Some(ref sel) = self.selected.clone() {
                    // Capture subtree BEFORE removing.
                    let remove = RemoveEntity::new(sel, scene);
                    let stamp = fallback_stamp(ui);
                    actions
                        .commands
                        .push(SequencedCommand::new(stamp, Box::new(remove)));
                    actions.selections.push(SequencedSelection {
                        stamp,
                        selection: None,
                    });
                    self.rename_target = None;
                }
            }
        } else {
            self.rename_target = None;
            ui.label_value("Authoring", "Stop Play to create or delete entities.");
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
                self.render_entity(ui, entity, &children, 0, authoring_enabled, &mut actions);
            }
        }

        if let Some(final_selection) = actions
            .selections
            .iter()
            .max_by_key(|event| event.stamp.sequence)
        {
            self.selected = final_selection.selection.clone();
        }
        actions
    }

    // ── Internal helpers ─────────────────────────────────────────

    /// Recursively render a single entity and its children.
    fn render_entity(
        &mut self,
        ui: &mut EditorUi,
        entity: &EntityRecord,
        children: &BTreeMap<Option<PersistentId>, Vec<&EntityRecord>>,
        indent: usize,
        authoring_enabled: bool,
        actions: &mut HierarchyActions,
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
            .is_some_and(|s| *s == entity.persistent_id);

        // ── Select button ────────────────────────────────────────
        // Highlight the selected entity with a visual marker.
        let display = if is_selected {
            format!("▶ {label}")
        } else {
            label.clone()
        };

        for sequence in ui.ordered_button_clicks(&display, true) {
            actions.selections.push(SequencedSelection {
                stamp: UiInteractionStamp {
                    sequence,
                    phase: UiInteractionPhase::AfterRawPointer,
                },
                selection: Some(entity.persistent_id.clone()),
            });
            self.rename_target = None;
        }

        // ── Inline rename (if this entity is the rename target) ──
        if authoring_enabled && self.rename_target.as_deref() == Some(&entity.persistent_id) {
            let current_name = entity.name.clone().unwrap_or_default();
            if let Some(edited) = ui.text_field("##rename", &current_name) {
                let new_name = if edited.is_empty() {
                    None
                } else {
                    Some(edited)
                };
                actions.commands.push(SequencedCommand::new(
                    fallback_stamp(ui),
                    Box::new(SetEntityName::new(entity.persistent_id.clone(), new_name)),
                ));
                self.rename_target = None;
            }
        }

        // ── Double-click to rename (detected via button toggle) ──
        // For now we support rename via an explicit context action:
        // If selected and user presses 'R' (simulated via button),
        // we activate rename. In a real UI this would be double-click.
        if authoring_enabled && is_selected && ui.button("Rename") {
            self.rename_target = Some(entity.persistent_id.clone());
        }

        // ── Recursive children ────────────────────────────────
        if let Some(kids) = children.get(&Some(entity.persistent_id.clone())) {
            for child in kids {
                self.render_entity(ui, child, children, indent + 1, authoring_enabled, actions);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_ui::UiEvent;
    use crate::{EditorScene, EditorUi};

    fn scene_with_numbered_entity() -> Scene {
        let mut scene = engine_scene::sample_scene();
        scene.entities.push(EntityRecord {
            persistent_id: "entity-0001".to_string(),
            parent: None,
            name: Some("Existing".to_string()),
            enabled: true,
            components: BTreeMap::new(),
        });
        scene
    }

    #[test]
    fn create_empty_uses_scene_id_and_has_transform() {
        let mut editor_scene = EditorScene::new(scene_with_numbered_entity());
        let mut panel = HierarchyPanel::new("Hierarchy");
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick("Create Empty".to_string()));

        let commands = panel.ui(&mut ui, &editor_scene.scene);
        assert_eq!(commands.len(), 1);
        editor_scene
            .execute(commands.into_iter().next().unwrap())
            .unwrap();

        let created = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "entity-0002")
            .expect("entity-0002 should be created");
        assert_eq!(panel.selected().map(String::as_str), Some("entity-0002"));
        assert_eq!(created.components.len(), 1);
        let transform = &created.components[TRANSFORM_COMPONENT_TYPE];
        assert_eq!(transform.schema_version, SchemaVersion::new(0, 1, 0));
        assert_eq!(
            transform.fields.get("translation"),
            Some(&Value::Vec3([0.0, 0.0, 0.0]))
        );
        assert_eq!(
            transform.fields.get("rotation"),
            Some(&Value::Quat([0.0, 0.0, 0.0, 1.0]))
        );
        assert_eq!(
            transform.fields.get("scale"),
            Some(&Value::Vec3([1.0, 1.0, 1.0]))
        );
    }

    #[test]
    fn create_renderable_has_runtime_compatible_component_fields() {
        let mut editor_scene = EditorScene::new(scene_with_numbered_entity());
        let mut panel = HierarchyPanel::new("Hierarchy");
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick("Create Renderable".to_string()));

        let commands = panel.ui(&mut ui, &editor_scene.scene);
        editor_scene
            .execute(commands.into_iter().next().unwrap())
            .unwrap();

        let created = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "entity-0002")
            .expect("entity-0002 should be created");
        let renderable = &created.components[RENDERABLE_COMPONENT_TYPE];
        assert_eq!(renderable.schema_version, SchemaVersion::new(0, 1, 0));
        assert_eq!(
            renderable.fields.get("mesh"),
            Some(&Value::Asset(AssetId::new("mesh-cube")))
        );
        assert_eq!(
            renderable.fields.get("material"),
            Some(&Value::Asset(AssetId::new("mat-default")))
        );
        assert_eq!(renderable.fields.get("visible"), Some(&Value::Bool(true)));
        assert_eq!(
            renderable.fields.get("cast_shadows"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            renderable.fields.get("render_layer"),
            Some(&Value::Str("Default".to_string()))
        );

        // Loading into the runtime world and serialising it back proves that
        // both records use field names and value variants understood by ECS.
        let runtime_roundtrip = engine_scene::World::from_scene(&editor_scene.scene).to_scene();
        let runtime_created = runtime_roundtrip
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "entity-0002")
            .expect("runtime should retain the created entity");
        assert!(runtime_created
            .components
            .contains_key(TRANSFORM_COMPONENT_TYPE));
        assert!(runtime_created
            .components
            .contains_key(RENDERABLE_COMPONENT_TYPE));

        editor_scene.undo().unwrap();
        assert!(editor_scene
            .scene
            .entities
            .iter()
            .all(|entity| entity.persistent_id != "entity-0002"));
        editor_scene.redo().unwrap();
        assert!(editor_scene
            .scene
            .entities
            .iter()
            .any(|entity| entity.persistent_id == "entity-0002"));
    }

    #[test]
    fn play_mode_hierarchy_keeps_selection_but_hides_authoring_actions() {
        let scene = scene_with_numbered_entity();
        let mut panel = HierarchyPanel::new("Hierarchy");
        panel.set_selected(Some("entity-0001".into()));
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick("Create Empty".into()));
        ui.inject_event(UiEvent::ButtonClick("Delete Selected".into()));
        ui.inject_event(UiEvent::ButtonClick("Rename".into()));

        let commands = panel.ui_with_authoring(&mut ui, &scene, false);
        assert!(commands.is_empty());
        assert_eq!(panel.selected().map(String::as_str), Some("entity-0001"));
        assert!(panel.rename_target.is_none());
    }
}
