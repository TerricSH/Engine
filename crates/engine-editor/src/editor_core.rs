use engine_serialize::PersistentId;
use tracing;

use crate::editor_ui::EditorUi;
use crate::panels::EditorPanel;

// -------------------------------------------------------------------
// Editor – main editor orchestrator
// -------------------------------------------------------------------

/// Main editor orchestrator.
///
/// Owns a list of dockable [`EditorPanel`]s, tracks the currently
/// selected entity, and drives per-frame update and rendering.
pub struct Editor {
    panels: Vec<Box<dyn EditorPanel>>,
    selected_entity: Option<PersistentId>,
}

impl Editor {
    /// Create an empty editor with no panels registered.
    pub fn new() -> Self {
        Self {
            panels: Vec::new(),
            selected_entity: None,
        }
    }

    /// Register a panel.
    pub fn add_panel(&mut self, panel: Box<dyn EditorPanel>) {
        tracing::info!(name = %panel.name(), "Editor: panel added");
        self.panels.push(panel);
    }

    /// Look up a registered panel by name.
    pub fn panel_mut(&mut self, name: &str) -> Option<&mut dyn EditorPanel> {
        for panel in &mut self.panels {
            if panel.name() == name {
                return Some(panel.as_mut());
            }
        }
        None
    }

    /// Advance all panels by `dt` seconds.
    ///
    /// Called once per frame before [`Editor::render`].
    pub fn update(&mut self, _dt: f32) {
        // Per-frame update hook for panels that need time-driven logic.
        for panel in &mut self.panels {
            let _ = panel.name();
        }
    }

    /// Render all visible panels.
    ///
    /// Each panel receives a fresh [`EditorUi`] context.
    pub fn render(&mut self, ui: &mut EditorUi) {
        for panel in &mut self.panels {
            if panel.visible() {
                ui.reset();
                panel.ui(ui);
            }
        }
    }

    /// The currently selected entity ID, if any.
    pub fn selected_entity(&self) -> Option<PersistentId> {
        self.selected_entity.clone()
    }

    /// Set or clear the selected entity.
    pub fn set_selected_entity(&mut self, entity: Option<PersistentId>) {
        tracing::debug!(
            old = ?self.selected_entity,
            new = ?entity,
            "Editor: selection changed"
        );
        self.selected_entity = entity;
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
